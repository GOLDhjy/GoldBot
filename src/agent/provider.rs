use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};

// ── Conversation message types ────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Role {
    System,
    User,
    Assistant,
}

#[derive(Debug, Clone)]
pub struct Message {
    pub role: Role,
    pub content: String,
}

impl Message {
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: Role::System,
            content: content.into(),
        }
    }
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: content.into(),
        }
    }
    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: Role::Assistant,
            content: content.into(),
        }
    }
}

// ── GLM native API wire types (OpenAI-compatible format) ──────────────────────

#[derive(Serialize)]
struct ThinkingParam {
    #[serde(rename = "type")]
    kind: &'static str,
    /// false = preserve reasoning across turns (recommended).
    /// Omitted when thinking is disabled.
    #[serde(skip_serializing_if = "Option::is_none")]
    clear_thinking: Option<bool>,
}

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
}

#[derive(Serialize)]
struct ApiMessage {
    role: &'static str,
    content: String,
}

// ── Non-streaming response ────────────────────────────────────────────────────

#[derive(Deserialize)]
struct ApiResponse {
    choices: Vec<ApiChoice>,
}

#[derive(Deserialize)]
struct ApiChoice {
    message: ApiChoiceMessage,
}

#[derive(Deserialize)]
struct ApiChoiceMessage {
    content: Option<String>,
}

// ── Streaming SSE response ────────────────────────────────────────────────────

#[derive(Deserialize)]
struct StreamEvent {
    choices: Vec<StreamChoice>,
}

#[derive(Deserialize)]
struct StreamChoice {
    delta: StreamDelta,
}

#[derive(Deserialize)]
struct StreamDelta {
    /// Text response content delta.
    content: Option<String>,
    /// Native thinking content delta.
    reasoning_content: Option<String>,
}

// ── HTTP client builder ───────────────────────────────────────────────────────

pub fn build_http_client() -> Result<reqwest::Client> {
    let mut builder = reqwest::Client::builder();

    if let Ok(proxy_url) = std::env::var("HTTP_PROXY") {
        builder = builder.proxy(reqwest::Proxy::all(&proxy_url)?);
    }

    if let Ok(ms) = std::env::var("API_TIMEOUT_MS") {
        if let Ok(ms) = ms.parse::<u64>() {
            builder = builder
                .timeout(std::time::Duration::from_millis(ms))
                .connect_timeout(std::time::Duration::from_secs(10));
        }
    }

    builder.build().map_err(Into::into)
}

// ── LLM calls ────────────────────────────────────────────────────────────────

/// Send the full conversation to the LLM and return its raw text response.
#[allow(dead_code)]
pub async fn chat(
    client: &reqwest::Client,
    messages: &[Message],
    show_thinking: bool,
) -> Result<String> {
    let (base_url, api_key, body) = build_request(messages, false, show_thinking)?;

    let resp = client
        .post(format!("{base_url}/chat/completions"))
        .header("Authorization", format!("Bearer {api_key}"))
        .json(&body)
        .send()
        .await
        .context("HTTP request failed")?;

    parse_non_stream_response(resp).await
}

/// Stream text deltas while returning the final merged response.
/// `on_delta` receives text content; `on_thinking_delta` receives native thinking content.
pub async fn chat_stream_with<F, G>(
    client: &reqwest::Client,
    messages: &[Message],
    show_thinking: bool,
    mut on_delta: F,
    mut on_thinking_delta: G,
) -> Result<String>
where
    F: FnMut(&str),
    G: FnMut(&str),
{
    let (base_url, api_key, body) = build_request(messages, true, show_thinking)?;

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

    while let Some(chunk) = resp.chunk().await.context("failed reading stream chunk")? {
        pending.push_str(&String::from_utf8_lossy(&chunk));
        drain_sse_frames(&mut pending, &mut merged, &mut on_delta, &mut on_thinking_delta);
    }
    drain_sse_frames(&mut pending, &mut merged, &mut on_delta, &mut on_thinking_delta);

    if merged.is_empty() {
        return Err(anyhow!("API returned empty content"));
    }
    Ok(merged)
}

fn build_request(
    messages: &[Message],
    stream: bool,
    show_thinking: bool,
) -> Result<(String, String, ApiRequest)> {
    const BASE_URL: &str = "https://open.bigmodel.cn/api/coding/paas/v4";
    const MODEL: &str = "GLM-4.7";

    let base_url = std::env::var("BIGMODEL_BASE_URL").unwrap_or_else(|_| BASE_URL.to_string());
    let api_key = std::env::var("BIGMODEL_API_KEY").context("BIGMODEL_API_KEY env var not set")?;
    let model = std::env::var("BIGMODEL_MODEL").unwrap_or_else(|_| MODEL.to_string());

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

    let thinking = if show_thinking {
        Some(ThinkingParam {
            kind: "enabled",
            clear_thinking: Some(false),
        })
    } else {
        Some(ThinkingParam {
            kind: "disabled",
            clear_thinking: None,
        })
    };

    let body = ApiRequest {
        model,
        messages: api_messages,
        max_tokens: Some(4096),
        stream: if stream { Some(true) } else { None },
        thinking,
    };

    Ok((base_url, api_key, body))
}

async fn parse_non_stream_response(resp: reqwest::Response) -> Result<String> {
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
    Ok(text)
}

fn drain_sse_frames<F, G>(
    pending: &mut String,
    merged: &mut String,
    on_delta: &mut F,
    on_thinking_delta: &mut G,
)
where
    F: FnMut(&str),
    G: FnMut(&str),
{
    loop {
        if let Some(pos) = pending.find("\n\n") {
            let frame = pending[..pos].to_string();
            pending.drain(..pos + 2);
            handle_sse_frame(&frame, merged, on_delta, on_thinking_delta);
            continue;
        }
        if let Some(pos) = pending.find("\r\n\r\n") {
            let frame = pending[..pos].to_string();
            pending.drain(..pos + 4);
            handle_sse_frame(&frame, merged, on_delta, on_thinking_delta);
            continue;
        }
        break;
    }
}

fn handle_sse_frame<F, G>(
    frame: &str,
    merged: &mut String,
    on_delta: &mut F,
    on_thinking_delta: &mut G,
)
where
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
    fn build_request_thinking_enabled_sets_correct_fields() {
        // Validate ThinkingParam serialization for enabled state
        let param = ThinkingParam {
            kind: "enabled",
            clear_thinking: Some(false),
        };
        let json = serde_json::to_string(&param).unwrap();
        assert!(json.contains("\"enabled\""));
        assert!(json.contains("\"clear_thinking\":false"));
    }

    #[test]
    fn build_request_thinking_disabled_omits_clear_thinking() {
        let param = ThinkingParam {
            kind: "disabled",
            clear_thinking: None,
        };
        let json = serde_json::to_string(&param).unwrap();
        assert!(json.contains("\"disabled\""));
        assert!(!json.contains("clear_thinking"));
    }
}
