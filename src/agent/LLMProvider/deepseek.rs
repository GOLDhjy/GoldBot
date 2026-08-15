use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};

use crate::agent::provider::{LlmProvider, Message, Role, Usage};

#[derive(Clone, Copy)]
pub(crate) struct DeepSeekProvider;

const DEFAULT_BASE_URL: &str = "https://api.deepseek.com";

// ── Wire types ────────────────────────────────────────────────────────────────

#[derive(Serialize)]
struct ApiMessage {
    role: &'static str,
    content: String,
}

#[derive(Serialize)]
struct ThinkingParam {
    #[serde(rename = "type")]
    kind: &'static str,
}

/// DeepSeek 请求体。
///
/// 思考模式（thinking 开启）下 `temperature` / `top_p` / `presence_penalty` /
/// `frequency_penalty` 均不生效，为避免歧义一律不发送。
/// 思考强度由 `reasoning_effort`（low/high/max）控制，默认 high。
#[derive(Serialize)]
struct ApiRequest {
    model: String,
    messages: Vec<ApiMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    thinking: Option<ThinkingParam>,
    /// 思考强度（low/high/max），仅思考模式开启时生效。
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning_effort: Option<String>,
}

// ── Response types ────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct ApiResponse {
    choices: Vec<ApiChoice>,
    usage: Option<UsageParam>,
}

#[derive(Deserialize)]
struct ApiChoice {
    message: ApiChoiceMessage,
}

#[derive(Deserialize)]
struct ApiChoiceMessage {
    content: Option<String>,
    /// 思考模式下的思维链内容（非流式返回）。
    #[allow(dead_code)]
    reasoning_content: Option<String>,
}

// ── SSE streaming types ───────────────────────────────────────────────────────

#[derive(Deserialize)]
struct StreamEvent {
    #[serde(default)]
    choices: Vec<StreamChoice>,
    usage: Option<UsageParam>,
}

#[derive(Deserialize)]
struct StreamChoice {
    delta: StreamDelta,
}

#[derive(Deserialize)]
struct StreamDelta {
    content: Option<String>,
    /// DeepSeek 思考模式流式思维链增量。
    reasoning_content: Option<String>,
}

#[derive(Deserialize, Clone)]
struct UsageParam {
    pub prompt_tokens: Option<u32>,
    pub completion_tokens: Option<u32>,
    pub total_tokens: Option<u32>,
}

impl UsageParam {
    fn to_usage(&self) -> Usage {
        Usage {
            prompt_tokens: self.prompt_tokens.unwrap_or(0),
            completion_tokens: self.completion_tokens.unwrap_or(0),
            total_tokens: self.total_tokens.unwrap_or(0),
        }
    }
}

// ── Implementation ────────────────────────────────────────────────────────────

impl LlmProvider for DeepSeekProvider {
    async fn chat_stream_with<F, G>(
        &self,
        client: &reqwest::Client,
        messages: &[Message],
        model: &str,
        show_thinking: bool,
        mut on_delta: F,
        mut on_thinking_delta: G,
    ) -> Result<(String, Usage)>
    where
        F: FnMut(&str),
        G: FnMut(&str),
    {
        let (base_url, api_key, body) = build_request(messages, model, true, show_thinking)?;

        let mut resp = client
            .post(format!("{base_url}/chat/completions"))
            .header("Authorization", format!("Bearer {api_key}"))
            .json(&body)
            .send()
            .await
            .context("HTTP request failed")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(anyhow!("API error {status}: {text}"));
        }

        let content_type = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_ascii_lowercase();

        if !content_type.contains("text/event-stream") {
            return parse_non_stream_response(resp).await;
        }

        let mut merged = String::new();
        let mut pending = String::new();
        let mut final_usage = Usage::default();

        while let Some(chunk) = resp.chunk().await.context("failed reading stream chunk")? {
            pending.push_str(&String::from_utf8_lossy(&chunk));
            drain_sse_frames(
                &mut pending,
                &mut merged,
                &mut final_usage,
                &mut on_delta,
                &mut on_thinking_delta,
            );
        }
        drain_sse_frames(
            &mut pending,
            &mut merged,
            &mut final_usage,
            &mut on_delta,
            &mut on_thinking_delta,
        );

        if merged.is_empty() {
            return Err(anyhow!("API returned empty content"));
        }
        Ok((merged, final_usage))
    }
}

