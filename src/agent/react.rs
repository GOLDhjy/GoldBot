use crate::{
    agent::{
        plan,
        sub_agent::{InputMerge, NodeId, OutputMerge, TaskGraph, TaskNode},
    },
    types::{AssistMode, LlmAction},
};
use anyhow::{Result, anyhow};
use serde_json::Value;

const SYSTEM_PROMPT_TEMPLATE: &str = "\
You are GoldBot, a terminal automation agent. Complete tasks step by step using the tools below, Think before Act.

# Response format

## Rules
- When asked to plan implementation, use <tool>set_mode</tool> with <mode>plan</mode>, then follow the 'plan mode' flow.
- One blocking tool call per response. <tool>phase</tool> are non-blocking and may be included too. Use <tool>explorer</tool> to batch read-only lookups with multiple <command> tags.
- Use <tool>phase</tool> to write what to do next (one short sentence). Update it when the stage changes; omit it if unchanged.
- <final> is rendered in the terminal: headings (#/##), lists (-/*), inline **bold**/`code`, and diffs are all supported. Use them for clarity,Start with the conclusion.
- Use <final> as soon as done; avoid extra commands.
- The current phase is shown in the running UI and fed back with later tool results, so you must maintain it yourself when the task enters a new stage.
- On failure, diagnose from output and try a different approach.
- Shell: {SHELL_HINT}.

## Tools

### completed task
Task complete (required):
<thought>reasoning</thought>
<final>summary</final>
<final> guidelines:
- Start with the conclusion, then add brief details.
- If files were changed, include the file paths.
- No Emoji

### Process Tools

<thought>reasoning</thought>
<tool>set_mode</tool>
<mode>agent</mode>
`<mode>` 可选值：`agent` / `plan`

Explorer (batch read-only commands; all results returned at once — put everything into ONE call, never repeat):
prefer native tool: read, search, write/update than explorer.
<thought>reasoning</thought>
<tool>explorer</tool>
<command>first read-only command</command>
<command>more read-only command</command>

Phase update (non-blocking; write what to do next):
<thought>reasoning</thought>
<tool>phase</tool>
<phase>what to do next (one short sentence)</phase>

Update file (replace lines by line number; always use <tool>read</tool> first to get line numbers):
<thought>reasoning</thought>
<tool>update</tool>
<path>relative/or/absolute/path</path>
<line_start>first line to replace, 1-indexed</line_start>
<line_end>last line to replace, 1-indexed, inclusive</line_end>
<new_string>replacement content (empty = delete those lines)</new_string>

Write file (create a new file; also overwrites existing):
<thought>reasoning</thought>
<tool>write</tool>
<path>relative/or/absolute/path</path>
<content>full file content here</content>

Read file (each line prefixed with its line number `N: content`; use these numbers with <tool>update</tool>):
<thought>reasoning</thought>
<tool>read</tool>
<path>relative/or/absolute/path</path>
<offset>start line, 1-indexed (optional)</offset>
<limit>number of lines to read (optional)</limit>

Search files (regex search across file contents; native, cross-platform):
<thought>reasoning</thought>
<tool>search</tool>
<pattern>regex or literal string</pattern>
<path>optional/path/to/search (default: .)</path>

Shell command: prefer native tool: read, search, write/update.
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
";

/// Build the base system prompt with the actual MCP config file path substituted in.
pub fn build_system_prompt() -> String {
    let mcp_path = crate::tools::mcp::mcp_servers_file_path();
    let skills_dir = crate::tools::skills::goldbot_skills_dir();
    let shell_hint = if cfg!(target_os = "windows") {
        "PowerShell (Windows). Use PowerShell syntax only Or Execute like this: powershell -Command .../
        For file encoding issues, try adding `| Out-File -Encoding utf8` at the end of your command to ensure UTF-8 output."
    } else {
        "bash (macOS/Linux)"
    };
    SYSTEM_PROMPT_TEMPLATE
        .replace("{MCP_CONFIG_PATH}", &mcp_path.to_string_lossy())
        .replace("{SKILLS_DIR}", &skills_dir.to_string_lossy())
        .replace("{SHELL_HINT}", shell_hint)
}

/// Build the user-role wrapper message used when the user interrupts the loop and interjects
/// mid-task.  Keeping the wording here makes LLM-facing prompts easier to review in one place.
pub fn build_interjection_user_message(task: &str) -> String {
    format!(
        "User interrupted the current LLM loop and is interjecting mid-task.\n\
         Continue from the current conversation context.\n\
         \n\
         User interjection:\n{task}"
    )
}

/// Build the fixed assistant-role context message injected right after the system prompt.
/// This message is always present and never removed during context compaction.
pub fn build_assistant_context(workspace: &std::path::Path, assist_mode: AssistMode) -> String {
    let memory_dir = crate::memory::store::MemoryStore::new().base_dir_display();
    let workspace_display = workspace.display();
    let mut out = format!(
        "Current workspace: `{workspace_display}`\n\
         All shell commands run in this directory, and file paths are resolved relative to it.\n\
         \n\
         I can access an internal memory system at `{memory_dir}`:\n\
         - Long-term memory: `{memory_dir}/MEMORY.md`\n\
         - Short-term memory: `{memory_dir}/memory/YYYY-MM-DD.md` (daily logs)\n\
         \n\
         Every file change I make is automatically recorded as a diff in today's short-term \
         memory. If a file must be restored, I can read that diff and reverse it: lines \
         starting with `NNN -` were removed, and lines starting with `NNN +` were added.\n\
         \n\
         Memory rules:\n\
         - Do not mention memory files or paths unless the user explicitly asks.\n\
         - If information comes from memory, answer naturally (e.g., \"I remember ...\") \
         without saying you read a memory file.\n\
         - When asked about past events, preferences, or prior agreements, check memory first \
         using case-insensitive search (prefer `rg -n -i`)."
    );
    append_workspace_agents_md(&mut out, workspace);
    if assist_mode == AssistMode::Plan {
        out.push_str("\n\n");
        out.push_str(plan::PLAN_MODE_ASSIST_CONTEXT_APPENDIX);
    }
    out
}

fn append_workspace_agents_md(out: &mut String, workspace: &std::path::Path) {
    let Some(path) = find_nearest_agents_md(workspace) else {
        return;
    };
    let Ok(content) = std::fs::read_to_string(&path) else {
        return;
    };
    let content = content.trim();
    if content.is_empty() {
        return;
    }

    out.push_str("\n\n");
    out.push_str("Workspace-specific instructions are defined in a local `AGENTS.md` file.\n");
    out.push_str(&format!("AGENTS path: `{}`\n", path.display()));
    out.push_str(
        "Below is the full file content. Follow these project-specific instructions before \
         acting on repository tasks.\n\n",
    );
    out.push_str("----- BEGIN AGENTS.md -----\n");
    out.push_str(content);
    out.push_str("\n----- END AGENTS.md -----");
}

fn find_nearest_agents_md(workspace: &std::path::Path) -> Option<std::path::PathBuf> {
    let git_root = nearest_git_root(workspace);
    let mut dir = workspace;
    loop {
        let candidate = dir.join("AGENTS.md");
        if candidate.is_file() {
            return Some(candidate);
        }
        if git_root.as_deref() == Some(dir) {
            return None;
        }
        let Some(parent) = dir.parent() else {
            return None;
        };
        dir = parent;
    }
}

fn nearest_git_root(start: &std::path::Path) -> Option<std::path::PathBuf> {
    let mut dir = Some(start);
    while let Some(p) = dir {
        if p.join(".git").exists() {
            return Some(p.to_path_buf());
        }
        dir = p.parent();
    }
    None
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
        return Ok((thought, vec![LlmAction::Shell { command }]));
    }

    Err(anyhow!(
        "cannot parse LLM response — expected shell call, MCP call, or <final>...</final>"
    ))
}

