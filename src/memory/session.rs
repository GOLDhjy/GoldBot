use std::{
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
    sync::RwLock,
};

use anyhow::Result;
use chrono::{Duration, Local};
use crossterm::style::Stylize;

use crate::App;
use crate::agent::executor::sync_context_budget;
use crate::agent::provider::Message;
use crate::types::Event;
use crate::ui::format::format_event;
use crate::ui::screen::Screen;

use super::project::current_project_base;

// ── Constants ─────────────────────────────────────────────────────────────────

/// Maximum characters for session final output.
pub const MAX_SESSION_FINAL_CHARS: usize = 4000;
/// Session files older than this are deleted at startup.
pub const SESSION_RETENTION_DAYS: i64 = 15;

// ── Process-level session ID ──────────────────────────────────────────────────

/// Active session identifier for the current process context (YYYYMMDD-HHMMSS).
static SESSION_ID: std::sync::OnceLock<RwLock<String>> = std::sync::OnceLock::new();

fn default_session_id() -> String {
    Local::now().format("%Y%m%d-%H%M%S").to_string()
}

fn session_id_cell() -> &'static RwLock<String> {
    SESSION_ID.get_or_init(|| RwLock::new(default_session_id()))
}

fn active_session_id() -> String {
    session_id_cell()
        .read()
        .expect("session id lock poisoned")
        .clone()
}

fn switch_active_session(id: &str) {
    *session_id_cell().write().expect("session id lock poisoned") = id.to_string();
}

// ── Session ─────────────────────────────────────────────────────────────────

/// Manages session file storage for a project.
///
/// Sessions are stored in `<base>/sessions/<id>.md`.
pub struct Session {
    base: PathBuf,
}

impl Session {
    /// Create a Session for the current workspace.
    pub fn current() -> Self {
        Self::new(current_project_base())
    }

    /// Create a Session for the given project base directory.
    pub fn new(base: PathBuf) -> Self {
        Self { base }
    }

    /// Get the active session ID (YYYYMMDD-HHMMSS format).
    pub fn active_id() -> String {
        active_session_id()
    }

    /// Switch to a different active session ID.
    pub fn switch_active(id: &str) {
        switch_active_session(id);
    }

    /// Delete the current session file and rotate to a new empty active session.
    pub fn clear_current_session(&self) -> Result<String> {
        let old_id = Self::active_id();
        let old_path = self.sessions_dir().join(format!("{old_id}.md"));
        match fs::remove_file(&old_path) {
            Ok(()) => {}
            Err(err) if err.kind() == ErrorKind::NotFound => {}
            Err(err) => return Err(err.into()),
        }

        let new_id = self.fresh_session_id();
        Self::switch_active(&new_id);
        ensure_session_header(&self.current_session_path())?;
        Ok(new_id)
    }

    fn sessions_dir(&self) -> PathBuf {
        self.base.join("sessions")
    }

    fn current_session_path(&self) -> PathBuf {
        self.sessions_dir()
            .join(format!("{}.md", Self::active_id()))
    }

    fn fresh_session_id(&self) -> String {
        let base = default_session_id();
        let current = Self::active_id();
        let mut candidate = base.clone();
        let mut suffix = 1usize;
        while candidate == current || self.sessions_dir().join(format!("{candidate}.md")).exists() {
            candidate = format!("{base}-{suffix}");
            suffix += 1;
        }
        candidate
    }

    /// Append a completed-task record to the current session file.
    pub fn append_to_session(&self, task: &str, output: &str) -> Result<()> {
        let path = self.current_session_path();
        ensure_session_header(&path)?;
        let now = Local::now().format("%H:%M:%S");
        let task = sanitize_fenced(task.trim());
        let output = truncate_chars(output.trim(), MAX_SESSION_FINAL_CHARS);
        let output = sanitize_fenced(&output);
        let block = format!(
            "\n## {now}\n- **Task**\n\n```text\n{task}\n```\n\
             - **Final**\n\n```text\n{output}\n```\n"
        );
        append_file(path, &block)
    }

