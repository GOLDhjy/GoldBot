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
        Self { role: Role::System, content: content.into() }
    }
    pub fn user(content: impl Into<String>) -> Self {
        Self { role: Role::User, content: content.into() }
    }
    pub fn assistant(content: impl Into<String>) -> Self {
        Self { role: Role::Assistant, content: content.into() }
    }
}

// ── Anthropic-compatible API wire types ───────────────────────────────────────

#[derive(Serialize)]
struct ApiRequest {
    model: String,
    max_tokens: u32,
    system: String,
    messages: Vec<ApiMessage>,
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
pub async fn chat(client: &reqwest::Client, messages: &[Message]) -> Result<String> {
    let base_url = std::env::var("ANTHROPIC_BASE_URL")
        .unwrap_or_else(|_| "https://open.bigmodel.cn/api/anthropic".to_string());
    let api_key = std::env::var("ANTHROPIC_AUTH_TOKEN")
        .context("ANTHROPIC_AUTH_TOKEN env var not set")?;
    let model = std::env::var("ANTHROPIC_DEFAULT_SONNET_MODEL")
        .unwrap_or_else(|_| "GLM-4.7".to_string());

    // Anthropic API separates system from the messages array.
    let system = messages
        .iter()
        .find(|m| m.role == Role::System)
        .map(|m| m.content.clone())
        .unwrap_or_default();

    let api_messages: Vec<ApiMessage> = messages
        .iter()
        .filter(|m| m.role != Role::System)
        .map(|m| ApiMessage {
            role: if m.role == Role::User { "user" } else { "assistant" },
            content: m.content.clone(),
        })
        .collect();

    let body = ApiRequest { model, max_tokens: 2048, system, messages: api_messages };

    let resp = client
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
