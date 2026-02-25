use std::{
    collections::{BTreeMap, HashMap},
    path::PathBuf,
};

use serde_json::Value;

#[derive(Debug, Clone, Default)]
pub struct McpRegistry {
    pub(super) servers: BTreeMap<String, ServerSpec>,
    pub(super) tools: BTreeMap<String, McpToolSpec>,
    pub(super) failed: Vec<String>,
}

pub struct McpStartupStatus {
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
pub(super) struct LocalServerSpec {
    pub(super) command: String,
    pub(super) args: Vec<String>,
    pub(super) env: HashMap<String, String>,
    pub(super) cwd: Option<PathBuf>,
    pub(super) transport: Option<String>,
}

#[derive(Debug, Clone)]
pub(super) struct RemoteServerSpec {
    pub(super) url: String,
    pub(super) headers: HashMap<String, String>,
}

#[derive(Debug, Clone)]
pub(super) enum ServerSpec {
    Local(LocalServerSpec),
    Remote(RemoteServerSpec),
}

#[derive(Debug, Clone)]
pub(super) struct DiscoveredTool {
    pub(super) tool_name: String,
    pub(super) description: String,
    pub(super) input_schema: Value,
    pub(super) read_only_hint: bool,
}
