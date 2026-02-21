use crate::types::LlmAction;
use anyhow::{Result, anyhow};
use serde_json::Value;

const SYSTEM_PROMPT_TEMPLATE: &str = "\
You are GoldBot, a terminal automation agent. Complete tasks step by step using the tools below.

## Response format

重要：遵循以下决策顺序：
1. 任务信息不足或有歧义 → 先用 question 工具向用户提问，获取足够信息后再规划
2. 信息充足但任务复杂 → 用 plan 工具输出完整计划，等用户确认后再执行
3. 任务简单且信息明确 → 直接执行，无需 plan

Plan 模式（信息充足时，输出具体可执行计划；plan 之后必须立即用 question 工具询问用户是否确认）：
<thought>reasoning</thought>
<tool>plan</tool>
<plan>## 计划
1. 第一步：...
2. 第二步：...
</plan>

向用户提问（需要澄清或让用户做选择时）：
- 必须提供 3 个预设选项 + 1 个自定义输入选项
- question 标签内写问题，每个 option 是一个选项
<thought>reasoning</thought>
<tool>question</tool>
<question>问题内容</question>
<option>选项A</option>
<option>选项B</option>
<option>选项C</option>
<option><user_input></option>

Shell command:
<thought>reasoning</thought>
<tool>shell</tool>
<command>bash command</command>

Web search (use when you need up-to-date or online information):
<thought>reasoning</thought>
<tool>web_search</tool>
<query>search query</query>

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

        if tool == "web_search" {
            let query = extract_last_tag(text, "query")
                .ok_or_else(|| anyhow!("missing <query> for web_search tool call"))?;
            return Ok((thought, LlmAction::WebSearch { query }));
        }

        if tool == "plan" {
            let content = extract_last_tag(text, "plan")
                .ok_or_else(|| anyhow!("missing <plan> for plan tool call"))?;
            return Ok((thought, LlmAction::Plan { content: strip_xml_tags(&content) }));
        }

        if tool == "question" {
            let text_q = extract_last_tag(text, "question")
                .ok_or_else(|| anyhow!("missing <question> for question tool call"))?;
            let options: Vec<String> = extract_all_tags(text, "option")
                .into_iter()
                .map(|o| {
                    // Normalize any <user_input> tag variant to the canonical sentinel.
                    if o.trim().starts_with("<user_input") {
                        "<user_input>".to_string()
                    } else {
                        o
                    }
                })
                .collect();
            if options.is_empty() {
                return Err(anyhow!("missing <option> for question tool call"));
            }
            return Ok((thought, LlmAction::Question { text: strip_xml_tags(&text_q), options }));
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

fn strip_xml_tags(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for ch in s.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(ch),
            _ => {}
        }
    }
    out.trim().to_string()
}

fn extract_all_tags(text: &str, tag: &str) -> Vec<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let mut results = Vec::new();
    let mut pos = 0;
    while let Some(start) = text[pos..].find(&open) {
        let abs_start = pos + start + open.len();
        if let Some(end) = text[abs_start..].find(&close) {
            results.push(text[abs_start..abs_start + end].trim().to_string());
            pos = abs_start + end + close.len();
        } else {
            break;
        }
    }
    results
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
