use std::{fs, path::{Path, PathBuf}};

use anyhow::Result;
use chrono::{Duration, Local};

// ── Constants ─────────────────────────────────────────────────────────────────

const MAX_SESSION_FINAL_CHARS: usize = 4000;
const MAX_MEMORY_NOTE_CHARS: usize = 120;
const MEMORY_SECTION: &str = "## Memories";
const SESSION_RETENTION_DAYS: i64 = 15;
/// Maximum number of notes injected per LLM call.
const MEMORY_TOP_N: usize = 15;

// ── Process-level statics ─────────────────────────────────────────────────────

/// Workspace set once at startup; accessible throughout the process.
static CURRENT_WORKSPACE: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();

/// Session identifier — one per process lifetime (YYYYMMDD-HHMMSS).
static SESSION_ID: std::sync::OnceLock<String> = std::sync::OnceLock::new();

/// Call once at startup, before any ProjectStore operations.
pub fn init_workspace(workspace: PathBuf) {
    let _ = CURRENT_WORKSPACE.set(workspace);
}

pub fn session_id() -> &'static str {
    SESSION_ID.get_or_init(|| Local::now().format("%Y%m%d-%H%M%S").to_string())
}

// ── ProjectStore ──────────────────────────────────────────────────────────────

/// All persistent state for one project:
///   `<base>/MEMORY.md`            — long-term memory
///   `<base>/sessions/<id>.md`     — per-session logs
///
/// where `<base>` = `~/.goldbot/projects/<sanitized_workspace_path>`.
pub struct ProjectStore {
    base: PathBuf,
}

impl ProjectStore {
    /// Build a store for the globally initialised workspace.
    /// Panics if `init_workspace` was never called.
    pub fn current() -> Self {
        let ws = CURRENT_WORKSPACE
            .get()
            .expect("ProjectStore::init_workspace must be called before ProjectStore::current");
        Self::new(ws)
    }

    pub fn new(workspace: &Path) -> Self {
        Self { base: project_base(workspace) }
    }

    // ── Paths ─────────────────────────────────────────────────────────────────

    fn memory_path(&self) -> PathBuf {
        self.base.join("MEMORY.md")
    }

    fn sessions_dir(&self) -> PathBuf {
        self.base.join("sessions")
    }

    fn current_session_path(&self) -> PathBuf {
        self.sessions_dir().join(format!("{}.md", session_id()))
    }

    /// Path displayed to the LLM in the assistant context message.
    pub fn memory_path_display(&self) -> String {
        self.memory_path().to_string_lossy().into_owned()
    }

    // ── Long-term memory ──────────────────────────────────────────────────────

    /// Append a deduplicated note to MEMORY.md. Returns true when actually written.
    pub fn append_memory(&self, note: &str) -> Result<bool> {
        let path = self.memory_path();
        ensure_memory_file(&path)?;

        let note = normalize_note(note);
        if note.is_empty() {
            return Ok(false);
        }
        let canonical = canonicalize(&note);

        if let Ok(existing) = fs::read_to_string(&path) {
            if notes_from_file(&existing)
                .iter()
                .any(|n| canonicalize(n) == canonical)
            {
                return Ok(false);
            }
        }

        append_file(path, &format!("- {note}\n"))?;
        Ok(true)
    }

