use std::{
    fs,
    path::{Path, PathBuf},
    sync::RwLock,
};

use crate::memory::project::current_project_base;
use anyhow::Result;
use chrono::{Duration, Local};

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

// ── SessionStore ──────────────────────────────────────────────────────────────

/// Manages session file storage for a project.
///
/// Sessions are stored in `<base>/sessions/<id>.md`.
pub struct SessionStore {
    base: PathBuf,
}

impl SessionStore {
    /// Create a SessionStore for the current workspace.
    pub fn current() -> Self {
        Self::new(current_project_base())
    }

    /// Create a SessionStore for the given project base directory.
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

    fn sessions_dir(&self) -> PathBuf {
        self.base.join("sessions")
    }

    fn current_session_path(&self) -> PathBuf {
        self.sessions_dir()
            .join(format!("{}.md", Self::active_id()))
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
}

// ── File helpers ──────────────────────────────────────────────────────────────

fn ensure_session_header(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    if !path.exists() {
        let active_session_id = SessionStore::active_id();
        let ts = SessionStore::format_session_timestamp(&active_session_id);
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

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static NEXT_ID: AtomicU64 = AtomicU64::new(0);

    fn temp_store() -> (SessionStore, PathBuf) {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let base = std::env::temp_dir().join(format!("goldbot-session-test-{nanos}-{id}"));
        fs::create_dir_all(&base).unwrap();
        (SessionStore::new(base.clone()), base)
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
        let ts = SessionStore::format_session_timestamp("20260331-142530");
        assert_eq!(ts, "2026-03-31  14:25:30");
    }

    #[test]
    fn format_session_timestamp_falls_back_for_bad_input() {
        let ts = SessionStore::format_session_timestamp("bad");
        assert_eq!(ts, "bad");
    }
}
