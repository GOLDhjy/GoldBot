use anyhow::Result;
use crossterm::style::Stylize;

use crate::App;
use crate::agent::executor::sync_context_budget;
use crate::agent::provider::Message;
use crate::types::Event;
use crate::ui::format::format_event;
use crate::ui::screen::Screen;

use super::store::SessionStore;

impl SessionStore {
    /// Restore a historical session into the current app state.
    pub fn restore(&self, app: &mut App, screen: &mut Screen, id: &str) -> Result<()> {
        let content = self.read_session(id)?;
        self.apply_restored_content(app, screen, id, &content);
        Ok(())
    }

    fn apply_restored_content(&self, app: &mut App, screen: &mut Screen, id: &str, content: &str) {
        let ts = Self::format_session_timestamp(id);
        Self::switch_active(id);

        Self::restore_messages(app, &ts, content);
        self.reset_app_for_restore(app, screen);
        Self::reset_screen_for_restore(screen);
        screen.emit(&Self::render_restored_session_lines(&ts, content));
    }

    fn restore_messages(app: &mut App, ts: &str, content: &str) {
        app.messages.truncate(1);
        app.rebuild_system_message();
        app.messages.push(Message::user(format!(
            "[Restored session: {ts}]\nTreat the following session log as the active conversation context.\n\n{content}"
        )));
    }

    fn reset_app_for_restore(&self, app: &mut App, screen: &mut Screen) {
        app.task.clear();
        app.steps_taken = 0;
        app.running = false;
        app.llm_calling = false;
        app.llm_call_started_at = None;
        app.task_started_at = None;
        app.last_task_elapsed = None;
        app.needs_agent_executor = false;
        app.interrupt_llm_loop_requested = false;
        app.interjection_mode = false;
        app.pending_confirm = None;
        app.pending_confirm_note = false;
        app.current_phase = None;
        app.task_events.clear();
        app.final_summary = None;
        app.task_collapsed = false;
        app.pending_question = None;
        app.answering_question = false;
        app.pending_api_key_name = None;
        app.paste_counter = 0;
        app.paste_chunks.clear();
        app.task_display_override = None;
        app.todo_items.clear();
        app.shell_task_running = false;
        app.shell_exec_rx = None;
        app.dag_task_running = false;
        app.dag_result_rx = None;
        app.dag_progress_rx = None;
        app.dag_tree_event_idx = None;
        app.dag_node_done.clear();
        app.dag_graph_nodes.clear();
        app.dag_output_nodes.clear();
        app.total_usage = Default::default();
        app.at_file = Default::default();
        app.cmd_picker = Default::default();
        app.model_picker = Default::default();
        app.pending_session_list = None;
        app.clear_message_queue(screen);
        sync_context_budget(app, screen);
    }

    fn reset_screen_for_restore(screen: &mut Screen) {
        screen.question_labels.clear();
        screen.confirm_selected = None;
        screen.todo_items.clear();
        screen.dag_tree = None;
        screen.at_file_labels.clear();
        screen.at_file_sel = 0;
        screen.command_labels.clear();
        screen.command_sel = 0;
        screen.model_picker_labels.clear();
        screen.model_picker_sel = 0;
        screen.status.clear();
        screen.status_right.clear();
        screen.input.clear();
        screen.input_cursor = 0;
        screen.input_focused = true;
        screen.reset_task_lines();
        screen.clear_screen();
    }

    pub(crate) fn render_restored_session_lines(ts: &str, content: &str) -> Vec<String> {
        let mut lines = vec![
            format!("  ✓ 已切换到会话：{ts}").green().to_string(),
            String::new(),
        ];
        lines.extend(Self::parse_restored_session_content(content));
        lines
    }

    pub(crate) fn parse_restored_session_content(content: &str) -> Vec<String> {
        let lines: Vec<&str> = content.lines().collect();
        let mut out = Vec::new();
        let mut idx = 0;

        while idx < lines.len() {
            let line = lines[idx].trim_end();
            if !line.starts_with("## ") {
                idx += 1;
                continue;
            }

            let heading = line.trim_start_matches("## ").trim().to_string();
            idx += 1;

            if heading.contains("[diff]") {
                out.extend(parse_restored_diff_section(&lines, &mut idx, &heading));
            } else {
                out.extend(parse_restored_task_section(&lines, &mut idx, &heading));
            }
        }

        if out.is_empty() {
            out.push("  (会话内容为空)".dark_grey().to_string());
        }

        out
    }
}

