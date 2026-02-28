use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};

use crate::agent::provider::{Message, Role, Usage};

#[derive(Clone, Copy)]
pub(crate) struct MiniMaxProvider;

// ── Wire types ────────────────────────────────────────────────────────────────

#[derive(Serialize)]
struct ApiMessage {
    role: &'static str,
    content: String,
}

#[derive(Serialize)]
struct ApiRequest {
    model: String,
    messages: Vec<ApiMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream: Option<bool>,
    /// 设为 true 时，思考内容从 reasoning_details 字段单独输出
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning_split: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
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
    #[serde(default)]
    reasoning_details: Vec<ReasoningDetail>,
}

#[derive(Deserialize)]
struct ReasoningDetail {
    text: String,
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
    /// MiniMax 思考内容（累积字符串，非增量）
    #[serde(default)]
    reasoning_details: Vec<ReasoningDetail>,
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

impl MiniMaxProvider {
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
        // reasoning_details 是累积字符串，记录已向上层发送的字节数，避免重复推送
        let mut reasoning_seen = 0usize;
        let mut final_usage = Usage::default();

        while let Some(chunk) = resp.chunk().await.context("failed reading stream chunk")? {
            pending.push_str(&String::from_utf8_lossy(&chunk));
            drain_sse_frames(
                &mut pending,
                &mut merged,
                &mut reasoning_seen,
                &mut final_usage,
                &mut on_delta,
                &mut on_thinking_delta,
            );
        }
        drain_sse_frames(
            &mut pending,
            &mut merged,
            &mut reasoning_seen,
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
    const BASE_URL: &str = "https://api.minimaxi.com/v1";

    let base_url = std::env::var("MINIMAX_BASE_URL").unwrap_or_else(|_| BASE_URL.to_string());
    let api_key = std::env::var("MINIMAX_API_KEY").context("MINIMAX_API_KEY env var not set")?;
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
        max_tokens: None,
        stream: if stream { Some(true) } else { None },
        reasoning_split: if show_thinking { Some(true) } else { None },
        temperature: Some(1.0),
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
    let usage = parsed.usage.map(|u| u.to_usage()).unwrap_or_default();
    let choice = parsed
        .choices
        .into_iter()
        .next()
        .ok_or_else(|| anyhow!("API returned no choices"))?;
    let text = choice
        .message
        .content
        .filter(|t| !t.is_empty())
        .or_else(|| {
            choice
                .message
                .reasoning_details
                .into_iter()
                .next()
                .map(|d| d.text)
        })
        .unwrap_or_default();
    if text.is_empty() {
        return Err(anyhow!("API returned empty content"));
    }
    Ok((text, usage))
}

fn drain_sse_frames<F, G>(
    pending: &mut String,
    merged: &mut String,
    reasoning_seen: &mut usize,
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
            handle_sse_frame(
                &frame,
                merged,
                reasoning_seen,
                final_usage,
                on_delta,
                on_thinking_delta,
            );
            continue;
        }
        if let Some(pos) = pending.find("\r\n\r\n") {
            let frame = pending[..pos].to_string();
            pending.drain(..pos + 4);
            handle_sse_frame(
                &frame,
                merged,
                reasoning_seen,
                final_usage,
                on_delta,
                on_thinking_delta,
            );
            continue;
        }
        break;
    }
}

fn handle_sse_frame<F, G>(
    frame: &str,
    merged: &mut String,
    reasoning_seen: &mut usize,
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
        // reasoning_details 是累积字符串，只向上层推送新增部分
        if let Some(detail) = choice.delta.reasoning_details.into_iter().next() {
            let full = &detail.text;
            if let Some(new_part) = full.get(*reasoning_seen..) {
                if !new_part.is_empty() {
                    on_thinking_delta(new_part);
                }
                *reasoning_seen = full.len();
            }
        }
    }
}
