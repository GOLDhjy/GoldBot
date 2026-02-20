use crate::types::LlmAction;
use anyhow::{Result, anyhow};

/// System prompt injected at the start of every conversation.
/// Defines the agent's identity, the available tools, and the exact response format.
pub const SYSTEM_PROMPT: &str = "\
You are GoldBot, a local terminal automation agent.
You help users complete tasks by running shell commands step by step.

You have one tool available:
  shell — executes a shell command and returns its stdout + stderr

## Response format

To call the shell tool, respond with EXACTLY this structure (nothing else):
<thought>your reasoning about what to do next</thought>
<tool>shell</tool>
<command>the shell command to run</command>

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

    if let Some(command) = extract_last_tag(text, "command") {
        return Ok((thought, LlmAction::Shell { command }));
    }

    Err(anyhow!(
        "cannot parse LLM response — expected <command>...</command> or <final>...</final>"
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
}