fn parse_restored_task_section(lines: &[&str], idx: &mut usize, heading: &str) -> Vec<String> {
    let mut task = None;
    let mut final_text = None;

    while *idx < lines.len() && !lines[*idx].starts_with("## ") {
        let trimmed = lines[*idx].trim();
        if trimmed == "- **Task**" {
            *idx += 1;
            task = read_fenced_block(lines, idx);
            continue;
        }
        if trimmed == "- **Final**" {
            *idx += 1;
            final_text = read_fenced_block(lines, idx);
            continue;
        }
        *idx += 1;
    }

    let mut out = Vec::new();
    out.push(format!("  {}", heading).dark_grey().to_string());
    if let Some(task) = task.filter(|t| !t.trim().is_empty()) {
        out.extend(format_event(&Event::UserTask { text: task }));
    }
    if let Some(final_text) = final_text.filter(|t| !t.trim().is_empty()) {
        out.push(String::new());
        out.extend(format_event(&Event::Final {
            summary: final_text,
        }));
    }
    out.push(String::new());
    out
}

fn parse_restored_diff_section(lines: &[&str], idx: &mut usize, heading: &str) -> Vec<String> {
    let mut command = String::new();
    let mut files: Vec<(String, String)> = Vec::new();

    while *idx < lines.len() && !lines[*idx].starts_with("## ") {
        let trimmed = lines[*idx].trim();
        if let Some(rest) = trimmed.strip_prefix("- **Command**:") {
            command = rest.trim().trim_matches('`').to_string();
            *idx += 1;
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("- **File**:") {
            let label = rest.trim().to_string();
            *idx += 1;
            if let Some(diff) = read_fenced_block(lines, idx).filter(|d| !d.trim().is_empty()) {
                files.push((label, diff));
            }
            continue;
        }
        *idx += 1;
    }

    let mut out = vec![format!("  {}", heading).dark_grey().to_string()];
    if !command.is_empty() {
        out.extend(format_event(&Event::ToolCall {
            label: format!("Diff({command})"),
            command: command.clone(),
            multiline: false,
        }));
    }
    for (label, diff) in files {
        out.extend(format_event(&Event::ToolResult {
            exit_code: 0,
            output: format!("Diff {label}:\n{diff}"),
        }));
    }
    out.push(String::new());
    out
}

fn read_fenced_block(lines: &[&str], idx: &mut usize) -> Option<String> {
    while *idx < lines.len() && lines[*idx].trim().is_empty() {
        *idx += 1;
    }
    if *idx >= lines.len() || !lines[*idx].trim_start().starts_with("```") {
        return None;
    }

    *idx += 1;
    let mut block = Vec::new();
    while *idx < lines.len() {
        let line = lines[*idx];
        if line.trim_start().starts_with("```") {
            *idx += 1;
            break;
        }
        block.push(line);
        *idx += 1;
    }
    Some(block.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::SessionStore;

    #[test]
    fn restored_session_renders_task_and_final_blocks() {
        let content = "\
# Session 2026-04-01  11:15:14

## 11:15:14
- **Task**

```text
你好
```
- **Final**

```text
你好！我是 GoldBot。
```";

        let rendered = SessionStore::parse_restored_session_content(content).join("\n");
        assert!(rendered.contains("11:15:14"));
        assert!(rendered.contains("你好"));
        assert!(rendered.contains("GoldBot"));
    }

    #[test]
    fn restored_session_renders_diff_blocks() {
        let content = "\
# Session 2026-04-01  11:20:09

## 11:20:09 [diff]
- **Command**: `src/ui/screen.rs`

- **File**: src/ui/screen.rs

```diff
@@ -1 +1 @@
-old
+new
```";

        let rendered = SessionStore::parse_restored_session_content(content).join("\n");
        assert!(rendered.contains("11:20:09 [diff]"));
        assert!(rendered.contains("Diff(src/ui/screen.rs)"));
        assert!(rendered.contains("Diff src/ui/screen.rs:"));
        assert!(rendered.contains("-old"));
        assert!(rendered.contains("+new"));
    }
}
