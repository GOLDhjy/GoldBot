use crate::types::LlmAction;
use anyhow::{Result, anyhow};
use serde_json::Value;

const SYSTEM_PROMPT_TEMPLATE: &str = "\
You are GoldBot, a terminal automation agent. Complete tasks step by step using the tools below.

## Response format

Shell command:
<thought>reasoning</thought>
<tool>shell</tool>
<command>bash command</command>

MCP tool (only if listed later in this prompt):
<thought>reasoning</thought>
<tool>exact_mcp_tool_name</tool>
<arguments>{\"key\":\"value\"}</arguments>

Load a skill (only if listed later in this prompt):
<thought>reasoning</thought>
<skill>skill-name</skill>

Create a new MCP server:
<thought>reasoning</thought>
<create_mcp>{\"name\":\"server-name\",\"command\":[\"npx\",\"-y\",\"@scope/pkg\"]}</create_mcp>


Task complete:
<thought>reasoning</thought>
<final>outcome summary</final>

## Rules
- One tool call per response; wait for the result before proceeding.
- Use <final> as soon as done; avoid extra commands.
- <final> is rendered in the terminal: headings (#/##), lists (-/*), inline **bold**/`code`, and diffs are all supported. Use them for clarity.
- Prefer read-only commands unless changes are required.
- On failure, diagnose from output and try a different approach.
- Shell: bash (macOS/Linux).

## create_mcp fields
- `name` (required): server identifier used as the config key
- `command` (required): **array** with executable and all arguments, e.g. `[\"npx\",\"-y\",\"@scope/pkg\",\"--flag\",\"val\"]`
- `env` (optional): env vars object, e.g. `{{\"API_KEY\":\"val\"}}` — omit if not needed
- `cwd` (optional): working directory — omit if not needed
`type` and `enabled` are added automatically. Config is written to {MCP_CONFIG_PATH}. Tell the user to restart GoldBot to activate.

## Creating skills
Use shell commands to create a skill directory and SKILL.md file:
  Skills directory: {SKILLS_DIR}
  Structure: {SKILLS_DIR}/<name>/SKILL.md
  SKILL.md format:
    ---
    name: <name>
    description: one-line summary
    ---

    # Markdown content (free-form)
Use `printf` or `python3 -c` to write the file. Tell the user to restart GoldBot to load it.";

/// Build the base system prompt with the actual MCP config file path substituted in.
pub fn build_system_prompt() -> String {
    let mcp_path = crate::tools::mcp::mcp_servers_file_path();
    let skills_dir = crate::tools::skills::goldbot_skills_dir();
    SYSTEM_PROMPT_TEMPLATE
        .replace("{MCP_CONFIG_PATH}", &mcp_path.to_string_lossy())
        .replace("{SKILLS_DIR}", &skills_dir.to_string_lossy())
}

/// Parse the raw text returned by the LLM into a thought + action pair.
pub fn parse_llm_response(text: &str) -> Result<(String, LlmAction)> {
    let thought = extract_last_tag(text, "thought").unwrap_or_default();

    if let Some(summary) = extract_last_tag(text, "final") {
        return Ok((thought, LlmAction::Final { summary }));
    }

    if let Some(name) = extract_last_tag(text, "skill") {
        return Ok((thought, LlmAction::Skill { name }));
    }

    if let Some(raw) = extract_last_tag(text, "create_mcp") {
        let config: Value =
            serde_json::from_str(&raw).map_err(|e| anyhow!("invalid <create_mcp> JSON: {e}"))?;
        if !config.is_object() {
            return Err(anyhow!("<create_mcp> must be a JSON object"));
        }
        return Ok((thought, LlmAction::CreateMcp { config }));
    }

    if let Some(tool) = extract_last_tag(text, "tool") {
        if tool == "shell" {
            let command = extract_last_tag(text, "command")
                .ok_or_else(|| anyhow!("missing <command> for shell tool call"))?;
            return Ok((thought, LlmAction::Shell { command }));
        }

        if tool.starts_with("mcp_") {
            let raw_args = extract_last_tag(text, "arguments").unwrap_or_else(|| "{}".to_string());
            let arguments: Value = serde_json::from_str(&raw_args)
                .map_err(|e| anyhow!("invalid <arguments> JSON for MCP tool call: {e}"))?;
            if !arguments.is_object() {
                return Err(anyhow!("MCP <arguments> must be a JSON object"));
            }
            return Ok((thought, LlmAction::Mcp { tool, arguments }));
        }

        return Err(anyhow!("unsupported tool `{tool}`"));
    }

    // Backward compatibility with older prompt format that only emitted <command>.
    if let Some(command) = extract_last_tag(text, "command") {
        return Ok((thought, LlmAction::Shell { command }));
    }

    Err(anyhow!(
        "cannot parse LLM response — expected shell call, MCP call, or <final>...</final>"
    ))
}

fn extract_last_tag(text: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let end = text.rfind(&close)?;
    let head = &text[..end];
    let start = head.rfind(&open)? + open.len();
    Some(head[start..].trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::parse_llm_response;
    use crate::types::LlmAction;
    use serde_json::json;

    #[test]
    fn parse_final_prefers_last_closed_tag() {
        let raw = "<thought>ok</thought><final>bad <final>good</final>";
        let (_, action) = parse_llm_response(raw).expect("should parse final");
        match action {
            LlmAction::Final { summary } => assert_eq!(summary, "good"),
            _ => panic!("expected final action"),
        }
    }

    #[test]
    fn parse_error_does_not_echo_raw_text() {
        let raw = "plain response without tags";
        let err = parse_llm_response(raw).expect_err("should fail");
        let msg = err.to_string();
        assert!(msg.contains("cannot parse LLM response"));
        assert!(!msg.contains(raw));
    }

    #[test]
    fn parse_mcp_tool_call() {
        let raw = "<thought>need docs</thought><tool>mcp_context7_resolve_library</tool><arguments>{\"libraryName\":\"tokio\"}</arguments>";
        let (_, action) = parse_llm_response(raw).expect("should parse MCP action");
        match action {
            LlmAction::Mcp { tool, arguments } => {
                assert_eq!(tool, "mcp_context7_resolve_library");
                assert_eq!(arguments, json!({"libraryName":"tokio"}));
            }
            _ => panic!("expected MCP action"),
        }
    }

    #[test]
    fn parse_mcp_arguments_requires_json_object() {
        let raw = "<tool>mcp_context7_lookup</tool><arguments>[1,2,3]</arguments>";
        let err = parse_llm_response(raw).expect_err("should fail");
        assert!(err.to_string().contains("JSON object"));
    }
}
