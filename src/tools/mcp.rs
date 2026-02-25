use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    fs,
    io::{BufRead, BufReader, Read, Write},
    path::PathBuf,
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
    sync::mpsc,
    thread,
    time::Duration,
};

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use serde_json::{Value, json};

const ENV_MCP_SERVERS: &str = "GOLDBOT_MCP_SERVERS";
const ENV_MCP_SERVERS_FILE: &str = "GOLDBOT_MCP_SERVERS_FILE";
const ENV_MCP_DISCOVERY_TIMEOUT_MS: &str = "GOLDBOT_MCP_DISCOVERY_TIMEOUT_MS";
const ENV_MEMORY_DIR: &str = "GOLDBOT_MEMORY_DIR";
const DEFAULT_MCP_SERVERS_FILENAME: &str = "mcp_servers.json";
const DEFAULT_MCP_DISCOVERY_TIMEOUT_MS: u64 = 3000;

// Global MCP config files relative to $HOME. GoldBot's own file is checked first.
const GLOBAL_MCP_CONFIG_FILES: &[&str] = &[
    ".goldbot/mcp_servers.json",
    ".kiro/settings/mcp.json",
    ".config/opencode/opencode.json",
    ".gemini/settings.json",
    ".claude/claude_desktop_config.json",
];

// Global TOML config files relative to $HOME (each parsed separately).
const GLOBAL_TOML_CONFIG_FILES: &[&str] = &[".codex/config.toml"];

// Project-local MCP config files (searched from cwd up to git root).
const LOCAL_MCP_CONFIG_FILES: &[&str] =
    &[".kiro/settings/mcp.json", "mcp.json", "mcp_servers.json"];