fn parse_tool_action(text: &str, tool: &str) -> Result<LlmAction> {
    if let Some(action) = plan::parse_tool_action(tool, text)? {
        return Ok(action);
    }

    match tool {
        "shell" => {
            let command = extract_last_tag(text, "command")
                .ok_or_else(|| anyhow!("missing <command> for shell tool call"))?;
            Ok(LlmAction::Shell { command })
        }
        "phase" => {
            let text = extract_last_tag(text, "phase")
                .ok_or_else(|| anyhow!("missing <phase> for phase tool call"))?;
            Ok(LlmAction::Phase { text })
        }
        "update" => {
            let path = extract_last_tag(text, "path")
                .ok_or_else(|| anyhow!("missing <path> for update tool call"))?;
            let line_start = extract_last_tag(text, "line_start")
                .and_then(|s| s.trim().parse::<usize>().ok())
                .ok_or_else(|| anyhow!("missing or invalid <line_start> for update tool call"))?;
            let line_end = extract_last_tag(text, "line_end")
                .and_then(|s| s.trim().parse::<usize>().ok())
                .ok_or_else(|| anyhow!("missing or invalid <line_end> for update tool call"))?;
            let new_string =
                extract_last_tag_preserve_block(text, "new_string").unwrap_or_default();
            Ok(LlmAction::UpdateFile {
                path,
                line_start,
                line_end,
                new_string,
            })
        }
        "write" => {
            let path = extract_last_tag(text, "path")
                .ok_or_else(|| anyhow!("missing <path> for write tool call"))?;
            let content = extract_last_tag_preserve_block(text, "content").unwrap_or_default();
            Ok(LlmAction::WriteFile { path, content })
        }
        "read" => {
            let path = extract_last_tag(text, "path")
                .ok_or_else(|| anyhow!("missing <path> for read tool call"))?;
            let offset =
                extract_last_tag(text, "offset").and_then(|s| s.trim().parse::<usize>().ok());
            let limit =
                extract_last_tag(text, "limit").and_then(|s| s.trim().parse::<usize>().ok());
            Ok(LlmAction::ReadFile {
                path,
                offset,
                limit,
            })
        }
        "search" => {
            let pattern = extract_last_tag(text, "pattern")
                .ok_or_else(|| anyhow!("missing <pattern> for search tool call"))?;
            let path = extract_last_tag(text, "path").unwrap_or_else(|| ".".to_string());
            Ok(LlmAction::SearchFiles { pattern, path })
        }
        "web_search" => {
            let query = extract_last_tag(text, "query")
                .ok_or_else(|| anyhow!("missing <query> for web_search tool call"))?;
            Ok(LlmAction::WebSearch { query })
        }
        "set_mode" => {
            let raw_mode = extract_last_tag(text, "mode")
                .ok_or_else(|| anyhow!("missing <mode> for set_mode tool call"))?;
            let mode = AssistMode::parse_llm_name(&raw_mode)
                .ok_or_else(|| anyhow!("unsupported <mode> `{raw_mode}` for set_mode tool call"))?;
            Ok(LlmAction::SetMode { mode })
        }
        "explorer" => {
            let commands = extract_all_tags(text, "command");
            if commands.is_empty() {
                return Err(anyhow!("missing <command> for explorer tool call"));
            }
            Ok(LlmAction::Explorer { commands })
        }
        "sub_agent" => {
            let raw = extract_last_tag(text, "graph")
                .ok_or_else(|| anyhow!("missing <graph> for sub_agent tool call"))?;
            let obj: Value = serde_json::from_str(&raw)
                .map_err(|e| anyhow!("invalid <graph> JSON for sub_agent: {e}"))?;

            // Parse nodes array
            let raw_nodes = obj
                .get("nodes")
                .and_then(|v| v.as_array())
                .ok_or_else(|| anyhow!("<graph> missing \"nodes\" array"))?;
            if raw_nodes.is_empty() {
                return Err(anyhow!("<graph>.nodes is empty"));
            }
            let mut nodes: Vec<TaskNode> = Vec::with_capacity(raw_nodes.len());
            for n in raw_nodes {
                let id: NodeId = n
                    .get("id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow!("sub_agent node missing \"id\""))?
                    .to_string();
                let task = n
                    .get("task")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow!("sub_agent node \"{id}\" missing \"task\""))?
                    .to_string();
                let model = n.get("model").and_then(|v| v.as_str()).map(str::to_string);
                let role = n.get("role").and_then(|v| v.as_str()).map(str::to_string);
                let system_prompt = n
                    .get("system_prompt")
                    .and_then(|v| v.as_str())
                    .map(str::to_string);
                let depends_on: Vec<NodeId> = n
                    .get("depends_on")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str().map(str::to_string))
                            .collect()
                    })
                    .unwrap_or_default();
                let input_merge = n
                    .get("input_merge")
                    .and_then(|v| v.as_str())
                    .map(InputMerge::from_str)
                    .unwrap_or_default();
                nodes.push(TaskNode {
                    id,
                    task,
                    model,
                    role,
                    system_prompt,
                    depends_on,
                    input_merge,
                });
            }

            // Parse output_nodes (optional; defaults to all leaf nodes)
            let output_nodes: Vec<NodeId> = obj
                .get("output_nodes")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default();

            let output_merge = obj
                .get("output_merge")
                .and_then(|v| v.as_str())
                .map(OutputMerge::from_str)
                .unwrap_or_default();

            Ok(LlmAction::SubAgent {
                graph: TaskGraph {
                    nodes,
                    output_nodes,
                    output_merge,
                },
            })
        }
        t if t.starts_with("mcp_") => {
            let raw_args = extract_last_tag(text, "arguments")
                .or_else(|| extract_last_tag(text, "args"))
                .unwrap_or_else(|| "{}".to_string());
            let arguments: Value = serde_json::from_str(&raw_args)
                .map_err(|e| anyhow!("invalid <arguments> JSON for MCP tool call: {e}"))?;
            if !arguments.is_object() {
                return Err(anyhow!("MCP <arguments> must be a JSON object"));
            }
            Ok(LlmAction::Mcp {
                tool: t.to_string(),
                arguments,
            })
        }
        t => Err(anyhow!("unsupported tool `{t}`")),
    }
}

