use std::{
    collections::{BTreeMap, HashMap},
    fs,
    path::PathBuf,
    time::Duration,
};

use serde::Deserialize;
use serde_json::Value;

use super::{
    CREATE_MCP_ASSIST_PROMPT_APPENDIX_TEMPLATE, DEFAULT_MCP_DISCOVERY_TIMEOUT_MS,
    DEFAULT_MCP_SERVERS_FILENAME, ENV_MCP_DISCOVERY_TIMEOUT_MS, ENV_MCP_SERVERS_FILE,
    ENV_MEMORY_DIR,
};

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub(super) enum RawServerEntry {
    Disabled(bool),
    Config(RawServerConfig),
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct RawServerConfig {
    #[serde(default = "default_server_type")]
    pub(super) r#type: String,
    pub(super) command: Option<RawCommand>,
    #[serde(default)]
    pub(super) args: Vec<String>,
    #[serde(default)]
    pub(super) env: HashMap<String, String>,
    #[serde(default)]
    pub(super) headers: HashMap<String, String>,
    pub(super) cwd: Option<String>,
    #[serde(default = "default_enabled")]
    pub(super) enabled: bool,
    /// Wire format override: "line" (default) or "framed" (Content-Length/LSP style).
    pub(super) transport: Option<String>,
    #[allow(dead_code)]
    pub(super) url: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub(super) enum RawCommand {
    String(String),
    Array(Vec<String>),
}

fn default_server_type() -> String {
    "local".to_string()
}

fn default_enabled() -> bool {
    true
}

/// Parse MCP server entries from a Codex-style TOML config.
/// Reads the `[mcp_servers.<name>]` tables and converts them to our internal entry map.
pub(super) fn parse_toml_mcp_servers(
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

pub(super) fn walk_to_git_root_mcp(start: &std::path::Path) -> Vec<PathBuf> {
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

pub(super) fn parse_server_entries(
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

pub(super) fn extract_local_command_and_args(
    cfg: &RawServerConfig,
) -> Option<(String, Vec<String>)> {
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

pub(super) fn mcp_discovery_timeout() -> Duration {
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
