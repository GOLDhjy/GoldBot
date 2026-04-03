use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};

use crate::agent::provider::{LlmProvider, Message, Role, Usage};

#[derive(Clone, Copy)]
pub(crate) struct MimoProvider;

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

#[derive(Serialize)]
struct ApiRequest {
    model: String,
    messages: Vec<ApiMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_completion_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    top_p: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    frequency_penalty: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    presence_penalty: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    thinking: Option<ThinkingParam>,
}

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
    #[allow(dead_code)]
    reasoning_content: Option<String>,
}

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
    /// 小米普通 chat 兼容接口的思考增量字段。
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

impl LlmProvider for MimoProvider {
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
    const BASE_URL: &str = "https://api.xiaomimimo.com/v1";

    let base_url = std::env::var("MIMO_BASE_URL").unwrap_or_else(|_| BASE_URL.to_string());
    let api_key = std::env::var("MIMO_API_KEY").context("MIMO_API_KEY env var not set")?;
    let model = model.to_string();

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
        // 先不强行限制输出长度，避免和不同模型的上限策略冲突。
        max_completion_tokens: None,
        stream: if stream { Some(true) } else { None },
        temperature: Some(1.0),
        top_p: Some(0.95),
        frequency_penalty: Some(0.0),
        presence_penalty: Some(0.0),
        thinking: Some(ThinkingParam {
            kind: if show_thinking { "enabled" } else { "disabled" },
        }),
    };

    Ok((base_url, api_key, body))
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

    #[test]
    fn thinking_param_enabled_serializes_correctly() {
        let json = serde_json::to_string(&ThinkingParam { kind: "enabled" }).unwrap();
        assert_eq!(json, r#"{"type":"enabled"}"#);
    }

    #[test]
    fn stream_delta_parses_reasoning_content() {
        let json = r#"{"content":null,"reasoning_content":"Hmm"}"#;
        let delta: StreamDelta = serde_json::from_str(json).unwrap();
        assert_eq!(delta.content, None);
        assert_eq!(delta.reasoning_content.as_deref(), Some("Hmm"));
    }

    #[test]
    fn handle_sse_frame_routes_content_and_reasoning() {
        let mut merged = String::new();
        let mut usage = Usage::default();
        let mut content_parts = Vec::new();
        let mut thinking_parts = Vec::new();

        handle_sse_frame(
            r#"data: {"choices":[{"delta":{"content":null,"reasoning_content":"先想一下"}}]}"#,
            &mut merged,
            &mut usage,
            &mut |s: &str| content_parts.push(s.to_string()),
            &mut |s: &str| thinking_parts.push(s.to_string()),
        );
        handle_sse_frame(
            r#"data: {"choices":[{"delta":{"content":"答案","reasoning_content":null}}]}"#,
            &mut merged,
            &mut usage,
            &mut |s: &str| content_parts.push(s.to_string()),
            &mut |s: &str| thinking_parts.push(s.to_string()),
        );

        assert_eq!(merged, "答案");
        assert_eq!(content_parts, vec!["答案"]);
        assert_eq!(thinking_parts, vec!["先想一下"]);
    }
}
