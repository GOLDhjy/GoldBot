use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io::{BufRead, BufReader, Read, Write},
    path::PathBuf,
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
};

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use serde_json::{Value, json};

const ENV_MCP_SERVERS: &str = "GOLDBOT_MCP_SERVERS";
const MCP_PROTOCOL_VERSION: &str = "2024-11-05";
const MAX_OUTPUT_CHARS: usize = 12_000;
const MAX_PROMPT_TOOLS: usize = 64;
const MAX_SCHEMA_FIELDS: usize = 8;
const MAX_FIELD_CHARS: usize = 32;
const MAX_DESC_CHARS: usize = 140;

#[derive(Debug, Clone, Default)]
pub struct McpRegistry {
    servers: BTreeMap<String, LocalServerSpec>,
    tools: BTreeMap<String, McpToolSpec>,
}

#[derive(Debug, Clone)]
pub struct McpToolSpec {
    pub action_name: String,
    pub server_name: String,
    pub tool_name: String,
    pub description: String,
    pub read_only_hint: bool,
    pub input_schema: Value,
}

#[derive(Debug, Clone)]
pub struct McpCallResult {
    pub exit_code: i32,
    pub output: String,
}

#[derive(Debug, Clone)]
struct LocalServerSpec {
    command: String,
    args: Vec<String>,
    env: HashMap<String, String>,
    cwd: Option<PathBuf>,
}

#[derive(Debug, Clone)]
struct DiscoveredTool {
    tool_name: String,
    description: String,
    input_schema: Value,
    read_only_hint: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum RawServerEntry {
    Disabled(bool),
    Config(RawServerConfig),
}

#[derive(Debug, Clone, Deserialize)]
struct RawServerConfig {
    #[serde(default = "default_server_type")]
    r#type: String,
    command: Option<String>,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    env: HashMap<String, String>,
    cwd: Option<String>,
    #[serde(default = "default_enabled")]
    enabled: bool,
    #[allow(dead_code)]
    url: Option<String>,
}

fn default_server_type() -> String {
    "local".to_string()
}

fn default_enabled() -> bool {
    true
}

impl McpRegistry {
    pub fn from_env() -> (Self, Vec<String>) {
        let Some(raw) = std::env::var(ENV_MCP_SERVERS).ok() else {
            return (Self::default(), Vec::new());
        };

        if raw.trim().is_empty() {
            return (Self::default(), Vec::new());
        }

        let parsed: BTreeMap<String, RawServerEntry> = match serde_json::from_str(&raw) {
            Ok(v) => v,
            Err(e) => {
                return (
                    Self::default(),
                    vec![format!(
                        "{ENV_MCP_SERVERS} is not valid JSON: {e}. MCP tools are disabled."
                    )],
                );
            }
        };

        let mut warnings = Vec::new();
        let mut registry = Self::default();

        for (server_name, entry) in parsed {
            match entry {
                RawServerEntry::Disabled(false) => {}
                RawServerEntry::Disabled(true) => warnings.push(format!(
                    "MCP server `{server_name}` config is boolean `true`; expected an object. Skipped."
                )),
                RawServerEntry::Config(cfg) => {
                    if !cfg.enabled {
                        continue;
                    }
                    let ty = cfg.r#type.to_lowercase();
                    if ty != "local" {
                        warnings.push(format!(
                            "MCP server `{server_name}` type `{ty}` is not supported yet (local stdio only). Skipped."
                        ));
                        continue;
                    }
                    let Some(command) = cfg.command.filter(|c| !c.trim().is_empty()) else {
                        warnings.push(format!(
                            "MCP server `{server_name}` is missing a non-empty `command`. Skipped."
                        ));
                        continue;
                    };

                    registry.servers.insert(
                        server_name,
                        LocalServerSpec {
                            command,
                            args: cfg.args,
                            env: cfg.env,
                            cwd: cfg.cwd.map(PathBuf::from),
                        },
                    );
                }
            }
        }

        if registry.servers.is_empty() {
            return (registry, warnings);
        }

        warnings.extend(registry.discover_tools());
        (registry, warnings)
    }

