use std::{
    collections::{BTreeSet, HashMap},
    fs,
    path::PathBuf,
};

use anyhow::Result;
use chrono::{Duration, Local, NaiveDate};

const SHORT_TERM_PROMPT_TAIL_CHARS: usize = 1800;
const SHORT_TERM_PROMPT_DAYS: i64 = 2;
const MAX_SHORT_TERM_FINAL_CHARS: usize = 4000;
const MAX_LONG_TERM_NOTE_CHARS: usize = 120;
const MAX_LONG_TERM_NOTES_IN_PROMPT: usize = 30;
const ENV_MEMORY_DIR: &str = "GOLDBOT_MEMORY_DIR";
const LT_SECTION_CAPS_ZH: &str = "## Bot Capabilities";
const LT_SECTION_MEM_ZH: &str = "## Conversation Memories";
const PROMOTE_LOOKBACK_DAYS: i64 = 14;
const PROMOTE_MIN_COUNT: usize = 3;
const PROMOTE_MAX_NOTES_PER_RUN: usize = 3;
const PROMOTE_MAX_TASK_SAMPLES: usize = 500;

pub struct MemoryStore {
    base: PathBuf,
}

impl MemoryStore {
    pub fn new() -> Self {
        Self {
            base: default_memory_base_dir(),
        }
    }

    fn short_term_path(&self) -> PathBuf {
        let day = Local::now().format("%Y-%m-%d").to_string();
        self.base.join("memory").join(format!("{day}.md"))
    }

    fn short_term_path_for_day(&self, day: NaiveDate) -> PathBuf {
        self.base
            .join("memory")
            .join(format!("{}.md", day.format("%Y-%m-%d")))
    }

    fn long_term_path(&self) -> PathBuf {
        self.base.join("MEMORY.md")
    }

    /// Return the base memory directory as a display string (for use in prompts).
    pub fn base_dir_display(&self) -> String {
        self.base.to_string_lossy().into_owned()
    }

    pub fn append_short_term(&self, task: &str, final_output: &str) -> Result<()> {
        let path = self.short_term_path();
        let day = Local::now().format("%Y-%m-%d").to_string();
        ensure_markdown_header(&path, &format!("Short-term Memory {day}"))?;
        let now = Local::now().format("%H:%M:%S");
        let task = sanitize_fenced_text(task.trim());
        let final_output = truncate_chars(final_output.trim(), MAX_SHORT_TERM_FINAL_CHARS);
        let final_output = sanitize_fenced_text(&final_output);
        let block = format!(
            "\n## {now}\n- **Task**\n\n```text\n{task}\n```\n- **Final**\n\n```text\n{final_output}\n```\n"
        );
        append_file(path, &block)
    }

    /// 将命令执行产生的文件差异写入今日短期记忆，方便后续查阅或恢复文件
    pub fn append_diff_to_short_term(&self, cmd: &str, diffs: &[(String, String)]) -> Result<()> {
        if diffs.is_empty() {
            return Ok(());
        }
        let path = self.short_term_path();
        let day = Local::now().format("%Y-%m-%d").to_string();
        ensure_markdown_header(&path, &format!("Short-term Memory {day}"))?;
        let now = Local::now().format("%H:%M:%S");
        let cmd = sanitize_fenced_text(cmd.trim());
        let mut block = format!("\n## {now} [diff]\n- **Command**: `{cmd}`\n");
        for (label, diff) in diffs {
            block.push_str(&format!("\n- **File**: {label}\n\n```diff\n{diff}\n```\n"));
        }
        append_file(path, &block)
    }

    pub fn append_long_term_if_new(&self, note: &str) -> Result<bool> {
        let path = self.long_term_path();
        ensure_long_term_template(&path)?;

        let note = normalize_note(note);
        if note.is_empty() {
            return Ok(false);
        }
        let canonical_note = canonicalize_note(&note);

        if let Ok(existing) = fs::read_to_string(&path) {
            let already_exists = long_term_notes_from_markdown(&existing)
                .into_iter()
                .any(|n| canonicalize_note(&n) == canonical_note);
            if already_exists {
                return Ok(false);
            }
        }

        let block = format!("- {note}\n");
        append_file(path, &block)?;
        Ok(true)
    }

