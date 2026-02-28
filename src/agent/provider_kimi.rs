use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};

use crate::agent::provider::{Message, Role, Usage};

#[derive(Clone, Copy)]
pub(crate) struct KimiProvider;

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

/// Kimi K2.5 请求体。
///
/// 根据官方文档，kimi-k2.5 的 temperature / top_p / n / presence_penalty /
/// frequency_penalty 均为固定值且**不可修改**，因此不应发送这些字段（服务端有默认值）。
/// `max_tokens` 已废弃，改用 `max_completion_tokens`。
#[derive(Serialize)]
struct ApiRequest {
    model: String,
    messages: Vec<ApiMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_completion_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    thinking: Option<ThinkingParam>,
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
    /// K2.5 thinking 模式下的推理内容
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
    /// K2.5 流式 thinking delta（字段名 `reasoning`，区别于 GLM 的 `reasoning_content`）
    reasoning: Option<String>,
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

impl KimiProvider {
    pub(crate) async fn chat_stream_with<F, G>(
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

        let mut req = client
            .post(format!("{base_url}/chat/completions"))
            .header("Authorization", format!("Bearer {api_key}"));

        // Kimi for Coding 端点要求 Coding Agent 标识
        if base_url.contains("api.kimi.com") {
            req = req.header("X-Kimi-Coding-Agent", "GoldBot");
        }

        let mut resp = req
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
    let api_key = std::env::var("KIMI_API_KEY").context("KIMI_API_KEY env var not set")?;

    // sk-kimi- 前缀 → Kimi for Coding 端点；其他 → Moonshot 通用端点
    let default_base = if api_key.starts_with("sk-kimi-") {
        "https://api.kimi.com/coding/v1"
    } else {
        "https://api.moonshot.cn/v1"
    };
    let base_url = std::env::var("KIMI_BASE_URL").unwrap_or_else(|_| default_base.to_string());
    let coding_endpoint = is_kimi_coding_endpoint(&base_url);

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

    // kimi-k2.5：thinking 参数控制是否启用思考，默认 enabled。
    // temperature / top_p / n / presence_penalty / frequency_penalty 均不可修改，不发送。
    let normalized_model = normalize_model_for_endpoint(model, coding_endpoint);
    let body = ApiRequest {
        model: normalized_model,
        messages: api_messages,
        max_completion_tokens: Some(32768),
        stream: if stream { Some(true) } else { None },
        // Kimi for Coding endpoint follows a different compatibility profile.
        // Avoid forcing `thinking` field there to reduce invalid_request failures.
        thinking: if coding_endpoint {
            None
        } else {
            Some(ThinkingParam {
                kind: if show_thinking { "enabled" } else { "disabled" },
            })
        },
    };

    Ok((base_url, api_key, body))
}

fn is_kimi_coding_endpoint(base_url: &str) -> bool {
    base_url.contains("api.kimi.com/coding")
}

fn normalize_model_for_endpoint(model: &str, coding_endpoint: bool) -> String {
    if !coding_endpoint {
        return model.to_string();
    }
    match model {
        "kimi-k2.5" | "k2p5" => "kimi-for-coding".to_string(),
        other => other.to_string(),
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
        if let Some(thinking) = choice.delta.reasoning.filter(|t| !t.is_empty()) {
            on_thinking_delta(&thinking);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_body_only_sends_required_fields() {
        // 验证 K2.5 请求体不包含 temperature/top_p/n 等不可修改字段
        let body = ApiRequest {
            model: "kimi-k2.5".to_string(),
            messages: vec![ApiMessage {
                role: "user",
                content: "Hi".to_string(),
            }],
            max_completion_tokens: Some(32768),
            stream: Some(true),
            thinking: Some(ThinkingParam { kind: "enabled" }),
        };
        let json = serde_json::to_value(&body).unwrap();
        assert!(json.get("temperature").is_none());
        assert!(json.get("top_p").is_none());
        assert!(json.get("n").is_none());
        assert!(json.get("presence_penalty").is_none());
        assert!(json.get("frequency_penalty").is_none());
        assert!(json.get("max_tokens").is_none());
        // 应该包含的字段
        assert_eq!(json["model"], "kimi-k2.5");
        assert_eq!(json["max_completion_tokens"], 32768);
        assert_eq!(json["stream"], true);
        assert_eq!(json["thinking"]["type"], "enabled");
    }

    #[test]
    fn stream_delta_parses_reasoning() {
        let json = r#"{"content":null,"reasoning":"Let me think..."}"#;
        let delta: StreamDelta = serde_json::from_str(json).unwrap();
        assert_eq!(delta.content, None);
        assert_eq!(delta.reasoning.as_deref(), Some("Let me think..."));
    }

    #[test]
    fn stream_delta_parses_content() {
        let json = r#"{"content":"Hello world"}"#;
        let delta: StreamDelta = serde_json::from_str(json).unwrap();
        assert_eq!(delta.content.as_deref(), Some("Hello world"));
        assert_eq!(delta.reasoning, None);
    }

    #[test]
    fn handle_sse_frame_routes_reasoning_to_callback() {
        let frame = r#"data: {"choices":[{"delta":{"content":null,"reasoning":"step 1"}}]}"#;
        let mut merged = String::new();
        let mut content_parts = Vec::new();
        let mut thinking_parts = Vec::new();

        handle_sse_frame(
            frame,
            &mut merged,
            &mut |s: &str| content_parts.push(s.to_string()),
            &mut |s: &str| thinking_parts.push(s.to_string()),
        );

        assert!(merged.is_empty());
        assert!(content_parts.is_empty());
        assert_eq!(thinking_parts, vec!["step 1"]);
    }

    #[test]
    fn handle_sse_frame_routes_content_to_merged() {
        let frame = r#"data: {"choices":[{"delta":{"content":"hello","reasoning":null}}]}"#;
        let mut merged = String::new();
        let mut content_parts = Vec::new();
        let mut thinking_parts = Vec::new();

        handle_sse_frame(
            frame,
            &mut merged,
            &mut |s: &str| content_parts.push(s.to_string()),
            &mut |s: &str| thinking_parts.push(s.to_string()),
        );

        assert_eq!(merged, "hello");
        assert_eq!(content_parts, vec!["hello"]);
        assert!(thinking_parts.is_empty());
    }

    #[test]
    fn thinking_disabled_serializes_correctly() {
        let body = ApiRequest {
            model: "kimi-k2.5".to_string(),
            messages: vec![],
            max_completion_tokens: None,
            stream: None,
            thinking: Some(ThinkingParam { kind: "disabled" }),
        };
        let json = serde_json::to_value(&body).unwrap();
        assert_eq!(json["thinking"]["type"], "disabled");
    }

    #[test]
    fn non_stream_response_parses_correctly() {
        let json = r#"{"choices":[{"message":{"content":"answer","reasoning_content":"thinking"}}]}"#;
        let resp: ApiResponse = serde_json::from_str(json).unwrap();
        let choice = &resp.choices[0];
        assert_eq!(choice.message.content.as_deref(), Some("answer"));
        assert_eq!(
            choice.message.reasoning_content.as_deref(),
            Some("thinking")
        );
    }

    #[test]
    fn coding_endpoint_normalizes_model_aliases() {
        assert_eq!(
            normalize_model_for_endpoint("kimi-k2.5", true),
            "kimi-for-coding"
        );
        assert_eq!(normalize_model_for_endpoint("k2p5", true), "kimi-for-coding");
        assert_eq!(
            normalize_model_for_endpoint("kimi-k2-thinking", true),
            "kimi-k2-thinking"
        );
    }

    #[test]
    fn moonshot_endpoint_keeps_model_name() {
        assert_eq!(
            normalize_model_for_endpoint("kimi-k2.5", false),
            "kimi-k2.5"
        );
    }
}
