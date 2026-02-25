use std::time::Duration;

use anyhow::{Context, Result};
use serde_json::{Value, json};

use super::{
    MAX_OUTPUT_CHARS,
    protocol::{RemoteMcpSession, StdioMcpSession, extract_jsonrpc_error},
    types::{LocalServerSpec, McpCallResult, RemoteServerSpec},
    util::truncate_chars,
};

pub(super) fn parse_tool_call_response(response: &Value) -> Result<McpCallResult> {
    if let Some(msg) = extract_jsonrpc_error(response) {
        return Ok(McpCallResult {
            exit_code: 1,
            output: truncate_chars(&format!("MCP tools/call error: {msg}"), MAX_OUTPUT_CHARS),
        });
    }

    let result = response
        .get("result")
        .context("tools/call missing `result` field")?;
    let is_error = result
        .get("isError")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    let mut sections = Vec::new();
    if let Some(content) = result.get("content").and_then(Value::as_array) {
        for chunk in content {
            if let Some(text) = chunk.get("text").and_then(Value::as_str)
                && !text.trim().is_empty()
            {
                sections.push(text.to_string());
                continue;
            }
            let rendered = serde_json::to_string_pretty(chunk)
                .unwrap_or_else(|_| chunk.to_string())
                .trim()
                .to_string();
            if !rendered.is_empty() {
                sections.push(rendered);
            }
        }
    }

    if let Some(structured) = result.get("structuredContent") {
        sections.push(format!(
            "structuredContent:\n{}",
            serde_json::to_string_pretty(structured).unwrap_or_else(|_| structured.to_string())
        ));
    }

    if sections.is_empty() {
        sections.push(
            serde_json::to_string_pretty(result)
                .unwrap_or_else(|_| result.to_string())
                .trim()
                .to_string(),
        );
    }

    let mut output = sections.join("\n");
    if output.trim().is_empty() {
        output = "(no output)".to_string();
    }
    output = truncate_chars(&output, MAX_OUTPUT_CHARS);

    Ok(McpCallResult {
        exit_code: if is_error { 1 } else { 0 },
        output,
    })
}

pub(super) fn call_tool_once(
    spec: &LocalServerSpec,
    tool_name: &str,
    arguments: &Value,
) -> Result<McpCallResult> {
    let mut session = StdioMcpSession::spawn(spec)?;
    session.initialize()?;

    let response = session.request(
        "tools/call",
        json!({
            "name": tool_name,
            "arguments": arguments
        }),
    )?;
    parse_tool_call_response(&response)
}

pub(super) fn call_tool_remote(
    spec: &RemoteServerSpec,
    tool_name: &str,
    arguments: &Value,
) -> Result<McpCallResult> {
    // Tool calls may take longer than discovery; use a generous timeout.
    let timeout = std::env::var("API_TIMEOUT_MS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .map(Duration::from_millis)
        .unwrap_or_else(|| Duration::from_secs(30));

    let mut session = RemoteMcpSession::new(spec, timeout)?;
    let _ = session.initialize();

    let response = session.request(
        "tools/call",
        json!({
            "name": tool_name,
            "arguments": arguments
        }),
    )?;

    parse_tool_call_response(&response)
}