    /// Build the memory block injected into the LLM context.
    ///
    /// When `query` is provided and the total note count exceeds `MEMORY_TOP_N`,
    /// only the top-scoring notes (by keyword overlap with the query) are included.
    /// Ties are broken by recency (later entries win). When `query` is `None` or
    /// fewer notes exist than `MEMORY_TOP_N`, the most-recent notes are returned.
    pub fn build_memory_message(&self, query: Option<&str>) -> Option<String> {
        let content = fs::read_to_string(self.memory_path()).ok()?;
        let notes = notes_from_file(&content);
        if notes.is_empty() {
            return None;
        }

        let selected: Vec<&str> = if notes.len() <= MEMORY_TOP_N {
            notes.iter().map(String::as_str).collect()
        } else if let Some(q) = query.filter(|s| !s.trim().is_empty()) {
            let q_tokens = tokenize(q);
            let mut scored: Vec<(usize, usize, &str)> = notes
                .iter()
                .enumerate()
                .map(|(i, n)| (keyword_score(&q_tokens, n), i, n.as_str()))
                .collect();
            // Higher score first; break ties by higher index (more recent).
            scored.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)));
            scored.truncate(MEMORY_TOP_N);
            scored.into_iter().map(|(_, _, n)| n).collect()
        } else {
            // No query: return the most recent MEMORY_TOP_N notes.
            notes.iter().rev().take(MEMORY_TOP_N).map(String::as_str).collect()
        };

        let lines = selected
            .iter()
            .map(|n| format!("- {n}"))
            .collect::<Vec<_>>()
            .join("\n");
        Some(format!(
            "## Memory\nOn conflict, follow the latest user instruction.\n\n\
             ### Project Memory\n{lines}"
        ))
    }

    // ── Session (short-term) ──────────────────────────────────────────────────

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

    /// Append file diffs produced by a shell command to the current session file.
    pub fn append_diff_to_session(
        &self,
        cmd: &str,
        diffs: &[(String, String)],
    ) -> Result<()> {
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

    // ── Session listing / restore ─────────────────────────────────────────────

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

    // ── Maintenance ───────────────────────────────────────────────────────────

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

// ── Path helpers ──────────────────────────────────────────────────────────────

fn project_base(workspace: &Path) -> PathBuf {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    
    let name = workspace
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "unknown".to_string());
    
    let mut hasher = DefaultHasher::new();
    workspace.hash(&mut hasher);
    let hash = format!("{:x}", hasher.finish());
    
    let sanitized = format!("{}-{}", name, &hash[..8]);
    
    crate::memory::store::default_memory_base_dir()
        .join("memory")
        .join(sanitized)
}

// ── File helpers ──────────────────────────────────────────────────────────────

fn ensure_memory_file(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    if !path.exists() {
        fs::write(path, format!("# Project Memory\n\n{MEMORY_SECTION}\n"))?;
    }
    Ok(())
}