fn extract_all_tags(text: &str, tag: &str) -> Vec<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let mut results = Vec::new();
    let mut pos = 0;
    while let Some(start) = text[pos..].find(&open) {
        let abs_start = pos + start + open.len();
        let next_close = text[abs_start..].find(&close).map(|i| abs_start + i);
        let next_open = text[abs_start..].find(&open).map(|i| abs_start + i);
        match (next_close, next_open) {
            // Malformed current tag (missing close) and a later sibling tag starts first.
            // Skip this opening tag and resync at the next one instead of swallowing everything
            // until a later closing tag.
            (Some(close_pos), Some(open_pos)) if open_pos < close_pos => {
                pos = open_pos;
            }
            (Some(close_pos), _) => {
                results.push(text[abs_start..close_pos].trim().to_string());
                pos = close_pos + close.len();
            }
            (None, _) => break,
        }
    }
    results
}

fn extract_last_tag(text: &str, tag: &str) -> Option<String> {
    extract_last_tag_raw(text, tag).map(|s| s.trim().to_string())
}

fn extract_last_tag_raw(text: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let end = text.rfind(&close)?;
    let head = &text[..end];
    let start = head.rfind(&open)? + open.len();
    Some(head[start..].to_string())
}

/// Preserve leading indentation for code/file content blocks while still stripping
/// the common wrapper newline introduced by pretty-printed XML tags.
fn extract_last_tag_preserve_block(text: &str, tag: &str) -> Option<String> {
    let mut s = extract_last_tag_raw(text, tag)?;
    let had_leading_wrapper_newline = if s.starts_with("\r\n") {
        s.drain(..2);
        true
    } else if s.starts_with('\n') {
        s.drain(..1);
        true
    } else {
        false
    };

    if had_leading_wrapper_newline {
        if s.ends_with("\r\n") {
            s.truncate(s.len().saturating_sub(2));
        } else if s.ends_with('\n') {
            s.pop();
        }
    }

    Some(s)
}

