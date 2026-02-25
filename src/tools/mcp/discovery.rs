use std::{sync::mpsc, thread, time::Duration};

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};

use super::{
    ENV_MCP_DISCOVERY_TIMEOUT_MS,
    protocol::{RemoteMcpSession, StdioMcpSession, extract_jsonrpc_error},
    types::{DiscoveredTool, LocalServerSpec, RemoteServerSpec, ServerSpec},
};

pub(super) fn list_tools_for_server(
    spec: &ServerSpec,
    timeout: Duration,
) -> Result<Vec<DiscoveredTool>> {
    match spec {
        ServerSpec::Local(spec) => list_tools_with_timeout(spec, timeout),
        ServerSpec::Remote(spec) => list_tools_remote(spec, timeout),
    }
}

pub(super) fn parse_discovered_tools_response(response: &Value) -> Result<Vec<DiscoveredTool>> {
    if let Some(msg) = extract_jsonrpc_error(response) {
        bail!("tools/list error: {msg}");
    }

    let tools = response
        .pointer("/result/tools")
        .and_then(Value::as_array)
        .context("tools/list missing `result.tools` array")?;

    let mut discovered = Vec::new();
    for item in tools {
        let Some(name) = item.get("name").and_then(Value::as_str) else {
            continue;
        };
        if name.trim().is_empty() {
            continue;
        }
        let description = item
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let input_schema = item
            .get("inputSchema")
            .cloned()
            .unwrap_or_else(|| json!({"type":"object"}));
        let read_only_hint = item
            .pointer("/annotations/readOnlyHint")
            .and_then(Value::as_bool)
            .or_else(|| {
                item.pointer("/annotations/read_only_hint")
                    .and_then(Value::as_bool)
            })
            .unwrap_or(false);

        discovered.push(DiscoveredTool {
            tool_name: name.to_string(),
            description,
            input_schema,
            read_only_hint,
        });
    }

    Ok(discovered)
}

pub(super) fn list_tools_once(spec: &LocalServerSpec) -> Result<Vec<DiscoveredTool>> {
    let mut session = StdioMcpSession::spawn(spec)?;
    session.initialize()?;

    let response = session.request("tools/list", json!({}))?;
    parse_discovered_tools_response(&response)
}

pub(super) fn list_tools_with_timeout(
    spec: &LocalServerSpec,
    timeout: Duration,
) -> Result<Vec<DiscoveredTool>> {
    let spec = spec.clone();
    let (tx, rx) = mpsc::channel();

    thread::spawn(move || {
        let _ = tx.send(list_tools_once(&spec));
    });

    match rx.recv_timeout(timeout) {
        Ok(result) => result,
        Err(mpsc::RecvTimeoutError::Timeout) => bail!(
            "discovery timed out after {}ms (set {} to increase)",
            timeout.as_millis(),
            ENV_MCP_DISCOVERY_TIMEOUT_MS
        ),
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            bail!("discovery worker terminated unexpectedly")
        }
    }
}

pub(super) fn list_tools_remote(
    spec: &RemoteServerSpec,
    timeout: Duration,
) -> Result<Vec<DiscoveredTool>> {
    let mut session = RemoteMcpSession::new(spec, timeout)?;
    // Initialize: ignore errors - many remote servers are stateless and skip this step.
    let _ = session.initialize();

    let response = session.request("tools/list", json!({}))?;
    parse_discovered_tools_response(&response)
}