    pub fn derive_long_term_notes(&self, task: &str, final_output: &str) -> Vec<String> {
        derive_long_term_notes(task, final_output)
    }

    pub fn promote_repeated_short_term_to_long_term(&self) -> Result<usize> {
        let tasks =
            self.collect_recent_short_term_tasks(PROMOTE_LOOKBACK_DAYS, PROMOTE_MAX_TASK_SAMPLES);
        if tasks.is_empty() {
            return Ok(0);
        }

        let mut counts: HashMap<String, usize> = HashMap::new();
        for task in tasks {
            let key = normalize_task_key(&task);
            if key.is_empty() || !eligible_frequent_memory_task(&key) {
                continue;
            }
            *counts.entry(key).or_insert(0) += 1;
        }

        let mut candidates: Vec<(String, usize)> = counts
            .into_iter()
            .filter(|(_, c)| *c >= PROMOTE_MIN_COUNT)
            .collect();
        if candidates.is_empty() {
            return Ok(0);
        }
        candidates.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));

        let mut promoted = 0usize;
        for (note, _) in candidates.into_iter().take(PROMOTE_MAX_NOTES_PER_RUN) {
            if self.append_long_term_if_new(&note)? {
                promoted += 1;
            }
        }

        Ok(promoted)
    }

    /// Build the memory assistant message injected at conversation start.
    /// Returns `None` when there is no memory to inject.
    pub fn build_memory_message(&self) -> Option<String> {
        let long_term_path = self.long_term_path();
        let _ = ensure_long_term_template(&long_term_path);

        let long_term = fs::read_to_string(&long_term_path)
            .ok()
            .map(|s| curated_long_term_for_prompt(&s))
            .unwrap_or_default();

        let mut short_chunks = Vec::new();
        let today = Local::now().date_naive();
        for i in (0..SHORT_TERM_PROMPT_DAYS).rev() {
            let day = today - Duration::days(i);
            let path = self.short_term_path_for_day(day);
            let Ok(content) = fs::read_to_string(path) else {
                continue;
            };
            let snippet = tail_chars(
                &content,
                SHORT_TERM_PROMPT_TAIL_CHARS / SHORT_TERM_PROMPT_DAYS as usize,
            );
            if !snippet.trim().is_empty() {
                short_chunks.push(format!(
                    "### {}\n{}",
                    day.format("%Y-%m-%d"),
                    snippet.trim()
                ));
            }
        }
        let short_term = short_chunks.join("\n\n");

        if long_term.trim().is_empty() && short_term.trim().is_empty() {
            return None;
        }

        Some(format!(
            "## Memory\n\
             On conflict, follow the latest user instruction.\n\n\
             ### Long-term Memory\n\
             {}\n\n\
             ### Recent Short-term Memory\n\
             {}",
            fallback_if_empty(&long_term),
            fallback_if_empty(&short_term),
        ))
    }

    fn collect_recent_short_term_tasks(&self, days: i64, max_tasks: usize) -> Vec<String> {
        let mut tasks = Vec::new();
        let today = Local::now().date_naive();
        for i in (0..days).rev() {
            let day = today - Duration::days(i);
            let path = self.short_term_path_for_day(day);
            let Ok(content) = fs::read_to_string(path) else {
                continue;
            };
            tasks.extend(extract_task_blocks_from_short_term(&content));
            if tasks.len() >= max_tasks {
                tasks.drain(..tasks.len() - max_tasks);
                break;
            }
        }
        tasks
    }
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

fn derive_long_term_notes(task: &str, final_output: &str) -> Vec<String> {
    if !has_explicit_memory_intent(task) {
        return Vec::new();
    }

    let mut notes = Vec::new();
    let task_line = normalize_note(task);

    for line in final_output.lines() {
        let cleaned = line
            .trim_start_matches(|c: char| {
                c.is_ascii_digit() || c.is_whitespace() || matches!(c, '-' | '*' | '•' | '.')
            })
            .trim();
        if cleaned.is_empty() {
            continue;
        }
        let normalized = normalize_note(cleaned);
        if normalized.is_empty() {
            continue;
        }
        notes.push(normalized);
        if notes.len() >= 3 {
            break;
        }
    }

    if notes.is_empty() && !task_line.is_empty() {
        notes.push(task_line);
    }

    let mut uniq = BTreeSet::new();
    notes
        .into_iter()
        .filter(|n| uniq.insert(n.clone()))
        .collect::<Vec<_>>()
}