    /// Overwrite the current session file with the compaction summary.
    ///
    /// After context compaction the old session history is no longer accurate.
    /// Overwriting ensures that a session restore injects only the compact
    /// summary rather than the full discarded history.
    pub fn rewrite_session_after_compaction(
        &self,
        summary: &str,
        messages_dropped: usize,
    ) -> Result<()> {
        let path = self.current_session_path();
        let active_session_id = Self::active_id();
        let ts = Self::format_session_timestamp(&active_session_id);
        let now = Local::now().format("%H:%M:%S");
        let content = format!(
            "# Session {ts}\n\n\
             ## {now} [context compacted · {messages_dropped} messages dropped]\n\n\
             {}\n",
            summary.trim()
        );
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&path, content)?;
        Ok(())
    }

    /// Append file diffs produced by a shell command to the current session file.
    pub fn append_diff_to_session(&self, cmd: &str, diffs: &[(String, String)]) -> Result<()> {
        if diffs.is_empty() {
            return Ok(());
        }
        let path = self.current_session_path();
        ensure_session_header(&path)?;
        let now = Local::now().format("%H:%M:%S");
        let cmd = sanitize_fenced(cmd.trim());
        let mut block = format!("\n## {now} [diff]\n- **Command**: `{cmd}`\n");
        for (label, diff) in diffs {
            block.push_str(&format!("\n- **File**: {label}\n\n```diff\n{diff}\n```\n"));
        }
        append_file(path, &block)
    }

    /// Return session IDs sorted oldest-first (file name without `.md`).
    pub fn list_sessions(&self) -> Vec<String> {
        let Ok(entries) = fs::read_dir(self.sessions_dir()) else {
            return Vec::new();
        };
        let mut ids: Vec<String> = entries
            .flatten()
            .filter_map(|e| {
                let name = e.file_name().to_string_lossy().into_owned();
                name.strip_suffix(".md").map(str::to_string)
            })
            .collect();
        ids.sort();
        ids
    }

    /// Read the raw markdown content of a past session.
    pub fn read_session(&self, id: &str) -> Result<String> {
        let path = self.sessions_dir().join(format!("{id}.md"));
        fs::read_to_string(&path).map_err(Into::into)
    }

    /// Format a session ID (YYYYMMDD-HHMMSS) as a human-readable timestamp.
    pub fn format_session_timestamp(id: &str) -> String {
        // Expected format: 20260331-142530
        if id.len() >= 15 {
            let d = &id[..8];
            let t = &id[9..15];
            if d.chars().all(|c| c.is_ascii_digit()) && t.chars().all(|c| c.is_ascii_digit()) {
                return format!(
                    "{}-{}-{}  {}:{}:{}",
                    &d[..4],
                    &d[4..6],
                    &d[6..8],
                    &t[..2],
                    &t[2..4],
                    &t[4..6]
                );
            }
        }
        id.to_string()
    }

    /// Remove session files older than SESSION_RETENTION_DAYS. Safe to call at startup.
    pub fn cleanup_old_sessions(&self) {
        let dir = self.sessions_dir();
        let cutoff = (Local::now() - Duration::days(SESSION_RETENTION_DAYS)).timestamp() as u64;
        let Ok(entries) = fs::read_dir(&dir) else {
            return;
        };
        for entry in entries.flatten() {
            if !entry.file_name().to_string_lossy().ends_with(".md") {
                continue;
            }
            if let Ok(meta) = entry.path().metadata() {
                if let Ok(modified) = meta.modified() {
                    if let Ok(dur) = modified.duration_since(std::time::UNIX_EPOCH) {
                        if dur.as_secs() < cutoff {
                            let _ = fs::remove_file(entry.path());
                        }
                    }
                }
            }
        }
    }

    // ── Restore ───────────────────────────────────────────────────────────────

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
        app.current_phase_summary = None;
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

// ── File helpers ──────────────────────────────────────────────────────────────

fn ensure_session_header(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    if !path.exists() {
        let active_session_id = Session::active_id();
        let ts = Session::format_session_timestamp(&active_session_id);
        fs::write(path, format!("# Session {ts}\n"))?;
    }
    Ok(())
}

fn append_file(path: PathBuf, content: &str) -> Result<()> {
    use std::io::Write;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    file.write_all(content.as_bytes())?;
    Ok(())
}

fn sanitize_fenced(text: &str) -> String {
    text.replace("```", "``\\`")
}

fn truncate_chars(text: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    let count = text.chars().count();
    if count <= max {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
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

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Mutex, OnceLock,
        atomic::{AtomicU64, Ordering},
    };
    use std::time::{SystemTime, UNIX_EPOCH};

    static NEXT_ID: AtomicU64 = AtomicU64::new(0);
    static SESSION_TEST_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

    fn temp_store() -> (Session, PathBuf) {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let base = std::env::temp_dir().join(format!("goldbot-session-test-{nanos}-{id}"));
        fs::create_dir_all(&base).unwrap();
        (Session::new(base.clone()), base)
    }

    #[test]
    fn list_sessions_sorted_oldest_first() {
        let (store, base) = temp_store();
        let sessions_dir = base.join("sessions");
        fs::create_dir_all(&sessions_dir).unwrap();
        fs::write(sessions_dir.join("20260330-100000.md"), "# Session").unwrap();
        fs::write(sessions_dir.join("20260331-090000.md"), "# Session").unwrap();
        let list = store.list_sessions();
        assert_eq!(list, vec!["20260330-100000", "20260331-090000"]);
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn format_session_timestamp_parses_correctly() {
        let ts = Session::format_session_timestamp("20260331-142530");
        assert_eq!(ts, "2026-03-31  14:25:30");
    }

    #[test]
    fn format_session_timestamp_falls_back_for_bad_input() {
        let ts = Session::format_session_timestamp("bad");
        assert_eq!(ts, "bad");
    }

    #[test]
    fn clear_current_session_removes_old_file_and_rotates_active_id() {
        let _guard = SESSION_TEST_LOCK
            .get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap();
        let old_active = Session::active_id();
        let (store, base) = temp_store();
        let old_id = "20260402-101010";
        Session::switch_active(old_id);

        let old_path = base.join("sessions").join(format!("{old_id}.md"));
        ensure_session_header(&old_path).unwrap();
        fs::write(&old_path, "# Session old\n\nstale").unwrap();

        let new_id = store.clear_current_session().unwrap();
        let new_path = base.join("sessions").join(format!("{new_id}.md"));

        assert_ne!(new_id, old_id);
        assert!(!old_path.exists());
        assert_eq!(Session::active_id(), new_id);
        assert!(new_path.exists());
        assert!(
            fs::read_to_string(&new_path)
                .unwrap()
                .starts_with("# Session ")
        );

        Session::switch_active(&old_active);
        let _ = fs::remove_dir_all(base);
    }

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

        let rendered = Session::parse_restored_session_content(content).join("\n");
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

        let rendered = Session::parse_restored_session_content(content).join("\n");
        assert!(rendered.contains("11:20:09 [diff]"));
        assert!(rendered.contains("Diff(src/ui/screen.rs)"));
        assert!(rendered.contains("Diff src/ui/screen.rs:"));
        assert!(rendered.contains("-old"));
        assert!(rendered.contains("+new"));
    }
}
