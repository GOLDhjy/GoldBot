use anyhow::{Result, anyhow};
use serde_json::Value;

use crate::types::{LlmAction, TodoItem, TodoStatus};

pub(crate) const PLAN_MODE_ASSIST_CONTEXT_APPENDIX: &str = "\
Plan mode is enabled. Use the plan workflow proactively.
## Plan Mode Rules
- If information is missing or the task is ambiguous, use <tool>question</tool> first (ask one key question at a time).
- Once information is sufficient and the task is complex, use <tool>plan</tool> to produce a complete plan with todo-style step breakdowns.
- After <tool>plan</tool>, immediately ask for confirmation with <tool>question</tool>.
- If the user confirms the plan content only (not execution), reply in <final> with the full plan content (do not summarize or rewrite it).
- If the user confirms execution, your NEXT response MUST include <tool>set_mode</tool> before any execution tool call or <final>.
- Default execution mode after confirmation is <mode>agent</mode> unless the user explicitly requests another mode.
- <tool>set_mode</tool> is non-blocking and only updates local mode/UI.

## Plan Tools

<thought>reasoning</thought>
<tool>set_mode</tool>
<mode>agent</mode>
`<mode>` 可选值：`agent` / `accept_edits` / `plan`


Plan 输出示例：
<thought>reasoning</thought>
<tool>plan</tool>
<plan>## 计划
1. 第一步：...
2. 第二步：...
</plan>

Question tool (use when clarification or a user choice is required; try to gather info yourself first):
- Provide 3 preset options + 1 custom input option.
- Put the question in `<question>` and each choice in an `<option>`.
<thought>reasoning</thought>
<tool>question</tool>
<question>question text</question>
<option>Option A</option>
<option>Option B</option>
<option>Option C</option>
<option><user_input></option>

在执行的时候把plan中的todo步骤拆分,使用todo tool展示任务进度,格式如下:
Todo progress panel (shows a live checklist in the terminal):
<thought>reasoning</thought>
<tool>todo</tool>
<todo>[{\"label\":\"Analyze code\",\"status\":\"done\"},{\"label\":\"Write tests\",\"status\":\"running\"},{\"label\":\"Run CI\",\"status\":\"pending\"}]</todo>
Todo rules (CRITICAL — you MUST follow these):
- status must be one of: pending / running / done
- todo is NON-BLOCKING: you can include <tool>todo</tool> alongside another tool call in the same response. It does NOT count toward the tool limit.
- For ANY task requiring 2 or more steps, you MUST emit a <tool>todo</tool> in your FIRST response, listing all planned steps (first step = running, rest = pending).
- YOU are responsible for advancing todo progress. After a tool returns its result, you MUST emit an updated <tool>todo</tool> that: (1) marks the completed step as done, (2) marks the next step as running, and (3) keeps future steps as pending.
- Before <final>, emit a final <tool>todo</tool> with ALL items set to done.
- Never skip todo updates between steps. Every response that contains a tool call MUST also contain an updated <tool>todo</tool>.
Example of a 3-step task progression:
  Response 1: [{\"label\":\"Read file\",\"status\":\"running\"},{\"label\":\"Fix bug\",\"status\":\"pending\"},{\"label\":\"Test\",\"status\":\"pending\"}] + <tool>shell</tool>
  Response 2: [{\"label\":\"Read file\",\"status\":\"done\"},{\"label\":\"Fix bug\",\"status\":\"running\"},{\"label\":\"Test\",\"status\":\"pending\"}] + <tool>shell</tool>
  Response 3: [{\"label\":\"Read file\",\"status\":\"done\"},{\"label\":\"Fix bug\",\"status\":\"done\"},{\"label\":\"Test\",\"status\":\"running\"}] + <tool>shell</tool>
  Response 4: [{\"label\":\"Read file\",\"status\":\"done\"},{\"label\":\"Fix bug\",\"status\":\"done\"},{\"label\":\"Test\",\"status\":\"done\"}] + <final>";

pub(crate) fn parse_tool_action(tool: &str, text: &str) -> Result<Option<LlmAction>> {
    let action = match tool {
        "plan" => {
            let content = extract_last_tag(text, "plan")
                .ok_or_else(|| anyhow!("missing <plan> for plan tool call"))?;
            Some(LlmAction::Plan {
                content: strip_xml_tags(&content),
            })
        }
        "question" => {
            let text_q = extract_last_tag(text, "question")
                .ok_or_else(|| anyhow!("missing <question> for question tool call"))?;
            let options: Vec<String> = extract_all_tags(text, "option")
                .into_iter()
                .map(|o| {
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
            Some(LlmAction::Question {
                text: strip_xml_tags(&text_q),
                options,
            })
        }
        "todo" => {
            let raw = extract_last_tag(text, "todo")
                .ok_or_else(|| anyhow!("missing <todo> for todo tool call"))?;
            Some(LlmAction::Todo {
                items: parse_todo_json(&raw)?,
            })
        }
        _ => None,
    };
    Ok(action)
}

pub(crate) fn is_plan_echo(actions: &[LlmAction]) -> bool {
    actions.iter().any(|a| matches!(a, LlmAction::Final { .. }))
        && !actions
            .iter()
            .any(|a| matches!(a, LlmAction::Question { .. }))
}

fn parse_todo_json(raw: &str) -> Result<Vec<TodoItem>> {
    let arr: Vec<Value> =
        serde_json::from_str(raw).map_err(|e| anyhow!("invalid <todo> JSON: {e}"))?;
    let mut items = Vec::with_capacity(arr.len());
    for val in arr {
        let label = val
            .get("label")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("todo item missing \"label\""))?
            .to_string();
        let status_str = val
            .get("status")
            .and_then(|v| v.as_str())
            .unwrap_or("pending");
        let status = match status_str {
            "done" => TodoStatus::Done,
            "running" => TodoStatus::Running,
            _ => TodoStatus::Pending,
        };
        items.push(TodoItem { label, status });
    }
    Ok(items)
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
        let start_idx = pos + start + open.len();
        if let Some(end_rel) = text[start_idx..].find(&close) {
            let end_idx = start_idx + end_rel;
            results.push(text[start_idx..end_idx].to_string());
            pos = end_idx + close.len();
        } else {
            break;
        }
    }
    results
}

fn extract_last_tag(text: &str, tag: &str) -> Option<String> {
    extract_all_tags(text, tag).into_iter().last()
}

#[cfg(test)]
mod tests {
    use super::{is_plan_echo, parse_tool_action};
    use crate::types::LlmAction;

    #[test]
    fn parse_question_maps_user_input_option() {
        let raw = "<tool>question</tool><question>Q</question><option>A</option><option><user_input></option>";
        let action = parse_tool_action("question", raw)
            .expect("parse ok")
            .expect("some action");
        match action {
            LlmAction::Question { options, .. } => {
                assert_eq!(options, vec!["A".to_string(), "<user_input>".to_string()]);
            }
            _ => panic!("expected question"),
        }
    }

    #[test]
    fn plan_echo_requires_final_without_question() {
        let a = vec![
            LlmAction::Plan {
                content: "x".into(),
            },
            LlmAction::Final {
                summary: "y".into(),
            },
        ];
        assert!(is_plan_echo(&a));
        let b = vec![
            LlmAction::Plan {
                content: "x".into(),
            },
            LlmAction::Question {
                text: "q".into(),
                options: vec!["a".into()],
            },
            LlmAction::Final {
                summary: "y".into(),
            },
        ];
        assert!(!is_plan_echo(&b));
    }
}
