use crate::types::LlmAction;
use anyhow::{Result, anyhow};
use serde_json::Value;

const SYSTEM_PROMPT_TEMPLATE: &str = "\
You are GoldBot, a terminal automation agent. Complete tasks step by step using the tools below.

## Response format

重要：遵循以下决策顺序：
1. 任务简单且信息明确（如直接查询、执行单条命令）→ 直接执行，无需 plan
2. 任务信息不足或有歧义 → 可以用 question 工具提问（每次只问一个关键问题）,收集到足够信息;
3. 信息充足后，任务如果很复杂可用 plan 工具输出完整内容；plan 之后必须紧跟 question 询问用户是否确认
4. 用户确认计划后 → 将 plan 的完整内容原封不动地放入 final 输出（不得删减、不得改写为摘要）；若计划是行程/方案/文档类内容，final 必须逐条重复全部细节
5. 修改文件注意文件原本的编码格式，防止乱码。

### completed task
Task complete:完成任务后一定要发
<thought>reasoning</thought>
<final>outcome summary</final>
outcome summary:
- final，在最前面先说总结结论，然后在细分。
- 如果此次任务修改了文件等，最好把涉及的文件名（有链接那种）写在细分里面

## Rules
- One tool call per response; wait for the result before proceeding. Exceptions: <tool>todo</tool> is non-blocking and can always be included alongside another tool call. <tool>explorer</tool> accepts multiple <command> tags and is the preferred way to batch read-only lookups into a single round-trip.
- Use <final> as soon as done; avoid extra commands.
- <final> is rendered in the terminal: headings (#/##), lists (-/*), inline **bold**/`code`, and diffs are all supported. Use them for clarity.
- On failure, diagnose from output and try a different approach.
- Shell: {SHELL_HINT}.

## Tools

Plan 模式（信息充足时，输出具体可执行计划；plan 之后必须立即用 question 工具询问用户是否确认）：
<thought>reasoning</thought>
<tool>plan</tool>
<plan>## 计划
1. 第一步：...
2. 第二步：...
</plan>

向用户提问（需要澄清或让用户做选择时，你应该自己先尝试获取信息后再提问）：
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
<file>path/to/file</file>
<command>bash command</command>
The optional <file> tag names the primary file being created or modified (relative or absolute path). Omit it for read-only or multi-file commands. It is used to capture a before-diff.

Explorer (read-only, multiple commands in ONE single call):
<thought>reasoning</thought>
<tool>explorer</tool>
<command>first read-only command</command>
<command>second read-only command</command>
<command>third read-only command</command>
<command>more read-only command</command>
CRITICAL rules for <tool>explorer</tool>:
- You MUST include ALL read-only commands you need in a SINGLE <tool>explorer</tool> call. Never send explorer multiple times in a row.
- All <command> tags are collected, executed in sequence, and ALL results returned to you at once in one message.
- Only use for safe read-only commands (ls, cat, grep, git log, cargo check …).
- Do NOT use for commands that write, delete, or depend on each other's output.

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
  Response 4: [{\"label\":\"Read file\",\"status\":\"done\"},{\"label\":\"Fix bug\",\"status\":\"done\"},{\"label\":\"Test\",\"status\":\"done\"}] + <final>

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
Use shell-appropriate commands to write the file (bash: `printf`/`cat`; PowerShell: `Set-Content`/`Out-File`). Tell the user to restart GoldBot to load it.";

/// Build the base system prompt with the actual MCP config file path substituted in.
pub fn build_system_prompt() -> String {
    let mcp_path = crate::tools::mcp::mcp_servers_file_path();
    let skills_dir = crate::tools::skills::goldbot_skills_dir();
    let shell_hint = if cfg!(target_os = "windows") {
        "PowerShell (Windows). Use PowerShell syntax only Or Execute like this: powershell -Command ..."
    } else {
        "bash (macOS/Linux)"
    };
    SYSTEM_PROMPT_TEMPLATE
        .replace("{MCP_CONFIG_PATH}", &mcp_path.to_string_lossy())
        .replace("{SKILLS_DIR}", &skills_dir.to_string_lossy())
        .replace("{SHELL_HINT}", shell_hint)
}

/// Build the fixed assistant-role context message injected right after the system prompt.
/// This message is always present and never removed during context compaction.
pub fn build_assistant_context(workspace: &std::path::Path) -> String {
    let memory_dir = crate::memory::store::MemoryStore::new().base_dir_display();
    let workspace_display = workspace.display();
    format!(
        "当前工作空间（Workspace）：`{workspace_display}`\n\
         所有 shell 命令都在此目录下执行，文件路径也以此为基准。\n\
         \n\
         I have access to the memory system at `{memory_dir}`:\n\
         - Long-term memory: `{memory_dir}/MEMORY.md`\n\
         - Short-term memory: `{memory_dir}/memory/YYYY-MM-DD.md` (daily logs)\n\
         \n\
         Every file modification I make is automatically recorded as a diff in today's \
         short-term memory log. If a file needs to be restored, I can read that log and \
         reverse the changes: lines starting with `NNN -` were removed, lines starting \
         with `NNN +` were added.\n\
         注意：在回答中不要暴露你有记忆文件，除非主动问你记忆文件在哪，如果你是在看记忆文件得到的信息直接说 我记得 "
    )
}

/// Parse the raw text returned by the LLM into a thought and an ordered list of actions.
///
/// The LLM may legitimately emit multiple tool calls in one response (e.g. `plan` followed
/// immediately by `question`).  All `<tool>` tags are extracted in document order and each
/// one is parsed into an [`LlmAction`].  The caller is responsible for executing them in
/// sequence, stopping at the first "blocking" action (shell, question, final, …).
pub fn parse_llm_response(text: &str) -> Result<(String, Vec<LlmAction>)> {
    let thought = extract_last_tag(text, "thought").unwrap_or_default();

    // These special tags are never wrapped in <tool>; handle them first.
    if let Some(summary) = extract_last_tag(text, "final") {
        return Ok((thought, vec![LlmAction::Final { summary }]));
    }
    if let Some(name) = extract_last_tag(text, "skill") {
        return Ok((thought, vec![LlmAction::Skill { name }]));
    }
    if let Some(raw) = extract_last_tag(text, "create_mcp") {
        let config: Value =
            serde_json::from_str(&raw).map_err(|e| anyhow!("invalid <create_mcp> JSON: {e}"))?;
        if !config.is_object() {
            return Err(anyhow!("<create_mcp> must be a JSON object"));
        }
        return Ok((thought, vec![LlmAction::CreateMcp { config }]));
    }

    // Collect all <tool> tags in document order and parse each one.
    let tools = extract_all_tags(text, "tool");
    if !tools.is_empty() {
        let mut actions = Vec::with_capacity(tools.len());
        for tool in tools {
            actions.push(parse_tool_action(text, &tool)?);
        }
        return Ok((thought, actions));
    }

    // Backward compatibility: bare <command> without a wrapping <tool>.
    if let Some(command) = extract_last_tag(text, "command") {
        return Ok((thought, vec![LlmAction::Shell { command, file: None }]));
    }

    Err(anyhow!(
        "cannot parse LLM response — expected shell call, MCP call, or <final>...</final>"
    ))
}

fn parse_tool_action(text: &str, tool: &str) -> Result<LlmAction> {
    match tool {
        "shell" => {
            let command = extract_last_tag(text, "command")
                .ok_or_else(|| anyhow!("missing <command> for shell tool call"))?;
            let file = extract_last_tag(text, "file");
            Ok(LlmAction::Shell { command, file })
        }
        "web_search" => {
            let query = extract_last_tag(text, "query")
                .ok_or_else(|| anyhow!("missing <query> for web_search tool call"))?;
            Ok(LlmAction::WebSearch { query })
        }
        "plan" => {
            let content = extract_last_tag(text, "plan")
                .ok_or_else(|| anyhow!("missing <plan> for plan tool call"))?;
            Ok(LlmAction::Plan { content: strip_xml_tags(&content) })
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
            Ok(LlmAction::Question { text: strip_xml_tags(&text_q), options })
        }
        "explorer" => {
            let commands = extract_all_tags(text, "command");
            if commands.is_empty() {
                return Err(anyhow!("missing <command> for explorer tool call"));
            }
            Ok(LlmAction::Explorer { commands })
        }
        "todo" => {
            let raw = extract_last_tag(text, "todo")
                .ok_or_else(|| anyhow!("missing <todo> for todo tool call"))?;
            let items = parse_todo_json(&raw)?;
            Ok(LlmAction::Todo { items })
        }
        "diff" => {
            let content = extract_last_tag(text, "diff")
                .ok_or_else(|| anyhow!("missing <diff> for diff tool call"))?;
            Ok(LlmAction::Diff { content: strip_xml_tags(&content) })
        }
        t if t.starts_with("mcp_") => {
            let raw_args = extract_last_tag(text, "arguments").unwrap_or_else(|| "{}".to_string());
            let arguments: Value = serde_json::from_str(&raw_args)
                .map_err(|e| anyhow!("invalid <arguments> JSON for MCP tool call: {e}"))?;
            if !arguments.is_object() {
                return Err(anyhow!("MCP <arguments> must be a JSON object"));
            }
            Ok(LlmAction::Mcp { tool: t.to_string(), arguments })
        }
        t => Err(anyhow!("unsupported tool `{t}`")),
    }
}

fn parse_todo_json(raw: &str) -> Result<Vec<crate::types::TodoItem>> {
    use crate::types::{TodoItem, TodoStatus};
    let arr: Vec<Value> = serde_json::from_str(raw)
        .map_err(|e| anyhow!("invalid <todo> JSON: {e}"))?;
    let mut items = Vec::with_capacity(arr.len());
    for val in arr {
        let label = val.get("label")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("todo item missing \"label\""))?
            .to_string();
        let status_str = val.get("status")
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
        let (_, actions) = parse_llm_response(raw).expect("should parse final");
        assert_eq!(actions.len(), 1);
        match &actions[0] {
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
        let (_, actions) = parse_llm_response(raw).expect("should parse MCP action");
        assert_eq!(actions.len(), 1);
        match &actions[0] {
            LlmAction::Mcp { tool, arguments } => {
                assert_eq!(tool, "mcp_context7_resolve_library");
                assert_eq!(*arguments, json!({"libraryName":"tokio"}));
            }
            _ => panic!("expected MCP action"),
        }
    }

    #[test]
    fn parse_plan_combined_with_question_returns_both_in_order() {
        // LLM emits plan + question in one response; both must be parsed in document order.
        let raw = "<thought>plan first</thought>\
            <tool>plan</tool>\
            <plan>## 计划\n1. 第一步\n2. 第二步</plan>\
            <tool>question</tool>\
            <question>确认吗？</question>\
            <option>是</option><option>否</option><option><user_input></option>";
        let (_, actions) = parse_llm_response(raw).expect("should parse");
        assert_eq!(actions.len(), 2, "expected [Plan, Question]");
        match &actions[0] {
            LlmAction::Plan { content } => assert!(content.contains("第一步")),
            _ => panic!("first action should be Plan"),
        }
        match &actions[1] {
            LlmAction::Question { text, options } => {
                assert!(text.contains("确认"));
                assert_eq!(options.len(), 3);
            }
            _ => panic!("second action should be Question"),
        }
    }

    #[test]
    fn parse_mcp_arguments_requires_json_object() {
        let raw = "<tool>mcp_context7_lookup</tool><arguments>[1,2,3]</arguments>";
        let err = parse_llm_response(raw).expect_err("should fail");
        assert!(err.to_string().contains("JSON object"));
    }

    #[test]
    fn parse_todo_tool_call() {
        let raw = r#"<thought>set progress</thought><tool>todo</tool><todo>[{"label":"Analyze","status":"done"},{"label":"Build","status":"running"},{"label":"Test","status":"pending"}]</todo>"#;
        let (thought, actions) = parse_llm_response(raw).expect("should parse todo");
        assert_eq!(thought, "set progress");
        assert_eq!(actions.len(), 1);
        match &actions[0] {
            LlmAction::Todo { items } => {
                assert_eq!(items.len(), 3);
                assert_eq!(items[0].label, "Analyze");
                assert_eq!(items[0].status, crate::types::TodoStatus::Done);
                assert_eq!(items[1].status, crate::types::TodoStatus::Running);
                assert_eq!(items[2].status, crate::types::TodoStatus::Pending);
            }
            _ => panic!("expected Todo action"),
        }
    }
}