fn has_explicit_memory_intent(task: &str) -> bool {
    let lower = task.to_lowercase();
    [
        "记住",
        "请记",
        "帮我记",
        "别忘",
        "以后",
        "下次",
        "默认",
        "偏好",
        "always",
        "from now on",
        "next time",
        "default to",
        "remember",
        "preference",
        "prefer",
    ]
    .iter()
    .any(|k| lower.contains(k))
}

fn curated_long_term_for_prompt(raw: &str) -> String {
    let notes = long_term_notes_from_markdown(raw);

    if notes.is_empty() {
        return String::new();
    }

    let start = notes.len().saturating_sub(MAX_LONG_TERM_NOTES_IN_PROMPT);
    notes[start..]
        .iter()
        .map(|n| format!("- {n}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn normalize_note(text: &str) -> String {
    let mut sentence = text
        .replace('`', "")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim_matches(|c: char| {
            c.is_whitespace() || matches!(c, '，' | ',' | '。' | '.' | ':' | '：')
        })
        .to_string();

    sentence = truncate_chars(&sentence, MAX_LONG_TERM_NOTE_CHARS);
    if sentence.is_empty() {
        return sentence;
    }
    if !ends_with_sentence_punctuation(&sentence) {
        let end = if sentence.is_ascii() { "." } else { "。" };
        sentence.push_str(end);
    }
    sentence
}

fn normalize_task_key(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim_matches(|c: char| {
            c.is_whitespace() || matches!(c, '，' | ',' | '。' | '.' | ':' | '：')
        })
        .to_string()
}

fn eligible_frequent_memory_task(task: &str) -> bool {
    let len = task.chars().count();
    if !(6..=80).contains(&len) {
        return false;
    }
    let lower = task.to_lowercase();
    if lower.ends_with('?')
        || lower.ends_with('？')
        || lower.contains("吗")
        || lower.contains("什么")
        || lower.contains("怎么")
        || lower.contains("why")
        || lower.starts_with("how ")
        || lower.starts_with("what ")
    {
        return false;
    }

    [
        "默认",
        "以后",
        "偏好",
        "习惯",
        "请用",
        "使用",
        "不要",
        "别",
        "快捷键",
        "风控",
        "显示",
        "折叠",
        "展开",
        "确认",
        "中文",
        "英文",
        "default",
        "prefer",
        "from now on",
        "use ",
    ]
    .iter()
    .any(|k| lower.contains(k))
}

fn ends_with_sentence_punctuation(s: &str) -> bool {
    s.ends_with(['。', '！', '？', '.', '!', '?', ';', '；'])
}

fn canonicalize_note(text: &str) -> String {
    text.trim()
        .trim_end_matches(['。', '.', '!', '?', ';', '；'])
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn fallback_if_empty(text: &str) -> String {
    if text.trim().is_empty() {
        "(none)".to_string()
    } else {
        text.trim().to_string()
    }
}

fn tail_chars(text: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }
    let count = text.chars().count();
    if count <= max_chars {
        return text.to_string();
    }
    text.chars().skip(count - max_chars).collect()
}

fn truncate_chars(text: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }
    let count = text.chars().count();
    if count <= max_chars {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max_chars.saturating_sub(1)).collect();
    out.push('…');
    out
}

fn sanitize_fenced_text(text: &str) -> String {
    text.replace("```", "``\\`")
}

fn ensure_markdown_header(path: &PathBuf, title: &str) -> Result<()> {
    if path.exists() {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, format!("# {title}\n"))?;
    Ok(())
}

fn long_term_template() -> String {
    format!(
        "# Long-term Memory\n\n\
         {LT_SECTION_CAPS_ZH}\n\
         - Execute tools: shell commands (Read/Search/Write/Update/Bash) and optional MCP tools.\n\
         - Risk control for mutating commands (confirm/block).\n\
         - Show tool traces and execution results.\n\
         - Maintain short-term and long-term memory.\n\n\
         {LT_SECTION_MEM_ZH}\n"
    )
}

fn ensure_long_term_template(path: &PathBuf) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    if !path.exists() {
        fs::write(path, long_term_template())?;
        return Ok(());
    }

    let existing = fs::read_to_string(path)?;
    let has_caps = existing.contains(LT_SECTION_CAPS_ZH);
    let has_mem = existing.contains(LT_SECTION_MEM_ZH);
    if has_caps && has_mem {
        return Ok(());
    }

    let notes = long_term_notes_from_markdown(&existing);
    let mut migrated = long_term_template();
    if !notes.is_empty() {
        for note in notes {
            let note = normalize_note(&note);
            if note.is_empty() {
                continue;
            }
            migrated.push_str(&format!("- {note}\n"));
        }
    }
    fs::write(path, migrated)?;
    Ok(())
}