// Keep this aligned with https://modelcontextprotocol.io/specification/versioning
const MCP_PROTOCOL_VERSION: &str = "2025-11-25";
const MAX_OUTPUT_CHARS: usize = 12_000;
const MAX_PROMPT_TOOLS: usize = 64;
const MAX_SCHEMA_FIELDS: usize = 8;
const MAX_FIELD_CHARS: usize = 32;
const MAX_DESC_CHARS: usize = 140;
#[allow(dead_code)]
const CREATE_MCP_ASSIST_PROMPT_APPENDIX_TEMPLATE: &str = "\
Create a new MCP server:
<thought>reasoning</thought>
<create_mcp>{\"name\":\"server-name\",\"command\":[\"npx\",\"-y\",\"@scope/pkg\"]}</create_mcp>

## create_mcp fields
- `name` (required): server identifier used as the config key
- `command` (required): **array** with executable and all arguments, e.g. `[\"npx\",\"-y\",\"@scope/pkg\",\"--flag\",\"val\"]`
- `env` (optional): env vars object, e.g. `{{\"API_KEY\":\"val\"}}` — omit if not needed
- `cwd` (optional): working directory — omit if not needed
`type` and `enabled` are added automatically. Config is written to {MCP_CONFIG_PATH}. Tell the user to restart GoldBot to activate.";

#[derive(Debug, Clone, Default)]
pub struct McpRegistry {
    servers: BTreeMap<String, LocalServerSpec>,
    tools: BTreeMap<String, McpToolSpec>,
    failed: Vec<String>,
}

pub struct McpStartupStatus {
    /// (server_name, tool_count)
    pub ok: Vec<(String, usize)>,
    pub failed: Vec<String>,
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
    /// Explicitly configured wire format; `None` means auto-detect.
    transport: Option<String>,
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
    command: Option<RawCommand>,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    env: HashMap<String, String>,
    #[serde(default)]
    headers: HashMap<String, String>,
    cwd: Option<String>,
    #[serde(default = "default_enabled")]
    enabled: bool,
    /// Wire format override: "line" (default) or "framed" (Content-Length/LSP style).
    transport: Option<String>,
    #[allow(dead_code)]
    url: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum RawCommand {
    String(String),
    Array(Vec<String>),
}

fn default_server_type() -> String {
    "local".to_string()
}

fn default_enabled() -> bool {
    true
}

impl McpRegistry {
    pub fn from_env() -> (Self, Vec<String>) {
        let mut warnings = Vec::new();

        // Explicit env override: inline JSON string.
        if let Ok(v) = std::env::var(ENV_MCP_SERVERS) {
            if !v.trim().is_empty() {
                let mut registry = Self::default();
                match parse_server_entries(&v) {
                    Ok(entries) => {
                        Self::populate_from_entries(&mut registry, entries, &mut warnings)
                    }
                    Err(e) => warnings.push(format!(
                        "MCP config is invalid: {e}. MCP tools are disabled."
                    )),
                }
                return (registry, warnings);
            }
        }

        // Explicit env override: single file path.
        if let Some(path) = std::env::var_os(ENV_MCP_SERVERS_FILE) {
            let p = PathBuf::from(path);
            if !p.as_os_str().is_empty() {
                match fs::read_to_string(&p) {
                    Ok(text) if !text.trim().is_empty() => {
                        let mut registry = Self::default();
                        match parse_server_entries(&text) {
                            Ok(entries) => {
                                Self::populate_from_entries(&mut registry, entries, &mut warnings)
                            }
                            Err(e) => warnings.push(format!(
                                "MCP config `{}` is invalid: {e}. MCP tools are disabled.",
                                p.display()
                            )),
                        }
                        return (registry, warnings);
                    }
                    Ok(_) => return (Self::default(), warnings),
                    Err(e) => {
                        warnings.push(format!("failed to read MCP config `{}`: {e}", p.display()));
                        return (Self::default(), warnings);
                    }
                }
            }
        }

        // Multi-file discovery: merge all found entries, first-server-name-wins.
        let mut merged: BTreeMap<String, RawServerEntry> = BTreeMap::new();

        // Project-local: walk from cwd up to git root.
        if let Ok(cwd) = std::env::current_dir() {
            for dir in walk_to_git_root_mcp(&cwd) {
                for &sub in LOCAL_MCP_CONFIG_FILES {
                    let path = dir.join(sub);
                    if let Ok(text) = fs::read_to_string(&path) {
                        if !text.trim().is_empty() {
                            match parse_server_entries(&text) {
                                Ok(entries) => {
                                    for (k, v) in entries {
                                        merged.entry(k).or_insert(v);
                                    }
                                }
                                Err(e) => warnings.push(format!(
                                    "MCP config `{}` is invalid: {e}. Skipped.",
                                    path.display()
                                )),
                            }
                        }
                    }
                }
            }
        }

        // Global: under home directory.
        if let Some(home) = crate::tools::home_dir() {
            for &sub in GLOBAL_MCP_CONFIG_FILES {
                let path = home.join(sub);
                if let Ok(text) = fs::read_to_string(&path) {
                    if !text.trim().is_empty() {
                        match parse_server_entries(&text) {
                            Ok(entries) => {
                                for (k, v) in entries {
                                    merged.entry(k).or_insert(v);
                                }
                            }
                            Err(e) => warnings.push(format!(
                                "MCP config `{}` is invalid: {e}. Skipped.",
                                path.display()
                            )),
                        }
                    }
                }
            }
            for &sub in GLOBAL_TOML_CONFIG_FILES {
                let path = home.join(sub);
                if let Ok(text) = fs::read_to_string(&path) {
                    if !text.trim().is_empty() {
                        match parse_toml_mcp_servers(&text) {
                            Ok(entries) => {
                                for (k, v) in entries {
                                    merged.entry(k).or_insert(v);
                                }
                            }
                            Err(e) => warnings.push(format!(
                                "MCP config `{}` is invalid: {e}. Skipped.",
                                path.display()
                            )),
                        }
                    }
                }
            }
        }

        if merged.is_empty() {
            return (Self::default(), warnings);
        }

        let mut registry = Self::default();
        Self::populate_from_entries(&mut registry, merged, &mut warnings);
        (registry, warnings)
    }

    fn populate_from_entries(
        registry: &mut Self,
        entries: BTreeMap<String, RawServerEntry>,
        warnings: &mut Vec<String>,
    ) {
        for (server_name, entry) in entries {
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
                        if ty == "remote" {
                            warnings.push(format!(
                                "MCP server `{server_name}` type `remote` is not supported yet (GoldBot currently supports local stdio only). Skipped."
                            ));
                        } else {
                            warnings.push(format!(
                                "MCP server `{server_name}` type `{ty}` is not supported yet (local stdio only). Skipped."
                            ));
                        }
                        continue;
                    }
                    let Some((command, args)) = extract_local_command_and_args(&cfg) else {
                        warnings.push(format!(
                            "MCP server `{server_name}` is missing a valid command. Use either `\"command\":\"npx\"` or `\"command\":[\"npx\",\"-y\",...]`. Skipped."
                        ));
                        continue;
                    };
                    let mut env = cfg.env.clone();
                    for (k, v) in &cfg.headers {
                        env.entry(k.clone()).or_insert_with(|| v.clone());
                    }

                    registry.servers.insert(
                        server_name,
                        LocalServerSpec {
                            command,
                            args,
                            env,
                            cwd: cfg.cwd.map(PathBuf::from),
                            transport: cfg.transport.clone(),
                        },
                    );
                }
            }
        }
    }

    /// Whether any MCP servers are configured (discovery not yet run).
    pub fn has_servers(&self) -> bool {
        !self.servers.is_empty()
    }

    /// 注入当前后端对应的内置 MCP 服务器。
    /// 先移除之前注入的内置服务器（以 "builtin_" 开头），再注入新后端的。
    /// 需在 run_discovery 之前调用，discovery 时会一并处理内置服务器。
    pub fn inject_builtin_for_backend(&mut self, backend_label: &str) {
        // 移除之前注入的内置服务器
        self.servers.retain(|k, _| !k.starts_with("builtin_"));
        self.tools
            .retain(|_, v| !v.server_name.starts_with("builtin_"));
        self.failed.retain(|k| !k.starts_with("builtin_"));

        match backend_label {
            "MiniMax" => {
                let mut env = HashMap::new();
                env.insert(
                    "MINIMAX_API_HOST".to_string(),
                    "https://api.minimaxi.com".to_string(),
                );
                // MINIMAX_API_KEY 直接从父进程环境继承，无需显式传递
                self.servers.insert(
                    "builtin_minimax".to_string(),
                    LocalServerSpec {
                        command: "uvx".to_string(),
                        args: vec!["minimax-coding-plan-mcp".to_string(), "-y".to_string()],
                        env,
                        cwd: None,
                        transport: None,
                    },
                );
            }
            _ => {}
        }
    }

    /// Run tool discovery synchronously. Intended to be called from a background thread.
    pub fn run_discovery(mut self) -> (Self, Vec<String>) {
        if self.servers.is_empty() {
            return (self, Vec::new());
        }
        let warnings = self.discover_tools(mcp_discovery_timeout());
        (self, warnings)
    }

    pub fn startup_status(&self) -> McpStartupStatus {
        let mut tool_counts: HashMap<&str, usize> = HashMap::new();
        for tool in self.tools.values() {
            *tool_counts.entry(tool.server_name.as_str()).or_insert(0) += 1;
        }
        let ok = self
            .servers
            .keys()
            .filter(|name| !self.failed.contains(*name))
            .map(|name| (name.clone(), *tool_counts.get(name.as_str()).unwrap_or(&0)))
            .collect();
        McpStartupStatus {
            ok,
            failed: self.failed.clone(),
        }
    }

    pub fn augment_system_prompt(&self, base_prompt: &str) -> String {
        if self.tools.is_empty() {
            return base_prompt.to_string();
        }

        let mut out = String::new();
        out.push_str(base_prompt);
        out.push_str(
            "\n\n## Available MCP tools\n\
             Use the MCP call format above. `<tool>` must be exactly one name from this list;\
             `<arguments>` must be a JSON object.\n\
             Prefer shell for filesystem/terminal work; use MCP for external context or APIs.\n\n",
        );

        for tool in self.tools.values().take(MAX_PROMPT_TOOLS) {
            let desc = truncate_chars(&tool.description, MAX_DESC_CHARS);
            let args = summarize_input_schema(&tool.input_schema);
            let ro = if tool.read_only_hint {
                "read-only"
            } else {
                "read/write"
            };
            let extra = if tool.server_name == "context7" && tool.tool_name == "resolve-library-id"
            {
                " | note: provide BOTH `libraryName` and `query`"
            } else {
                ""
            };
            out.push_str(&format!(
                "- {} => server=`{}` tool=`{}` ({ro}), args: {}, desc: {}{}\n",
                tool.action_name,
                tool.server_name,
                tool.tool_name,
                args,
                fallback_if_empty(&desc, "(no description)"),
                extra
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
        let Some(tool) = self.resolve_tool_spec(action_name) else {
            let suggestions = self.suggest_tool_names(action_name, 5);
            if suggestions.is_empty() {
                bail!("unknown MCP tool `{action_name}`");
            }
            bail!(
                "unknown MCP tool `{action_name}`. Try one of: {}",
                suggestions.join(", ")
            );
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

        let normalized_arguments = normalize_arguments_for_tool(tool, arguments);
        call_tool_once(server, &tool.tool_name, &normalized_arguments)
    }

    fn resolve_tool_spec(&self, action_name: &str) -> Option<&McpToolSpec> {
        if let Some(spec) = self.tools.get(action_name) {
            return Some(spec);
        }

        let normalized = normalize_action_name_for_lookup(action_name)?;
        self.tools.get(&normalized)
    }

    fn suggest_tool_names(&self, action_name: &str, limit: usize) -> Vec<String> {
        let normalized = normalize_action_name_for_lookup(action_name).unwrap_or_default();
        let needle = normalized.strip_prefix("mcp_").unwrap_or(&normalized);

        let mut out: Vec<String> = self
            .tools
            .keys()
            .filter(|name| {
                let hay = name.as_str();
                if needle.is_empty() {
                    return false;
                }
                hay.contains(needle) || needle.contains(hay.trim_start_matches("mcp_"))
            })
            .take(limit)
            .cloned()
            .collect();

        if out.is_empty() {
            out.extend(self.tools.keys().take(limit).cloned());
        }
        out
    }

    fn discover_tools(&mut self, timeout: Duration) -> Vec<String> {
        let mut warnings = Vec::new();
        let mut used_names = BTreeSet::new();

        for (server_name, server) in &self.servers {
            match list_tools_with_timeout(server, timeout) {
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
                Err(e) => {
                    self.failed.push(server_name.clone());
                    warnings.push(format!(
                        "Failed to load MCP tools from `{server_name}`: {e}. Server skipped."
                    ));
                }
            }
        }

        warnings
    }
}

/// Parse MCP server entries from a Codex-style TOML config.
/// Reads the `[mcp_servers.<name>]` tables and converts them to our internal entry map.
fn parse_toml_mcp_servers(
    text: &str,
) -> std::result::Result<BTreeMap<String, RawServerEntry>, String> {
    let doc: toml::Value = toml::from_str(text).map_err(|e| format!("not valid TOML: {e}"))?;

    let Some(mcp_servers) = doc.get("mcp_servers").and_then(|v| v.as_table()) else {
        return Ok(BTreeMap::new());
    };

    let mut out = BTreeMap::new();
    for (name, val) in mcp_servers {
        let Some(table) = val.as_table() else {
            continue;
        };

        // Skip remote servers (have `url` but no `command`).
        if table.contains_key("url") && !table.contains_key("command") {
            continue;
        }

        let command = match table.get("command").and_then(|v| v.as_str()) {
            Some(s) if !s.trim().is_empty() => Some(RawCommand::String(s.trim().to_string())),
            _ => None,
        };
        let mut args: Vec<String> = table
            .get("args")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str())
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        // Codex configs often omit -y; add it so npx doesn't hang waiting for input.
        if matches!(&command, Some(RawCommand::String(s)) if s == "npx")
            && !args.iter().any(|a| a == "-y")
        {
            args.insert(0, "-y".to_string());
        }
        let env: HashMap<String, String> = table
            .get("env")
            .and_then(|v| v.as_table())
            .map(|t| {
                t.iter()
                    .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                    .collect()
            })
            .unwrap_or_default();
        let cwd: Option<String> = table
            .get("cwd")
            .and_then(|v| v.as_str())
            .map(str::to_string);
        let enabled = table
            .get("enabled")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        out.insert(
            name.clone(),
            RawServerEntry::Config(RawServerConfig {
                r#type: "local".to_string(),
                command,
                args,
                env,
                headers: HashMap::new(),
                cwd,
                enabled,
                transport: None,
                url: None,
            }),
        );
    }
    Ok(out)
}

fn walk_to_git_root_mcp(start: &std::path::Path) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let mut cur = start.to_path_buf();
    loop {
        dirs.push(cur.clone());
        if cur.join(".git").exists() {
            break;
        }
        match cur.parent() {
            Some(p) if p != cur => cur = p.to_path_buf(),
            _ => break,
        }
    }
    dirs
}

fn parse_server_entries(
    raw: &str,
) -> std::result::Result<BTreeMap<String, RawServerEntry>, String> {
    let value: Value = serde_json::from_str(raw).map_err(|e| format!("not valid JSON: {e}"))?;
    let normalized = normalize_server_root(value)?;
    serde_json::from_value(normalized).map_err(|e| format!("unsupported config shape: {e}"))
}

fn normalize_server_root(value: Value) -> std::result::Result<Value, String> {
    let Some(root) = value.as_object() else {
        return Err("top-level JSON must be an object".to_string());
    };

    if let Some(inner) = root.get("mcp").and_then(Value::as_object) {
        return Ok(Value::Object(inner.clone()));
    }
    if let Some(inner) = root.get("mcpServers").and_then(Value::as_object) {
        return Ok(Value::Object(inner.clone()));
    }

    Ok(value)
}

fn extract_local_command_and_args(cfg: &RawServerConfig) -> Option<(String, Vec<String>)> {
    let mut args = Vec::new();
    let command = match cfg.command.as_ref()? {
        RawCommand::String(cmd) => {
            let trimmed = cmd.trim();
            if trimmed.is_empty() {
                return None;
            }
            trimmed.to_string()
        }
        RawCommand::Array(parts) => {
            let mut iter = parts
                .iter()
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .map(ToString::to_string);
            let command = iter.next()?;
            args.extend(iter);
            command
        }
    };

    args.extend(
        cfg.args
            .iter()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(ToString::to_string),
    );

    Some((command, args))
}

/// Returns GoldBot's home directory (`$GOLDBOT_MEMORY_DIR` or `~/.goldbot`).
pub fn goldbot_home_dir() -> PathBuf {
    default_memory_base_dir()
}

/// Returns the resolved path of the MCP servers config file.
pub fn mcp_servers_file_path() -> PathBuf {
    resolve_mcp_servers_file_path()
}

/// Build the assistant-prompt appendix for `create_mcp` guidance.
/// This is intended for command-specific injection (not always-on system prompt).
#[allow(dead_code)]
pub fn create_mcp_assist_prompt_appendix() -> String {
    let path = mcp_servers_file_path();
    CREATE_MCP_ASSIST_PROMPT_APPENDIX_TEMPLATE.replace("{MCP_CONFIG_PATH}", &path.to_string_lossy())
}

/// Add or overwrite a server entry in the MCP config file.
/// `config` must be a JSON object with at minimum a `command` field.
/// Returns the path of the config file that was written.
pub fn create_mcp_server(name: &str, config: &serde_json::Value) -> anyhow::Result<PathBuf> {
    use anyhow::{Context, bail};

    if name.trim().is_empty() {
        bail!("MCP server name must not be empty");
    }
    if !config.is_object() {
        bail!("MCP server config must be a JSON object");
    }
    if config.get("command").is_none() {
        bail!("MCP server config requires a `command` field");
    }

    let path = mcp_servers_file_path();

    // Read existing config, or start fresh.
    let mut root: serde_json::Map<String, serde_json::Value> = match fs::read_to_string(&path)
        .ok()
        .filter(|s| !s.trim().is_empty())
    {
        Some(text) => match serde_json::from_str::<serde_json::Value>(&text)
            .ok()
            .and_then(|v| v.as_object().cloned())
        {
            Some(obj) => obj,
            None => serde_json::Map::new(),
        },
        None => serde_json::Map::new(),
    };

    // Normalise into canonical format: command array, explicit type + enabled.
    let mut spec = config.as_object().cloned().unwrap_or_default();
    spec.remove("name");

    // Merge `command` (string or array) + `args` into a single command array.
    let cmd_val = spec.remove("command");
    let args_val = spec.remove("args");
    let mut cmd_parts: Vec<serde_json::Value> = match &cmd_val {
        Some(serde_json::Value::Array(arr)) => arr.clone(),
        Some(serde_json::Value::String(s)) if !s.trim().is_empty() => {
            vec![serde_json::Value::String(s.clone())]
        }
        _ => vec![],
    };
    if let Some(serde_json::Value::Array(extra)) = args_val {
        cmd_parts.extend(extra);
    }
    spec.insert("command".to_string(), serde_json::Value::Array(cmd_parts));

    // Always write type and enabled explicitly.
    spec.insert(
        "type".to_string(),
        serde_json::Value::String("local".to_string()),
    );
    spec.insert("enabled".to_string(), serde_json::Value::Bool(true));

    // Remove empty env / headers.
    for key in &["env", "headers"] {
        if spec
            .get(*key)
            .and_then(|v| v.as_object())
            .map_or(false, |m| m.is_empty())
        {
            spec.remove(*key);
        }
    }

    let spec_value = serde_json::Value::Object(spec);

    // Insert server at the right level (handle mcpServers/mcp wrappers).
    if let Some(inner) = root.get_mut("mcpServers").and_then(|v| v.as_object_mut()) {
        inner.insert(name.to_string(), spec_value);
    } else if let Some(inner) = root.get_mut("mcp").and_then(|v| v.as_object_mut()) {
        inner.insert(name.to_string(), spec_value);
    } else {
        root.insert(name.to_string(), spec_value);
    }

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create config dir `{}`", parent.display()))?;
    }
    fs::write(
        &path,
        serde_json::to_string_pretty(&serde_json::Value::Object(root))?,
    )
    .with_context(|| format!("failed to write MCP config `{}`", path.display()))?;

    Ok(path)
}

