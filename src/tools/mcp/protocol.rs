use std::{
    collections::HashMap,
    io::{BufRead, BufReader, Read, Write},
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
    time::Duration,
};

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};

use super::{
    MCP_PROTOCOL_VERSION,
    types::{LocalServerSpec, RemoteServerSpec},
};

pub(super) struct StdioMcpSession {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: u64,
    wire_format: StdioWireFormat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StdioWireFormat {
    Framed,
    LineDelimited,
}

impl StdioMcpSession {
    pub(super) fn spawn(spec: &LocalServerSpec) -> Result<Self> {
        let wire_format = detect_stdio_wire_format(spec);
        let mut command = Command::new(&spec.command);
        command
            .args(&spec.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        if let Some(cwd) = &spec.cwd {
            command.current_dir(cwd);
        }
        for (k, v) in &spec.env {
            command.env(k, v);
        }

        let mut child = command.spawn().with_context(|| {
            format!(
                "failed to spawn MCP server command `{}`",
                command_line_for_log(&spec.command, &spec.args)
            )
        })?;

        let stdin = child.stdin.take().context("MCP server has no stdin")?;
        let stdout = child.stdout.take().context("MCP server has no stdout")?;

        Ok(Self {
            child,
            stdin,
            stdout: BufReader::new(stdout),
            next_id: 1,
            wire_format,
        })
    }

    pub(super) fn initialize(&mut self) -> Result<()> {
        let response = self.request(
            "initialize",
            json!({
                "protocolVersion": MCP_PROTOCOL_VERSION,
                "capabilities": {},
                "clientInfo": {
                    "name": "goldbot",
                    "version": env!("CARGO_PKG_VERSION")
                }
            }),
        )?;

        if let Some(msg) = extract_jsonrpc_error(&response) {
            bail!("initialize error: {msg}");
        }

        self.notify("notifications/initialized", json!({}))
    }

    fn notify(&mut self, method: &str, params: Value) -> Result<()> {
        self.send(&json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params
        }))
    }

    pub(super) fn request(&mut self, method: &str, params: Value) -> Result<Value> {
        let id = self.next_id;
        self.next_id += 1;

        self.send(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params
        }))?;

        loop {
            let message = self.read()?;
            if message.get("id").and_then(Value::as_u64) == Some(id) {
                return Ok(message);
            }
        }
    }

    fn send(&mut self, message: &Value) -> Result<()> {
        match self.wire_format {
            StdioWireFormat::Framed => {
                let payload = serde_json::to_vec(message)?;
                write!(self.stdin, "Content-Length: {}\r\n\r\n", payload.len())?;
                self.stdin.write_all(&payload)?;
            }
            StdioWireFormat::LineDelimited => {
                let payload = serde_json::to_string(message)?;
                self.stdin.write_all(payload.as_bytes())?;
                self.stdin.write_all(b"\n")?;
            }
        }
        self.stdin.flush()?;
        Ok(())
    }

    fn read(&mut self) -> Result<Value> {
        match self.wire_format {
            StdioWireFormat::Framed => self.read_framed(),
            StdioWireFormat::LineDelimited => self.read_line_delimited(),
        }
    }

    fn read_framed(&mut self) -> Result<Value> {
        let mut content_length: Option<usize> = None;
        let mut line = String::new();

        loop {
            line.clear();
            let n = self.stdout.read_line(&mut line)?;
            if n == 0 {
                bail!("MCP server closed output stream");
            }
            let trimmed = line.trim_end_matches(['\r', '\n']);
            if trimmed.is_empty() {
                break;
            }
            if let Some(rest) = trimmed.strip_prefix("Content-Length:") {
                let len = rest
                    .trim()
                    .parse::<usize>()
                    .context("invalid Content-Length header")?;
                content_length = Some(len);
            }
        }

        let len = content_length.context("missing Content-Length header")?;
        let mut body = vec![0u8; len];
        self.stdout.read_exact(&mut body)?;
        serde_json::from_slice::<Value>(&body).context("invalid JSON-RPC payload")
    }

    fn read_line_delimited(&mut self) -> Result<Value> {
        let mut line = String::new();
        loop {
            line.clear();
            let n = self.stdout.read_line(&mut line)?;
            if n == 0 {
                bail!("MCP server closed output stream");
            }
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            if let Ok(v) = serde_json::from_str::<Value>(trimmed) {
                return Ok(v);
            }
            if let Ok(v) = serde_json::from_str::<Value>(&line) {
                return Ok(v);
            }
        }
    }
}