    pub fn augment_system_prompt(&self, base_prompt: &str) -> String {
        if self.tools.is_empty() {
            return base_prompt.to_string();
        }

        let mut out = String::new();
        out.push_str(base_prompt);
        out.push_str(
            "\n\nYou also have MCP tools discovered from local MCP servers.\n\n\
             To call an MCP tool, respond with EXACTLY this structure (nothing else):\n\
             <thought>your reasoning about what to do next</thought>\n\
             <tool>mcp_server_tool</tool>\n\
             <arguments>{\"key\":\"value\"}</arguments>\n\n\
             Rules for MCP calls:\n\
             - `<tool>` must be exactly one of the listed MCP tool names below.\n\
             - `<arguments>` must be valid JSON object.\n\
             - Output one tool call per response, then wait for the tool result.\n\
             - Use shell when filesystem/terminal work is needed; use MCP when external context/API is needed.\n\n\
             Available MCP tools:\n",
        );

        for tool in self.tools.values().take(MAX_PROMPT_TOOLS) {
            let desc = truncate_chars(&tool.description, MAX_DESC_CHARS);
            let args = summarize_input_schema(&tool.input_schema);
            let ro = if tool.read_only_hint {
                "read-only"
            } else {
                "read/write"
            };
            out.push_str(&format!(
                "- {} => server=`{}` tool=`{}` ({ro}), args: {}, desc: {}\n",
                tool.action_name,
                tool.server_name,
                tool.tool_name,
                args,
                fallback_if_empty(&desc, "(no description)")
            ));
        }

        if self.tools.len() > MAX_PROMPT_TOOLS {
            out.push_str(&format!(
                "- ... {} more MCP tools omitted for brevity.\n",
                self.tools.len() - MAX_PROMPT_TOOLS
            ));
        }

        out
    }

    pub fn execute_tool(&self, action_name: &str, arguments: &Value) -> Result<McpCallResult> {
        let Some(tool) = self.tools.get(action_name) else {
            bail!("unknown MCP tool `{action_name}`");
        };

        if !arguments.is_object() {
            bail!("MCP <arguments> must be a JSON object");
        }

        let Some(server) = self.servers.get(&tool.server_name) else {
            bail!(
                "MCP server `{}` is not available for tool `{}`",
                tool.server_name,
                action_name
            );
        };

        call_tool_once(server, &tool.tool_name, arguments)
    }

