use crate::types::LlmAction;
use anyhow::{Result, anyhow};
use serde_json::Value;

pub const SYSTEM_PROMPT: &str = "\
You are GoldBot, a local terminal automation agent.
You help users complete tasks by running shell commands step by step.

You have at least one tool available:
  shell — executes a shell command and returns its stdout + stderr

## Response format

To call the shell tool, respond with EXACTLY this structure (nothing else):
<thought>your reasoning about what to do next</thought>
<tool>shell</tool>
<command>the shell command to run</command>

To call an MCP tool (if MCP tools are listed), respond with EXACTLY:
<thought>your reasoning about what to do next</thought>
<tool>mcp_server_tool</tool>
<arguments>{\"key\":\"value\"}</arguments>

When the task is complete, respond with EXACTLY:
<thought>your reasoning</thought>
<final>the summary you want to show the user</final>

## Rules
- Output one command per response, then wait for the result before deciding the next step.
- Use <final> as soon as the task is done — do not run unnecessary extra commands.
- Keep <final> concise: summarize outcome only, do not repeat full tool logs already shown.
- <final> must be plain terminal text. Do NOT use Markdown headings, lists, tables, or fenced code blocks.
- Prefer read-only commands unless the task explicitly requires changes.
- If a command fails, diagnose from the output and try a different approach.
- If file writes fail because heredoc formatting/indentation is broken, it is a command construction issue; retry using printf or python -c to write the file content exactly.
- macOS / Linux shell (bash).";

/// Parse the raw text returned by the LLM into a thought + action pair.
pub fn parse_llm_response(text: &str) -> Result<(String, LlmAction)> {
    let thought = extract_last_tag(text, "thought").unwrap_or_default();

    if let Some(summary) = extract_last_tag(text, "final") {
        return Ok((thought, LlmAction::Final { summary }));
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