impl Drop for StdioMcpSession {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

pub(super) fn extract_jsonrpc_error(value: &Value) -> Option<String> {
    let err = value.get("error")?;
    let code = err.get("code").and_then(Value::as_i64).unwrap_or_default();
    let message = err
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("unknown JSON-RPC error");
    let data = err.get("data").and_then(|v| {
        let pretty = serde_json::to_string(v).ok()?;
        (!pretty.is_empty()).then_some(pretty)
    });

    Some(match data {
        Some(d) => format!("code={code}, message={message}, data={d}"),
        None => format!("code={code}, message={message}"),
    })
}

fn command_line_for_log(command: &str, args: &[String]) -> String {
    if args.is_empty() {
        return command.to_string();
    }
    format!("{command} {}", args.join(" "))
}

fn detect_stdio_wire_format(spec: &LocalServerSpec) -> StdioWireFormat {
    // Explicit config takes priority.
    if let Some(t) = &spec.transport {
        return match t.to_ascii_lowercase().as_str() {
            "framed" | "lsp" | "content-length" => StdioWireFormat::Framed,
            _ => StdioWireFormat::LineDelimited,
        };
    }
    // MCP stdio standard is newline-delimited JSON; default to that.
    StdioWireFormat::LineDelimited
}

pub(super) struct RemoteMcpSession {
    url: String,
    headers: HashMap<String, String>,
    client: reqwest::blocking::Client,
    next_id: u64,
    session_id: Option<String>,
}

impl RemoteMcpSession {
    pub(super) fn new(spec: &RemoteServerSpec, timeout: Duration) -> Result<Self> {
        let client = reqwest::blocking::Client::builder()
            .timeout(timeout)
            .build()
            .context("failed to build HTTP client for remote MCP")?;
        Ok(Self {
            url: spec.url.clone(),
            headers: spec.headers.clone(),
            client,
            next_id: 1,
            session_id: None,
        })
    }

    pub(super) fn initialize(&mut self) -> Result<()> {
        let id = self.next_id;
        self.next_id += 1;
        let payload = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "initialize",
            "params": {
                "protocolVersion": MCP_PROTOCOL_VERSION,
                "capabilities": {},
                "clientInfo": {
                    "name": "goldbot",
                    "version": env!("CARGO_PKG_VERSION")
                }
            }
        });
        let response = self.do_request(&payload)?;
        if let Some(msg) = extract_jsonrpc_error(&response) {
            bail!("initialize error: {msg}");
        }
        // notifications/initialized 閿?best-effort, no response expected
        let notif = json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized",
            "params": {}
        });
        let _ = self.do_request(&notif);
        Ok(())
    }

    pub(super) fn request(&mut self, method: &str, params: Value) -> Result<Value> {
        let id = self.next_id;
        self.next_id += 1;
        let payload = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params
        });
        self.do_request(&payload)
    }

    fn do_request(&mut self, payload: &Value) -> Result<Value> {
        let mut req = self.client.post(&self.url);
        for (k, v) in &self.headers {
            req = req.header(k.as_str(), v.as_str());
        }
        req = req
            .header("Content-Type", "application/json")
            .header("Accept", "application/json, text/event-stream");
        if let Some(sid) = &self.session_id {
            req = req.header("Mcp-Session-Id", sid.as_str());
        }

        let response = req
            .json(payload)
            .send()
            .context("HTTP request to remote MCP server failed")?;

        // Extract session-id before consuming response body.
        let new_sid = response
            .headers()
            .get("Mcp-Session-Id")
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);

        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_lowercase();

        let status = response.status();
        let body = response
            .text()
            .context("failed to read remote MCP response body")?;

        if let Some(sid) = new_sid {
            self.session_id = Some(sid);
        }

        // 202 Accepted or empty body 閿?notification acknowledged, nothing to parse.
        if status.as_u16() == 202 || body.trim().is_empty() {
            return Ok(json!({}));
        }

        if !status.is_success() {
            bail!(
                "HTTP {} from remote MCP server: {}",
                status.as_u16(),
                &body[..body.len().min(300)]
            );
        }

        if content_type.contains("text/event-stream") {
            parse_sse_jsonrpc(&body)
        } else {
            serde_json::from_str::<Value>(&body).with_context(|| {
                format!(
                    "invalid JSON from remote MCP server: {}",
                    &body[..body.len().min(200)]
                )
            })
        }
    }
}

/// Parse the first JSON-RPC response message from an SSE stream body.
fn parse_sse_jsonrpc(body: &str) -> Result<Value> {
    for line in body.lines() {
        let line = line.trim();
        if let Some(data) = line.strip_prefix("data:") {
            let data = data.trim();
            if data.is_empty() || data == "[DONE]" {
                continue;
            }
            if let Ok(v) = serde_json::from_str::<Value>(data) {
                // We want a JSON-RPC response (has id + result/error) or a batch.
                if v.get("id").is_some() && (v.get("result").is_some() || v.get("error").is_some())
                {
                    return Ok(v);
                }
            }
        }
    }
    bail!("no valid JSON-RPC response found in SSE stream")
}
