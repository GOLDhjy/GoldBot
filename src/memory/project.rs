use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::Result;
const MAX_MEMORY_NOTE_CHARS: usize = 120;
const MEMORY_SECTION: &str = "## Memories";
/// Maximum number of notes injected per LLM call.
const MEMORY_TOP_N: usize = 15;

// ── Process-level statics ─────────────────────────────────────────────────────

/// Workspace set once at startup; accessible throughout the process.
static CURRENT_WORKSPACE: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();

/// Call once at startup, before any ProjectStore operations.
pub fn init_workspace(workspace: PathBuf) {
    let _ = CURRENT_WORKSPACE.set(workspace);
}

// ── ProjectStore ──────────────────────────────────────────────────────────────

/// Project-level long-term memory storage.
///
/// Session logs live in `SessionStore`, but share the same project base path:
/// `~/.goldbot/projects/<sanitized_workspace_path>`.
pub struct ProjectStore {
    base: PathBuf,
}

impl ProjectStore {
    /// Build a store for the globally initialised workspace.
    /// Panics if `init_workspace` was never called.
    pub fn current() -> Self {
        let ws = CURRENT_WORKSPACE
            .get()
            .expect("init_workspace must be called before ProjectStore::current");
        Self::new(ws)
    }

    pub fn new(workspace: &Path) -> Self {
        Self {
            base: project_base_for(workspace),
        }
    }

    // ── Paths ─────────────────────────────────────────────────────────────────

    fn memory_path(&self) -> PathBuf {
        self.base.join("MEMORY.md")
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
            notes
                .iter()
                .rev()
                .take(MEMORY_TOP_N)
                .map(String::as_str)
                .collect()
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
}

// ── Path helpers ──────────────────────────────────────────────────────────────

pub(crate) fn current_project_base() -> PathBuf {
    let ws = CURRENT_WORKSPACE
        .get()
        .expect("init_workspace must be called before current_project_base");
    project_base_for(ws)
}

pub(crate) fn project_base_for(workspace: &Path) -> PathBuf {
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
            store
                .append_memory(&format!("数据库连接用 .env 文件 no{i}"))
                .unwrap();
        }
        store.append_memory("用 rebase -i 合并 git 提交").unwrap();
        store.append_memory("git push 前先 cargo test").unwrap();
        let msg = store
            .build_memory_message(Some("帮我 rebase 这个 git 分支"))
            .unwrap();
        assert!(msg.contains("rebase"), "relevant note must be included");
        assert!(
            !msg.contains("no0"),
            "unrelated notes should be filtered out"
        );
        let _ = fs::remove_dir_all(base);
    }
}