fn ensure_session_header(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    if !path.exists() {
        let ts = ProjectStore::format_session_timestamp(session_id());
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

/// Extract bullet-list notes from the `## Memories` section.
fn notes_from_file(raw: &str) -> Vec<String> {
    let mut in_section = false;
    let mut notes = Vec::new();
    for line in raw.lines() {
        let t = line.trim();
        if t.starts_with("## ") {
            in_section = t == MEMORY_SECTION;
            continue;
        }
        if !in_section {
            continue;
        }
        if let Some(note) = t.strip_prefix("- ") {
            let note = note.trim();
            if !note.is_empty() && note.chars().count() <= MAX_MEMORY_NOTE_CHARS + 40 {
                notes.push(note.to_string());
            }
        }
    }
    // Fallback for files without section header (plain bullet list).
    if notes.is_empty() {
        notes = raw
            .lines()
            .filter_map(|l| l.trim_start().strip_prefix("- ").map(str::trim))
            .filter(|n| !n.is_empty())
            .map(str::to_string)
            .collect();
    }
    notes
}

fn normalize_note(text: &str) -> String {
    let mut s = text
        .replace('`', "")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim_matches(|c: char| {
            c.is_whitespace() || matches!(c, '，' | ',' | '。' | '.' | ':' | '：')
        })
        .to_string();
    s = truncate_chars(&s, MAX_MEMORY_NOTE_CHARS);
    if s.is_empty() {
        return s;
    }
    if !s.ends_with(['。', '！', '？', '.', '!', '?', ';', '；']) {
        let end = if s.is_ascii() { "." } else { "。" };
        s.push_str(end);
    }
    s
}

fn canonicalize(text: &str) -> String {
    text.trim()
        .trim_end_matches(['。', '.', '!', '?', ';', '；'])
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// Tokenise `text` into a set of lowercase tokens for keyword-overlap scoring.
///
/// - CJK characters are treated as individual tokens.
/// - Latin/digit runs of 2+ characters form a single token.
fn tokenize(text: &str) -> std::collections::HashSet<String> {
    let mut tokens = std::collections::HashSet::new();
    let mut word = String::new();
    for ch in text.chars() {
        let is_cjk = matches!(ch, '\u{4E00}'..='\u{9FFF}' | '\u{3400}'..='\u{4DBF}'
            | '\u{F900}'..='\u{FAFF}' | '\u{2CEB0}'..='\u{2EBEF}');
        if is_cjk {
            if word.len() >= 2 {
                tokens.insert(word.to_lowercase());
            }
            word.clear();
            tokens.insert(ch.to_string());
        } else if ch.is_alphanumeric() {
            word.push(ch);
        } else {
            if word.len() >= 2 {
                tokens.insert(word.to_lowercase());
            }
            word.clear();
        }
    }
    if word.len() >= 2 {
        tokens.insert(word.to_lowercase());
    }
    tokens
}

/// Count how many tokens from `query_tokens` appear in `note`.
fn keyword_score(query_tokens: &std::collections::HashSet<String>, note: &str) -> usize {
    if query_tokens.is_empty() {
        return 0;
    }
    let note_tokens = tokenize(note);
    query_tokens.intersection(&note_tokens).count()
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

    fn temp_store() -> (ProjectStore, PathBuf) {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let base = std::env::temp_dir().join(format!("goldbot-proj-test-{nanos}-{id}"));
        fs::create_dir_all(&base).unwrap();
        (ProjectStore { base: base.clone() }, base)
    }

    #[test]
    fn append_memory_deduplicates() {
        let (store, base) = temp_store();
        assert!(store.append_memory("默认用中文回答").unwrap());
        assert!(!store.append_memory("默认用中文回答").unwrap());
        let content = fs::read_to_string(base.join("MEMORY.md")).unwrap();
        assert_eq!(content.matches("默认用中文回答").count(), 1);
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn build_memory_message_returns_none_when_empty() {
        let (store, base) = temp_store();
        assert!(store.build_memory_message(None).is_none());
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn build_memory_message_returns_notes() {
        let (store, base) = temp_store();
        store.append_memory("用 Ctrl+d 折叠").unwrap();
        let msg = store.build_memory_message(None).unwrap();
        assert!(msg.contains("### Project Memory"));
        assert!(msg.contains("Ctrl+d"));
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn build_memory_message_filters_by_keyword_when_over_limit() {
        let (store, base) = temp_store();
        // Add MEMORY_TOP_N + 2 notes; two are clearly relevant to "rebase git".
        for i in 0..(MEMORY_TOP_N - 1) {
            store.append_memory(&format!("数据库连接用 .env 文件 no{i}")).unwrap();
        }
        store.append_memory("用 rebase -i 合并 git 提交").unwrap();
        store.append_memory("git push 前先 cargo test").unwrap();
        let msg = store.build_memory_message(Some("帮我 rebase 这个 git 分支")).unwrap();
        assert!(msg.contains("rebase"), "relevant note must be included");
        assert!(!msg.contains("no0"), "unrelated notes should be filtered out");
        let _ = fs::remove_dir_all(base);
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
        let ts = ProjectStore::format_session_timestamp("20260331-142530");
        assert_eq!(ts, "2026-03-31  14:25:30");
    }

    #[test]
    fn format_session_timestamp_falls_back_for_bad_input() {
        let ts = ProjectStore::format_session_timestamp("bad");
        assert_eq!(ts, "bad");
    }
}