fn long_term_notes_from_markdown(raw: &str) -> Vec<String> {
    let mut notes = Vec::new();
    let mut in_memory_section = false;
    let mut saw_memory_section = false;

    for line in raw.lines() {
        let t = line.trim();
        if t.starts_with("## ") {
            in_memory_section =
                t == LT_SECTION_MEM_ZH || t.eq_ignore_ascii_case("## Conversation Memories");
            saw_memory_section = saw_memory_section || in_memory_section;
            continue;
        }
        if !in_memory_section {
            continue;
        }
        let Some(note) = t.strip_prefix("- ") else {
            continue;
        };
        let note = note.trim();
        if note.is_empty() || note.starts_with("(none") || note.starts_with("task=") {
            continue;
        }
        if note.chars().count() > MAX_LONG_TERM_NOTE_CHARS + 40 {
            continue;
        }
        notes.push(note.to_string());
    }

    // Backward compatibility for older files without section template.
    if saw_memory_section || !notes.is_empty() {
        return notes;
    }
    raw.lines()
        .filter_map(|line| line.trim_start().strip_prefix("- ").map(str::trim))
        .filter(|note| !note.is_empty() && !note.starts_with("task="))
        .map(ToString::to_string)
        .collect()
}

fn extract_task_blocks_from_short_term(raw: &str) -> Vec<String> {
    let mut tasks = Vec::new();
    let lines: Vec<&str> = raw.lines().collect();
    let mut i = 0usize;
    while i < lines.len() {
        if lines[i].trim() != "- **Task**" {
            i += 1;
            continue;
        }

        i += 1;
        while i < lines.len() && lines[i].trim().is_empty() {
            i += 1;
        }
        if i >= lines.len() {
            break;
        }

        if lines[i].trim_start().starts_with("```") {
            i += 1;
            let mut block = String::new();
            while i < lines.len() {
                let t = lines[i].trim_start();
                if t.starts_with("```") {
                    break;
                }
                if !block.is_empty() {
                    block.push('\n');
                }
                block.push_str(lines[i]);
                i += 1;
            }
            let normalized = normalize_task_key(&block);
            if !normalized.is_empty() {
                tasks.push(normalized);
            }
            while i < lines.len() && !lines[i].trim_start().starts_with("```") {
                i += 1;
            }
        } else {
            let mut block = String::new();
            while i < lines.len() {
                let t = lines[i].trim();
                if t.starts_with("## ") || t == "- **Final**" || t == "- **Task**" {
                    break;
                }
                if !block.is_empty() {
                    block.push('\n');
                }
                block.push_str(lines[i]);
                i += 1;
            }
            let normalized = normalize_task_key(&block);
            if !normalized.is_empty() {
                tasks.push(normalized);
            }
            continue;
        }
        i += 1;
    }
    tasks
}

fn default_memory_base_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os(ENV_MEMORY_DIR) {
        let p = PathBuf::from(dir);
        if !p.as_os_str().is_empty() {
            return p;
        }
    }

    if let Some(home) = crate::tools::home_dir() {
        return home.join(".goldbot");
    }

    PathBuf::from(".goldbot")
}

#[cfg(test)]
mod tests {
    use super::{
        LT_SECTION_MEM_ZH, MAX_LONG_TERM_NOTE_CHARS, MemoryStore, ends_with_sentence_punctuation,
    };
    use chrono::Local;
    use std::{
        fs,
        path::PathBuf,
        sync::atomic::{AtomicU64, Ordering},
        time::{SystemTime, UNIX_EPOCH},
    };

    static NEXT_TEST_ID: AtomicU64 = AtomicU64::new(0);