#[cfg(test)]
mod tests {
    use super::{build_assistant_context, build_system_prompt, parse_llm_response};
    use crate::types::{AssistMode, LlmAction};
    use serde_json::json;
    use std::{
        fs,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    fn unique_temp_dir(label: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be monotonic enough for tests")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "goldbot-react-{label}-{}-{nanos}",
            std::process::id()
        ))
    }

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
    fn parse_mcp_tool_call_accepts_args_alias() {
        let raw = "<tool>mcp_context7_resolve_library</tool><args>{\"libraryName\":\"tokio\",\"query\":\"runtime\"}</args>";
        let (_, actions) =
            parse_llm_response(raw).expect("should parse MCP action with args alias");
        match &actions[0] {
            LlmAction::Mcp { arguments, .. } => {
                assert_eq!(*arguments, json!({"libraryName":"tokio","query":"runtime"}));
            }
            _ => panic!("expected MCP action"),
        }
    }

    #[test]
    fn parse_tools_recovers_after_unclosed_tool_tag() {
        let raw = "<tool>mcp_builtin_zread_read_file>\n<args>{\"file_path\":\"README.md\"}</args>\n\
                   <tool>read</tool><path>README.md</path>";
        let (_, actions) = parse_llm_response(raw).expect("should recover and parse later tool");
        assert_eq!(actions.len(), 1);
        match &actions[0] {
            LlmAction::ReadFile { path, .. } => assert_eq!(path, "README.md"),
            _ => panic!("expected ReadFile action"),
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
    fn parse_set_mode_tool_call() {
        let raw =
            "<thought>switch to execution mode</thought><tool>set_mode</tool><mode>agent</mode>";
        let (_, actions) = parse_llm_response(raw).expect("should parse set_mode");
        assert_eq!(actions.len(), 1);
        match &actions[0] {
            LlmAction::SetMode { mode } => assert_eq!(*mode, AssistMode::Off),
            _ => panic!("expected SetMode action"),
        }
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

    #[test]
    fn parse_phase_tool_call() {
        let raw = "<thought>switch stage</thought><tool>phase</tool><phase>我先收集上下文，确认当前实现。</phase>";
        let (_, actions) = parse_llm_response(raw).expect("should parse phase");
        assert_eq!(actions.len(), 1);
        match &actions[0] {
            LlmAction::Phase { text } => assert!(text.contains("收集上下文")),
            _ => panic!("expected Phase action"),
        }
    }

    #[test]
    fn parse_update_preserves_new_string_indentation() {
        let raw = "<tool>update</tool>\
            <path>src/main.rs</path>\
            <line_start>10</line_start>\
            <line_end>12</line_end>\
            <new_string>\n    if cond {\n        work();\n    }\n</new_string>";
        let (_, actions) = parse_llm_response(raw).expect("should parse update");
        match &actions[0] {
            LlmAction::UpdateFile { new_string, .. } => {
                assert!(new_string.starts_with("    if cond {"));
                assert!(new_string.contains("\n        work();\n"));
                assert!(!new_string.starts_with('\n'));
            }
            _ => panic!("expected UpdateFile action"),
        }
    }

    #[test]
    fn parse_write_preserves_content_indentation() {
        let raw = "<tool>write</tool>\
            <path>tmp.txt</path>\
            <content>\n  alpha\n    beta\n</content>";
        let (_, actions) = parse_llm_response(raw).expect("should parse write");
        match &actions[0] {
            LlmAction::WriteFile { content, .. } => {
                assert_eq!(content, "  alpha\n    beta");
            }
            _ => panic!("expected WriteFile action"),
        }
    }
    #[test]
    fn build_assistant_context_off_does_not_include_plan_rules() {
        let ctx = build_assistant_context(std::path::Path::new("."), AssistMode::Off);
        assert!(!ctx.contains("Plan decision order (simplified):"));
        assert!(!ctx.contains("<tool>plan</tool>"));
    }
    #[test]
    fn build_assistant_context_plan_includes_plan_rules() {
        let ctx = build_assistant_context(std::path::Path::new("."), AssistMode::Plan);
        assert!(ctx.contains("Plan decision order (simplified):"));
        assert!(ctx.contains("<tool>plan</tool>"));
        assert!(ctx.contains("Todo progress panel (shows a live checklist in the terminal):"));
        assert!(ctx.contains("<tool>todo</tool>"));
        assert!(ctx.contains("<plan>"));
        assert!(ctx.contains("<tool>question</tool>"));
    }
    #[test]
    fn build_system_prompt_does_not_include_old_plan_block() {
        let prompt = build_system_prompt();
        assert!(!prompt.contains("<tool>plan</tool>"));
        assert!(!prompt.contains("Plan decision order (simplified):"));
        assert!(!prompt.contains("Todo progress panel (shows a live checklist in the terminal):"));
    }

    #[test]
    fn build_assistant_context_includes_full_agents_runtime_prompt_ascii() {
        let root = unique_temp_dir("agents-full-ascii");
        fs::create_dir_all(&root).expect("should create workspace dir");
        fs::write(
            root.join("AGENTS.md"),
            "# Repo Rules\n\n## Read local knowledge base first\nRead ./.AIDB/README.md before searching the whole repo.\n\n```md\nexample block\n```\n",
        )
        .expect("should write AGENTS");

        let ctx = build_assistant_context(&root, AssistMode::Off);
        assert!(ctx.contains("----- BEGIN AGENTS.md -----"));
        assert!(ctx.contains("## Read local knowledge base first"));
        assert!(ctx.contains("Read ./.AIDB/README.md before searching the whole repo."));
        assert!(ctx.contains("```md"));

        let _ = fs::remove_dir_all(&root);
    }
}