fn build_request(
    messages: &[Message],
    model: &str,
    stream: bool,
    show_thinking: bool,
) -> Result<(String, String, ApiRequest)> {
    let api_key = std::env::var("DEEPSEEK_API_KEY").context("DEEPSEEK_API_KEY env var not set")?;
    let base_url =
        std::env::var("DEEPSEEK_BASE_URL").unwrap_or_else(|_| DEFAULT_BASE_URL.to_string());
    let model = normalize_deepseek_model(model);

    let api_messages: Vec<ApiMessage> = messages
        .iter()
        .map(|m| ApiMessage {
            role: match m.role {
                Role::System => "system",
                Role::User => "user",
                Role::Assistant => "assistant",
            },
            content: m.content.clone(),
        })
        .collect();

    let body = ApiRequest {
        model,
        messages: api_messages,
        max_tokens: None,
        stream: if stream { Some(true) } else { None },
        // 思考模式默认开启，effort 默认 high；show_thinking 控制开关。
        thinking: Some(ThinkingParam {
            kind: if show_thinking { "enabled" } else { "disabled" },
        }),
        reasoning_effort: crate::agent::provider::deepseek_effort_from_env(),
    };

    Ok((base_url, api_key, body))
}

/// 归一化模型名：仅接受官方模型，未知值回落默认模型。
fn normalize_deepseek_model(model: &str) -> String {
    match model.trim().to_ascii_lowercase().as_str() {
        "deepseek-v4-pro" | "deepseek-v4-flash" => model.trim().to_ascii_lowercase(),
        _ => crate::agent::provider::default_deepseek_model(),
    }
}

async fn parse_non_stream_response(resp: reqwest::Response) -> Result<(String, Usage)> {
    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(anyhow!("API error {status}: {text}"));
    }
    let parsed: ApiResponse = resp.json().await.context("failed to parse API response")?;
    let text = parsed
        .choices
        .into_iter()
        .next()
        .and_then(|c| c.message.content)
        .unwrap_or_default();
    if text.is_empty() {
        return Err(anyhow!("API returned empty content"));
    }
    let usage = parsed.usage.map(|u| u.to_usage()).unwrap_or_default();
    Ok((text, usage))
}

fn drain_sse_frames<F, G>(
    pending: &mut String,
    merged: &mut String,
    final_usage: &mut Usage,
    on_delta: &mut F,
    on_thinking_delta: &mut G,
) where
    F: FnMut(&str),
    G: FnMut(&str),
{
    loop {
        if let Some(pos) = pending.find("\n\n") {
            let frame = pending[..pos].to_string();
            pending.drain(..pos + 2);
            handle_sse_frame(&frame, merged, final_usage, on_delta, on_thinking_delta);
            continue;
        }
        if let Some(pos) = pending.find("\r\n\r\n") {
            let frame = pending[..pos].to_string();
            pending.drain(..pos + 4);
            handle_sse_frame(&frame, merged, final_usage, on_delta, on_thinking_delta);
            continue;
        }
        break;
    }
}