    fn unique_base() -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time")
            .as_nanos();
        let id = NEXT_TEST_ID.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!("goldbot-memory-test-{nanos}-{id}"))
    }

    #[test]
    fn long_term_append_is_deduplicated() {
        let base = unique_base();
        fs::create_dir_all(&base).expect("mkdir");
        let store = MemoryStore { base: base.clone() };

        assert!(
            store
                .append_long_term_if_new("默认使用 Ctrl+d")
                .expect("append")
        );
        assert!(
            !store
                .append_long_term_if_new("默认使用 Ctrl+d")
                .expect("dedupe")
        );

        let content = fs::read_to_string(base.join("MEMORY.md")).expect("read");
        let count = content.matches("默认使用 Ctrl+d").count();
        assert_eq!(count, 1);
        assert!(content.starts_with("# Long-term Memory"));
        assert!(content.contains("## Bot Capabilities"));
        assert!(content.contains(LT_SECTION_MEM_ZH));
        assert!(content.lines().any(|l| l.starts_with("- ")));

        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn derive_long_term_requires_signal() {
        let base = unique_base();
        fs::create_dir_all(&base).expect("mkdir");
        let store = MemoryStore { base: base.clone() };

        let notes =
            store.derive_long_term_notes("帮我列一下文件", "项目目录包含 src 和 Cargo.toml");
        assert!(notes.is_empty());

        let notes =
            store.derive_long_term_notes("你现在有什么记忆", "我会把短期和长期记忆分开处理。");
        assert!(notes.is_empty());

        let notes = store.derive_long_term_notes(
            "以后默认使用 Ctrl+d 展开",
            "好的，记住：默认快捷键是 Ctrl+d。",
        );
        assert!(!notes.is_empty());
        assert!(notes[0].chars().count() <= MAX_LONG_TERM_NOTE_CHARS + 1);
        assert!(!notes[0].contains("=>"));
        assert!(ends_with_sentence_punctuation(&notes[0]));

        let notes = store.derive_long_term_notes(
            "以后都用中文回我",
            "我记住了。从现在开始，我会用中文回答你的问题。请告诉我你需要什么帮助？",
        );
        assert!(!notes[0].is_empty());
        assert!(ends_with_sentence_punctuation(&notes[0]));

        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn promote_repeated_short_term_tasks_into_long_term() {
        let base = unique_base();
        fs::create_dir_all(base.join("memory")).expect("mkdir");
        let day = Local::now().format("%Y-%m-%d").to_string();
        let path = base.join("memory").join(format!("{day}.md"));
        fs::write(
            &path,
            "# Short-term Memory\n\n\
             ## 10:00:00\n- **Task**\n\n```text\n以后默认用中文回答\n```\n- **Final**\n\n```text\n收到\n```\n\n\
             ## 10:10:00\n- **Task**\n\n```text\n以后默认用中文回答\n```\n- **Final**\n\n```text\n收到\n```\n\n\
             ## 10:20:00\n- **Task**\n\n```text\n以后默认用中文回答\n```\n- **Final**\n\n```text\n收到\n```\n",
        )
        .expect("write");

        let store = MemoryStore { base: base.clone() };
        let promoted = store
            .promote_repeated_short_term_to_long_term()
            .expect("promote");
        assert!(promoted >= 1);

        let long = fs::read_to_string(base.join("MEMORY.md")).expect("read long");
        assert!(long.contains("以后默认用中文回答"));

        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn build_system_prompt_migrates_legacy_long_term_file() {
        let base = unique_base();
        fs::create_dir_all(&base).expect("mkdir");
        fs::write(
            base.join("MEMORY.md"),
            "- 以后默认使用中文回答。\n- 默认展示 compact 视图。\n",
        )
        .expect("write legacy");

        let store = MemoryStore { base: base.clone() };
        let prompt = store.build_system_prompt("BASE");

        let content = fs::read_to_string(base.join("MEMORY.md")).expect("read migrated");
        assert!(content.starts_with("# Long-term Memory"));
        assert!(content.contains("## Bot Capabilities"));
        assert!(content.contains(LT_SECTION_MEM_ZH));
        assert!(content.contains("- 以后默认使用中文回答。"));
        assert!(content.contains("- 默认展示 compact 视图。"));
        assert!(prompt.contains("### Long-term Memory"));
        assert!(prompt.contains("- 以后默认使用中文回答。"));

        let _ = fs::remove_dir_all(base);
    }
}
