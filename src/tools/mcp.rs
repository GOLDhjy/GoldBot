mod config;
mod discovery;
mod executor;
mod protocol;
mod registry;
#[cfg(test)]
mod tests;
mod types;
mod util;

#[allow(unused_imports)]
pub use self::config::{
    create_mcp_assist_prompt_appendix, create_mcp_server, goldbot_home_dir, mcp_servers_file_path,
};
#[allow(unused_imports)]
pub use self::types::{McpCallResult, McpRegistry, McpStartupStatus, McpToolSpec};

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
- `env` (optional): env vars object, e.g. `{{\"API_KEY\":\"val\"}}` - omit if not needed
- `cwd` (optional): working directory - omit if not needed
`type` and `enabled` are added automatically. Config is written to {MCP_CONFIG_PATH}. Tell the user to restart GoldBot to activate.";