fn resolve_mcp_servers_file_path() -> PathBuf {
    if let Some(path) = std::env::var_os(ENV_MCP_SERVERS_FILE) {
        let p = PathBuf::from(path);
        if !p.as_os_str().is_empty() {
            return p;
        }
    }

    default_memory_base_dir().join(DEFAULT_MCP_SERVERS_FILENAME)
}

fn mcp_discovery_timeout() -> Duration {
    let ms = std::env::var(ENV_MCP_DISCOVERY_TIMEOUT_MS)
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(DEFAULT_MCP_DISCOVERY_TIMEOUT_MS);
    Duration::from_millis(ms)
}

fn default_memory_base_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os(ENV_MEMORY_DIR) {
        let p = PathBuf::from(dir);
        if !p.as_os_str().is_empty() {
            return p;
        }
    }

    if let Some(home) = crate::tools::home_dir() {
        return home.join(".goldbot");
    }

    PathBuf::from(".goldbot")
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

fn list_tools_with_timeout(
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
    wire_format: StdioWireFormat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StdioWireFormat {
    Framed,
    LineDelimited,
}

impl StdioMcpSession {
    fn spawn(spec: &LocalServerSpec) -> Result<Self> {
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

fn normalize_action_name_for_lookup(action_name: &str) -> Option<String> {
    let trimmed = action_name.trim();
    if trimmed.is_empty() {
        return None;
    }

    let normalized = sanitize_token(trimmed);
    if normalized.is_empty() {
        None
    } else {
        Some(normalized)
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

fn normalize_arguments_for_tool(tool: &McpToolSpec, arguments: &Value) -> Value {
    let Some(obj) = arguments.as_object() else {
        return arguments.clone();
    };

    let mut normalized = obj.clone();

    // Context7 `resolve-library-id` requires both `libraryName` and `query`.
    // When the model only provides `libraryName`, mirror it into `query`
    // to avoid a noisy first-call validation failure.
    if tool.server_name == "context7"
        && tool.tool_name == "resolve-library-id"
        && !normalized.contains_key("query")
        && let Some(library_name) = normalized
            .get("libraryName")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
    {
        normalized.insert("query".to_string(), Value::String(library_name.to_string()));
    }

    Value::Object(normalized)
}

#[cfg(test)]
mod tests {
    use super::{
        McpToolSpec, RawServerEntry, extract_local_command_and_args,
        normalize_action_name_for_lookup, normalize_arguments_for_tool, parse_server_entries,
        sanitize_token, summarize_input_schema, unique_action_name,
    };
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

    #[test]
    fn parse_server_entries_accepts_mcp_wrapper() {
        let raw = r#"{
            "mcp": {
                "context7": { "type": "local", "command": "npx", "args": ["-y", "@upstash/context7-mcp"] }
            }
        }"#;
        let parsed = parse_server_entries(raw).expect("parse should succeed");
        assert!(matches!(
            parsed.get("context7"),
            Some(RawServerEntry::Config(_))
        ));
    }

    #[test]
    fn command_array_is_supported() {
        let raw = r#"{
            "context7": {
                "type": "local",
                "command": ["npx", "-y", "@upstash/context7-mcp", "--api-key", "k"]
            }
        }"#;
        let parsed = parse_server_entries(raw).expect("parse should succeed");
        let RawServerEntry::Config(cfg) = parsed.get("context7").expect("missing config") else {
            panic!("expected config entry");
        };
        let (cmd, args) = extract_local_command_and_args(cfg).expect("command should parse");
        assert_eq!(cmd, "npx");
        assert_eq!(args, vec!["-y", "@upstash/context7-mcp", "--api-key", "k"]);
    }

    #[test]
    fn normalize_action_name_handles_double_underscore() {
        assert_eq!(
            normalize_action_name_for_lookup("mcp__context7__get_repository").as_deref(),
            Some("mcp_context7_get_repository")
        );
    }

    #[test]
    fn context7_resolve_library_id_autofills_query() {
        let spec = McpToolSpec {
            action_name: "mcp_context7_resolve_library_id".to_string(),
            server_name: "context7".to_string(),
            tool_name: "resolve-library-id".to_string(),
            description: String::new(),
            read_only_hint: true,
            input_schema: json!({}),
        };
        let args = json!({ "libraryName": "tokio" });
        let normalized = normalize_arguments_for_tool(&spec, &args);
        assert_eq!(normalized, json!({"libraryName":"tokio","query":"tokio"}));
    }
}
