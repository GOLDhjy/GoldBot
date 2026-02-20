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

// ── Anthropic-compatible API wire types ───────────────────────────────────────

#[derive(Serialize)]
struct ApiRequest {
    model: String,
    max_tokens: u32,
    system: String,
    messages: Vec<ApiMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream: Option<bool>,
}

#[derive(Serialize)]
struct ApiMessage {
    role: &'static str,
    content: String,
}

#[derive(Deserialize)]
struct ApiResponse {
    content: Vec<ContentBlock>,
}

#[derive(Deserialize)]
struct ContentBlock {
    #[serde(rename = "type")]
    kind: String,
    text: Option<String>,
}

#[derive(Deserialize)]
struct StreamEvent {
    #[serde(rename = "type")]
    kind: String,
    delta: Option<StreamDelta>,
    content_block: Option<ContentBlock>,
}

#[derive(Deserialize)]
struct StreamDelta {
    text: Option<String>,
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

// ── LLM call ─────────────────────────────────────────────────────────────────

/// Send the full conversation to the LLM and return its raw text response.
#[allow(dead_code)]
pub async fn chat(client: &reqwest::Client, messages: &[Message]) -> Result<String> {
    let (base_url, api_key, body) = build_request(messages, false)?;

    let resp = client
        .post(format!("{base_url}/v1/messages"))
        .header("x-api-key", &api_key)
        .header("anthropic-version", "2023-06-01")
        .json(&body)
        .send()
        .await
        .context("HTTP request failed")?;

    parse_non_stream_response(resp).await
}

/// Stream text deltas while returning the final merged response.
/// Falls back to non-stream JSON parsing if SSE is not available.
pub async fn chat_stream_with<F>(
    client: &reqwest::Client,
    messages: &[Message],
    mut on_delta: F,
) -> Result<String>
where
    F: FnMut(&str),
{
    let (base_url, api_key, body) = build_request(messages, true)?;

    let mut resp = client
        .post(format!("{base_url}/v1/messages"))
        .header("x-api-key", &api_key)
        .header("anthropic-version", "2023-06-01")
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
        drain_sse_frames(&mut pending, &mut merged, &mut on_delta);
    }
    drain_sse_frames(&mut pending, &mut merged, &mut on_delta);

    if merged.is_empty() {
        return Err(anyhow!("API returned empty content"));
    }
    Ok(merged)
}

fn build_request(messages: &[Message], stream: bool) -> Result<(String, String, ApiRequest)> {
    let base_url = std::env::var("ANTHROPIC_BASE_URL")
        .unwrap_or_else(|_| "https://open.bigmodel.cn/api/anthropic".to_string());
    let api_key =
        std::env::var("ANTHROPIC_AUTH_TOKEN").context("ANTHROPIC_AUTH_TOKEN env var not set")?;
    let model =
        std::env::var("ANTHROPIC_DEFAULT_SONNET_MODEL").unwrap_or_else(|_| "GLM-4.7".to_string());

    let system = messages
        .iter()
        .find(|m| m.role == Role::System)
        .map(|m| m.content.clone())
        .unwrap_or_default();

    let api_messages: Vec<ApiMessage> = messages
        .iter()
        .filter(|m| m.role != Role::System)
        .map(|m| ApiMessage {
            role: if m.role == Role::User {
                "user"
            } else {
                "assistant"
            },
            content: m.content.clone(),
        })
        .collect();

    let body = ApiRequest {
        model,
        max_tokens: 2048,
        system,
        messages: api_messages,
        stream: if stream { Some(true) } else { None },
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
    let text: String = parsed
        .content
        .iter()
        .filter(|b| b.kind == "text")
        .filter_map(|b| b.text.as_deref())
        .collect::<Vec<_>>()
        .join("");

    if text.is_empty() {
        return Err(anyhow!("API returned empty content"));
    }
    Ok(text)
}

fn drain_sse_frames<F>(pending: &mut String, merged: &mut String, on_delta: &mut F)
where
    F: FnMut(&str),
{
    loop {
        if let Some(pos) = pending.find("\n\n") {
            let frame = pending[..pos].to_string();
            pending.drain(..pos + 2);
            handle_sse_frame(&frame, merged, on_delta);
            continue;
        }
        if let Some(pos) = pending.find("\r\n\r\n") {
            let frame = pending[..pos].to_string();
            pending.drain(..pos + 4);
            handle_sse_frame(&frame, merged, on_delta);
            continue;
        }
        break;
    }
}

fn handle_sse_frame<F>(frame: &str, merged: &mut String, on_delta: &mut F)
where
    F: FnMut(&str),
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
        if let Some(text) = extract_stream_text(&event) {
            merged.push_str(&text);
            on_delta(&text);
        }
    }
}

fn extract_stream_text(event: &StreamEvent) -> Option<String> {
    match event.kind.as_str() {
        "content_block_delta" => event.delta.as_ref()?.text.clone().filter(|t| !t.is_empty()),
        "content_block_start" => event
            .content_block
            .as_ref()?
            .text
            .clone()
            .filter(|t| !t.is_empty()),
        _ => None,
    }
}
