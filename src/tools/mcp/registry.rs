use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    fs,
    path::PathBuf,
    time::Duration,
};

use anyhow::{Result, bail};
use serde_json::Value;

use super::{
    ENV_MCP_SERVERS, ENV_MCP_SERVERS_FILE, GLOBAL_MCP_CONFIG_FILES, GLOBAL_TOML_CONFIG_FILES,
    LOCAL_MCP_CONFIG_FILES, MAX_DESC_CHARS, MAX_PROMPT_TOOLS,
    config::{
        RawServerEntry, extract_local_command_and_args, mcp_discovery_timeout,
        parse_server_entries, parse_toml_mcp_servers, walk_to_git_root_mcp,
    },
    discovery::list_tools_for_server,
    executor::{call_tool_once, call_tool_remote},
    types::{
        DiscoveredTool, LocalServerSpec, McpCallResult, McpRegistry, McpStartupStatus, McpToolSpec,
        RemoteServerSpec, ServerSpec,
    },
    util::{
        fallback_if_empty, normalize_action_name_for_lookup, normalize_arguments_for_tool,
        resolve_env_var_refs, summarize_input_schema, truncate_chars, unique_action_name,
    },
};

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
                    if ty == "remote" {
                        let Some(url) = cfg.url.clone() else {
                            warnings.push(format!(
                                "MCP server `{server_name}` type `remote` is missing `url`. Skipped."
                            ));
                            continue;
                        };
                        let headers: HashMap<String, String> = cfg
                            .headers
                            .iter()
                            .map(|(k, v)| (k.clone(), resolve_env_var_refs(v)))
                            .collect();
                        registry
                            .servers
                            .insert(server_name, ServerSpec::Remote(RemoteServerSpec { url, headers }));
                        continue;
                    } else if ty != "local" {
                        warnings.push(format!(
                            "MCP server `{server_name}` type `{ty}` is not supported yet. Skipped."
                        ));
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
                        ServerSpec::Local(LocalServerSpec {
                            command,
                            args,
                            env,
                            cwd: cfg.cwd.map(PathBuf::from),
                            transport: cfg.transport.clone(),
                        }),
                    );
                }
            }
        }
    }

    /// Whether any MCP servers are configured (discovery not yet run).
    pub fn has_servers(&self) -> bool {
        !self.servers.is_empty()
    }

    /// Inject backend-specific built-in MCP servers before discovery.
    pub fn inject_builtin_for_backend(&mut self, backend_label: &str) {
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
                self.servers.insert(
                    "builtin_minimax".to_string(),
                    ServerSpec::Local(LocalServerSpec {
                        command: "uvx".to_string(),
                        args: vec!["minimax-coding-plan-mcp".to_string(), "-y".to_string()],
                        env,
                        cwd: None,
                        transport: None,
                    }),
                );
            }
            "GLM" => {
                let mut env = HashMap::new();
                if let Ok(key) = std::env::var("BIGMODEL_API_KEY") {
                    env.insert("Z_AI_API_KEY".to_string(), key);
                }
                env.insert("Z_AI_MODE".to_string(), "ZHIPU".to_string());
                self.servers.insert(
                    "builtin_zai_mcp_server".to_string(),
                    ServerSpec::Local(LocalServerSpec {
                        command: "npx".to_string(),
                        args: vec!["-y".to_string(), "@z_ai/mcp-server".to_string()],
                        env,
                        cwd: None,
                        transport: None,
                    }),
                );

                let mut headers = HashMap::new();
                if let Ok(key) = std::env::var("BIGMODEL_API_KEY") {
                    headers.insert("Authorization".to_string(), format!("Bearer {key}"));
                }
                self.servers.insert(
                    "builtin_web_search_prime".to_string(),
                    ServerSpec::Remote(RemoteServerSpec {
                        url: "https://open.bigmodel.cn/api/mcp/web_search_prime/mcp".to_string(),
                        headers: headers.clone(),
                    }),
                );
                self.servers.insert(
                    "builtin_zread".to_string(),
                    ServerSpec::Remote(RemoteServerSpec {
                        url: "https://open.bigmodel.cn/api/mcp/zread/mcp".to_string(),
                        headers,
                    }),
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

        let normalized_arguments = normalize_arguments_for_tool(tool, arguments);
        let tool_name = tool.tool_name.clone();
        let server_name = tool.server_name.clone();

        if let Some(server) = self.servers.get(&server_name) {
            return match server {
                ServerSpec::Local(server) => {
                    call_tool_once(server, &tool_name, &normalized_arguments)
                }
                ServerSpec::Remote(server) => {
                    call_tool_remote(server, &tool_name, &normalized_arguments)
                }
            };
        }

        bail!(
            "MCP server `{}` is not available for tool `{}`",
            server_name,
            action_name
        );
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

    fn register_discovered_tools(
        &mut self,
        server_name: &str,
        tools: Vec<DiscoveredTool>,
        used_names: &mut BTreeSet<String>,
    ) {
        for tool in tools {
            let action_name = unique_action_name(server_name, &tool.tool_name, used_names);
            self.tools.insert(
                action_name.clone(),
                McpToolSpec {
                    action_name,
                    server_name: server_name.to_string(),
                    tool_name: tool.tool_name,
                    description: tool.description,
                    read_only_hint: tool.read_only_hint,
                    input_schema: tool.input_schema,
                },
            );
        }
    }

    fn record_discovery_failure(
        &mut self,
        server_name: &str,
        error: anyhow::Error,
        warnings: &mut Vec<String>,
    ) {
        self.failed.push(server_name.to_string());
        warnings.push(format!(
            "Failed to load MCP tools from `{server_name}`: {error}. Server skipped."
        ));
    }

    fn discover_tools(&mut self, timeout: Duration) -> Vec<String> {
        let mut warnings = Vec::new();
        let mut used_names = BTreeSet::new();
        let server_entries: Vec<(String, ServerSpec)> = self
            .servers
            .iter()
            .map(|(name, spec)| (name.clone(), spec.clone()))
            .collect();

        for (server_name, server) in &server_entries {
            match list_tools_for_server(server, timeout) {
                Ok(tools) => self.register_discovered_tools(server_name, tools, &mut used_names),
                Err(e) => self.record_discovery_failure(server_name, e, &mut warnings),
            }
        }

        warnings
    }
}