fn handle_sse_frame<F, G>(
    frame: &str,
    merged: &mut String,
    final_usage: &mut Usage,
    on_delta: &mut F,
    on_thinking_delta: &mut G,
) where
    F: FnMut(&str),
    G: FnMut(&str),
{
    for raw_line in frame.lines() {
        let line = raw_line.trim_end_matches('\r');
        let Some(data) = line.strip_prefix("data: ") else {
            continue;
        };
        if data.is_empty() || data == "[DONE]" {
            continue;
        }
        let Ok(event) = serde_json::from_str::<StreamEvent>(data) else {
            continue;
        };
        if let Some(usage) = event.usage {
            *final_usage = usage.to_usage();
        }
        let Some(choice) = event.choices.into_iter().next() else {
            continue;
        };
        if let Some(text) = choice.delta.content.filter(|t| !t.is_empty()) {
            merged.push_str(&text);
            on_delta(&text);
        }
        if let Some(thinking) = choice.delta.reasoning_content.filter(|t| !t.is_empty()) {
            on_thinking_delta(&thinking);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::provider::ENV_LOCK;

    #[test]
    fn thinking_param_enabled_serializes_correctly() {
        let json = serde_json::to_string(&ThinkingParam { kind: "enabled" }).unwrap();
        assert_eq!(json, r#"{"type":"enabled"}"#);
    }

    #[test]
    fn thinking_param_disabled_serializes_correctly() {
        let json = serde_json::to_string(&ThinkingParam { kind: "disabled" }).unwrap();
        assert_eq!(json, r#"{"type":"disabled"}"#);
    }

    #[test]
    fn request_body_only_sends_thinking_fields() {
        // 思考模式下 temperature/top_p 等采样参数不生效，不应出现在请求体中。
        let body = ApiRequest {
            model: "deepseek-v4-pro".to_string(),
            messages: vec![ApiMessage {
                role: "user",
                content: "Hi".to_string(),
            }],
            max_tokens: None,
            stream: Some(true),
            thinking: Some(ThinkingParam { kind: "enabled" }),
            reasoning_effort: Some("high".to_string()),
        };
        let json = serde_json::to_value(&body).unwrap();
        assert!(json.get("temperature").is_none());
        assert!(json.get("top_p").is_none());
        assert!(json.get("presence_penalty").is_none());
        assert!(json.get("frequency_penalty").is_none());
        assert_eq!(json["model"], "deepseek-v4-pro");
        assert_eq!(json["stream"], true);
        assert_eq!(json["thinking"]["type"], "enabled");
        assert_eq!(json["reasoning_effort"], "high");
    }

    #[test]
    fn stream_delta_parses_reasoning_content() {
        let json = r#"{"content":null,"reasoning_content":"Let me think..."}"#;
        let delta: StreamDelta = serde_json::from_str(json).unwrap();
        assert_eq!(delta.content, None);
        assert_eq!(delta.reasoning_content.as_deref(), Some("Let me think..."));
    }

    #[test]
    fn stream_delta_parses_content() {
        let json = r#"{"content":"Hello world"}"#;
        let delta: StreamDelta = serde_json::from_str(json).unwrap();
        assert_eq!(delta.content.as_deref(), Some("Hello world"));
        assert_eq!(delta.reasoning_content, None);
    }

    #[test]
    fn handle_sse_frame_routes_reasoning_to_callback() {
        let frame =
            r#"data: {"choices":[{"delta":{"content":null,"reasoning_content":"step 1"}}]}"#;
        let mut merged = String::new();
        let mut usage = Usage::default();
        let mut content_parts = Vec::new();
        let mut thinking_parts = Vec::new();

        handle_sse_frame(
            frame,
            &mut merged,
            &mut usage,
            &mut |s: &str| content_parts.push(s.to_string()),
            &mut |s: &str| thinking_parts.push(s.to_string()),
        );

        assert!(merged.is_empty());
        assert!(content_parts.is_empty());
        assert_eq!(thinking_parts, vec!["step 1"]);
    }

    #[test]
    fn handle_sse_frame_routes_content_to_merged() {
        let frame = r#"data: {"choices":[{"delta":{"content":"hello","reasoning_content":null}}]}"#;
        let mut merged = String::new();
        let mut usage = Usage::default();
        let mut content_parts = Vec::new();
        let mut thinking_parts = Vec::new();

        handle_sse_frame(
            frame,
            &mut merged,
            &mut usage,
            &mut |s: &str| content_parts.push(s.to_string()),
            &mut |s: &str| thinking_parts.push(s.to_string()),
        );

        assert_eq!(merged, "hello");
        assert_eq!(content_parts, vec!["hello"]);
        assert!(thinking_parts.is_empty());
    }

    #[test]
    fn model_aliases_normalize_to_official_names() {
        assert_eq!(
            normalize_deepseek_model("deepseek-v4-pro"),
            "deepseek-v4-pro"
        );
        assert_eq!(
            normalize_deepseek_model("DeepSeek-V4-Pro"),
            "deepseek-v4-pro"
        );
        assert_eq!(
            normalize_deepseek_model("deepseek-v4-flash"),
            "deepseek-v4-flash"
        );
    }

    #[test]
    fn unknown_model_falls_back_to_default() {
        let _guard = ENV_LOCK.lock().unwrap();
        unsafe {
            std::env::set_var("DEEPSEEK_MODEL", "deepseek-v4-pro");
        }
        assert_eq!(normalize_deepseek_model("gpt-4o"), "deepseek-v4-pro");
        unsafe {
            std::env::remove_var("DEEPSEEK_MODEL");
        }
    }

    #[test]
    fn build_request_includes_effort_and_thinking_switch() {
        let _guard = ENV_LOCK.lock().unwrap();

        unsafe {
            std::env::set_var("DEEPSEEK_API_KEY", "test-key");
            std::env::set_var("DEEPSEEK_EFFORT", "max");
        }

        let messages = vec![Message::user("hi")];
        let (_, _, body) = build_request(&messages, "deepseek-v4-pro", true, true).unwrap();
        assert_eq!(body.reasoning_effort.as_deref(), Some("max"));
        assert_eq!(body.thinking.as_ref().unwrap().kind, "enabled");

        let (_, _, body) = build_request(&messages, "deepseek-v4-pro", true, false).unwrap();
        assert_eq!(body.thinking.as_ref().unwrap().kind, "disabled");

        unsafe {
            std::env::remove_var("DEEPSEEK_API_KEY");
            std::env::remove_var("DEEPSEEK_EFFORT");
        }
    }
}