    fn discover_tools(&mut self) -> Vec<String> {
        let mut warnings = Vec::new();
        let mut used_names = BTreeSet::new();

        for (server_name, server) in &self.servers {
            match list_tools_once(server) {
                Ok(tools) => {
                    for tool in tools {
                        let action_name =
                            unique_action_name(server_name, &tool.tool_name, &mut used_names);
                        self.tools.insert(
                            action_name.clone(),
                            McpToolSpec {
                                action_name,
                                server_name: server_name.clone(),
                                tool_name: tool.tool_name,
                                description: tool.description,
                                read_only_hint: tool.read_only_hint,
                                input_schema: tool.input_schema,
                            },
                        );
                    }
                }
                Err(e) => warnings.push(format!(
                    "Failed to load MCP tools from `{server_name}`: {e}. Server skipped."
                )),
            }
        }

        warnings
    }
}

fn list_tools_once(spec: &LocalServerSpec) -> Result<Vec<DiscoveredTool>> {
    let mut session = StdioMcpSession::spawn(spec)?;
    session.initialize()?;

    let response = session.request("tools/list", json!({}))?;
    if let Some(msg) = extract_jsonrpc_error(&response) {
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

fn call_tool_once(
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

    if let Some(msg) = extract_jsonrpc_error(&response) {
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

struct StdioMcpSession {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: u64,
}

impl StdioMcpSession {
    fn spawn(spec: &LocalServerSpec) -> Result<Self> {
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
        })
    }

    fn initialize(&mut self) -> Result<()> {
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

    fn request(&mut self, method: &str, params: Value) -> Result<Value> {
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
        let payload = serde_json::to_vec(message)?;
        write!(self.stdin, "Content-Length: {}\r\n\r\n", payload.len())?;
        self.stdin.write_all(&payload)?;
        self.stdin.flush()?;
        Ok(())
    }

    fn read(&mut self) -> Result<Value> {
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
}

impl Drop for StdioMcpSession {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn extract_jsonrpc_error(value: &Value) -> Option<String> {
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

fn unique_action_name(server_name: &str, tool_name: &str, used: &mut BTreeSet<String>) -> String {
    let server = sanitize_token(server_name);
    let tool = sanitize_token(tool_name);
    let base = format!("mcp_{server}_{tool}");

    if used.insert(base.clone()) {
        return base;
    }

    let mut index = 2usize;
    loop {
        let candidate = format!("{base}_{index}");
        if used.insert(candidate.clone()) {
            return candidate;
        }
        index += 1;
    }
}

fn sanitize_token(input: &str) -> String {
    let mut out = String::new();
    let mut prev_underscore = false;

    for ch in input.chars() {
        let c = ch.to_ascii_lowercase();
        if c.is_ascii_alphanumeric() {
            out.push(c);
            prev_underscore = false;
        } else if !prev_underscore {
            out.push('_');
            prev_underscore = true;
        }
    }

    let trimmed = out.trim_matches('_').to_string();
    if trimmed.is_empty() {
        "tool".to_string()
    } else {
        trimmed
    }
}

fn summarize_input_schema(schema: &Value) -> String {
    let required: BTreeSet<&str> = schema
        .get("required")
        .and_then(Value::as_array)
        .map(|arr| arr.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();

    let Some(properties) = schema.get("properties").and_then(Value::as_object) else {
        return "object".to_string();
    };
    if properties.is_empty() {
        return "object".to_string();
    }

    let mut keys: Vec<&str> = properties.keys().map(String::as_str).collect();
    keys.sort_unstable();

    let mut parts = Vec::new();
    for key in keys.iter().take(MAX_SCHEMA_FIELDS) {
        let ty = properties
            .get(*key)
            .and_then(|v| v.get("type"))
            .and_then(Value::as_str)
            .unwrap_or("any");
        let req = if required.contains(*key) { "*" } else { "" };
        parts.push(format!(
            "{}:{}{}",
            truncate_chars(key, MAX_FIELD_CHARS),
            truncate_chars(ty, MAX_FIELD_CHARS),
            req
        ));
    }

    if keys.len() > MAX_SCHEMA_FIELDS {
        parts.push(format!("+{}", keys.len() - MAX_SCHEMA_FIELDS));
    }

    parts.join(", ")
}

fn fallback_if_empty<'a>(value: &'a str, fallback: &'a str) -> &'a str {
    if value.trim().is_empty() {
        fallback
    } else {
        value
    }
}

fn truncate_chars(text: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }
    let count = text.chars().count();
    if count <= max_chars {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max_chars.saturating_sub(1)).collect();
    out.push('…');
    out
}

fn command_line_for_log(command: &str, args: &[String]) -> String {
    if args.is_empty() {
        return command.to_string();
    }
    format!("{command} {}", args.join(" "))
}

#[cfg(test)]
mod tests {
    use super::{sanitize_token, summarize_input_schema, unique_action_name};
    use serde_json::json;
    use std::collections::BTreeSet;

    #[test]
    fn sanitize_token_collapses_symbols() {
        assert_eq!(sanitize_token("Context7 MCP"), "context7_mcp");
        assert_eq!(sanitize_token("@@"), "tool");
        assert_eq!(sanitize_token("A__B"), "a_b");
    }

    #[test]
    fn unique_action_name_adds_suffix() {
        let mut used = BTreeSet::new();
        let first = unique_action_name("context7", "lookup", &mut used);
        let second = unique_action_name("context7", "lookup", &mut used);
        assert_eq!(first, "mcp_context7_lookup");
        assert_eq!(second, "mcp_context7_lookup_2");
    }

    #[test]
    fn summarize_schema_marks_required_fields() {
        let schema = json!({
            "type": "object",
            "properties": {
                "libraryName": { "type": "string" },
                "tokens": { "type": "integer" }
            },
            "required": ["libraryName"]
        });
        let summary = summarize_input_schema(&schema);
        assert!(summary.contains("libraryName:string*"));
        assert!(summary.contains("tokens:integer"));
    }
}
