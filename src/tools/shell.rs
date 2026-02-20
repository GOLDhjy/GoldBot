use std::{
    collections::{BTreeSet, HashMap},
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::UNIX_EPOCH,
};

use anyhow::Result;

const MAX_OUTPUT_CHARS: usize = 10_000;
const MAX_SNAPSHOT_FILES: usize = 20_000;
const MAX_DIFF_PER_KIND: usize = 6;
const MAX_PREVIEW_FILES: usize = 2;
const MAX_PREVIEW_LINES: usize = 8;
const MAX_PREVIEW_CHARS_PER_LINE: usize = 140;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperationKind {
    Read,
    Write,
    Update,
    Bash,
}

impl OperationKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Read => "Read",
            Self::Write => "Write",
            Self::Update => "Update",
            Self::Bash => "Bash",
        }
    }
}

#[derive(Debug, Clone)]
pub struct CommandIntent {
    pub kind: OperationKind,
    pub target: Option<String>,
}

impl CommandIntent {
    pub fn label(&self) -> String {
        match &self.target {
            Some(target) => format!("{}({target})", self.kind.as_str()),
            None => self.kind.as_str().to_string(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct CommandResult {
    pub exit_code: i32,
    pub output: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FileSignature {
    size: u64,
    modified_secs: u64,
    modified_nanos: u32,
}

pub fn classify_command(cmd: &str) -> CommandIntent {
    let trimmed = cmd.trim();
    let lower = trimmed.to_lowercase();
    let target = extract_target(trimmed);

    let kind = if looks_read_only(trimmed, &lower) {
        OperationKind::Read
    } else if looks_write(trimmed, &lower) {
        OperationKind::Write
    } else if looks_update(trimmed, &lower) {
        OperationKind::Update
    } else {
        OperationKind::Bash
    };

    CommandIntent { kind, target }
}

pub fn run_command(cmd: &str) -> Result<CommandResult> {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let before = snapshot_files(&cwd);

    let output = if cfg!(target_os = "windows") {
        Command::new("powershell")
            .args(["-NoProfile", "-Command", cmd])
            .output()?
    } else {
        Command::new("bash").args(["-lc", cmd]).output()?
    };

    let after = snapshot_files(&cwd);
    let fs_summary = build_fs_summary(&cwd, &before, &after);

    let mut text = String::new();
    text.push_str(&String::from_utf8_lossy(&output.stdout));
    text.push_str(&String::from_utf8_lossy(&output.stderr));

    if !fs_summary.is_empty() {
        if !text.trim_end().is_empty() {
            text.push('\n');
        }
        text.push_str(&fs_summary);
    }

    if text.trim().is_empty() {
        text = "(no output)".to_string();
    }

    if text.len() > MAX_OUTPUT_CHARS {
        text.truncate(MAX_OUTPUT_CHARS);
        text.push_str("\n...[truncated]");
    }

    Ok(CommandResult {
        exit_code: output.status.code().unwrap_or(-1),
        output: text,
    })
}

fn looks_read_only(trimmed: &str, lower: &str) -> bool {
    if contains_write_redirection(trimmed) {
        return false;
    }
    matches_any_prefix(
        lower,
        &[
            "cat ",
            "less ",
            "more ",
            "ls",
            "pwd",
            "find ",
            "grep ",
            "rg ",
            "head ",
            "tail ",
            "wc ",
            "stat ",
            "du ",
            "tree",
            "git status",
            "git log",
            "git show",
        ],
    )
}

fn looks_write(trimmed: &str, lower: &str) -> bool {
    contains_write_redirection(trimmed)
        || lower.contains("<<")
        || matches_any_prefix(lower, &["tee ", "touch ", "printf ", "echo "])
        || lower.contains("open(") && (lower.contains("\"w\"") || lower.contains("'w'"))
}

fn looks_update(_trimmed: &str, lower: &str) -> bool {
    matches_any_prefix(
        lower,
        &[
            "rm ", "mv ", "cp ", "mkdir ", "rmdir ", "chmod ", "chown ", "sed -i", "perl -pi",
            "git add ", "git rm ", "git mv ",
        ],
    )
}

fn matches_any_prefix(lower: &str, prefixes: &[&str]) -> bool {
    prefixes.iter().any(|p| lower.starts_with(p))
}

fn contains_write_redirection(cmd: &str) -> bool {
    let bytes = cmd.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'>' {
            if i + 1 < bytes.len() && bytes[i + 1] == b'&' {
                i += 2;
                continue;
            }
            return true;
        }
        i += 1;
    }
    false
}

fn extract_target(cmd: &str) -> Option<String> {
    if let Some(target) = extract_target_from_redirection(cmd) {
        return Some(target);
    }

    let tokens: Vec<&str> = cmd.split_whitespace().collect();
    if tokens.is_empty() {
        return None;
    }

    let first = tokens[0];
    let candidate = match first {
        "cat" | "less" | "more" | "head" | "tail" | "stat" => {
            tokens.iter().skip(1).find(|t| !t.starts_with('-')).copied()
        }
        "rm" | "mkdir" | "rmdir" | "touch" | "chmod" | "chown" => {
            tokens.iter().skip(1).find(|t| !t.starts_with('-')).copied()
        }
        "mv" | "cp" => tokens.last().copied(),
        "python" | "python3" => extract_python_script_target(cmd),
        _ => None,
    }?;

    normalize_target(candidate)
}

fn extract_target_from_redirection(cmd: &str) -> Option<String> {
    let tokens: Vec<&str> = cmd.split_whitespace().collect();
    for (idx, token) in tokens.iter().enumerate() {
        if *token == ">" || *token == ">>" {
            if let Some(next) = tokens.get(idx + 1) {
                return normalize_target(next);
            }
        }
    }

    let bytes = cmd.as_bytes();
    for i in 0..bytes.len() {
        if bytes[i] == b'>' {
            if i + 1 < bytes.len() && bytes[i + 1] == b'&' {
                continue;
            }
            let mut j = i + 1;
            while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                j += 1;
            }
            if j < bytes.len() && bytes[j] == b'>' {
                j += 1;
            }
            while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                j += 1;
            }
            let start = j;
            while j < bytes.len() && !bytes[j].is_ascii_whitespace() {
                j += 1;
            }
            if start < j {
                return normalize_target(&cmd[start..j]);
            }
        }
    }
    None
}

fn extract_python_script_target(cmd: &str) -> Option<&str> {
    if let Some(i) = cmd.find("open(") {
        let s = &cmd[i + "open(".len()..];
        let quote = s.chars().next()?;
        if quote == '\'' || quote == '"' {
            let rest = &s[quote.len_utf8()..];
            if let Some(end) = rest.find(quote) {
                return Some(&rest[..end]);
            }
        }
    }
    None
}

fn normalize_target(s: &str) -> Option<String> {
    let cleaned = s
        .trim_matches(|c| c == '\'' || c == '"' || c == '`' || c == ';' || c == ',' || c == ')')
        .trim();
    if cleaned.is_empty() || cleaned.starts_with('-') {
        return None;
    }
    Some(cleaned.to_string())
}

fn snapshot_files(root: &Path) -> HashMap<PathBuf, FileSignature> {
    let mut out = HashMap::new();
    walk_dir(root, root, &mut out);
    out
}

fn walk_dir(root: &Path, dir: &Path, out: &mut HashMap<PathBuf, FileSignature>) {
    if out.len() >= MAX_SNAPSHOT_FILES {
        return;
    }

    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        if out.len() >= MAX_SNAPSHOT_FILES {
            return;
        }

        let path = entry.path();
        let rel = match path.strip_prefix(root) {
            Ok(p) => p,
            Err(_) => continue,
        };

        if should_skip(rel) {
            continue;
        }

        let file_type = match entry.file_type() {
            Ok(ft) => ft,
            Err(_) => continue,
        };

        if file_type.is_dir() {
            walk_dir(root, &path, out);
            continue;
        }

        if !file_type.is_file() {
            continue;
        }

        if let Ok(meta) = entry.metadata() {
            let modified = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map(|d| (d.as_secs(), d.subsec_nanos()))
                .unwrap_or((0, 0));
            out.insert(
                rel.to_path_buf(),
                FileSignature {
                    size: meta.len(),
                    modified_secs: modified.0,
                    modified_nanos: modified.1,
                },
            );
        }
    }
}

fn should_skip(rel: &Path) -> bool {
    let first = rel.iter().next().and_then(|s| s.to_str()).unwrap_or("");
    matches!(first, ".git" | "target")
}

fn build_fs_summary(
    root: &Path,
    before: &HashMap<PathBuf, FileSignature>,
    after: &HashMap<PathBuf, FileSignature>,
) -> String {
    let before_keys: BTreeSet<&PathBuf> = before.keys().collect();
    let after_keys: BTreeSet<&PathBuf> = after.keys().collect();

    let created: Vec<&PathBuf> = after_keys.difference(&before_keys).copied().collect();
    let deleted: Vec<&PathBuf> = before_keys.difference(&after_keys).copied().collect();
    let updated: Vec<&PathBuf> = before_keys
        .intersection(&after_keys)
        .filter(|p| before.get((*p).as_path()) != after.get((*p).as_path()))
        .copied()
        .collect();

    if created.is_empty() && deleted.is_empty() && updated.is_empty() {
        return String::new();
    }

    let mut lines: Vec<String> = Vec::new();
    lines.push("Filesystem changes:".to_string());
    push_change_lines(&mut lines, "created", &created, '+');
    push_change_lines(&mut lines, "updated", &updated, '~');
    push_change_lines(&mut lines, "deleted", &deleted, '-');

    let mut preview_paths: Vec<&PathBuf> = created.iter().chain(updated.iter()).copied().collect();
    preview_paths.truncate(MAX_PREVIEW_FILES);
    for p in preview_paths {
        if let Some(preview) = read_preview(root, p) {
            lines.push(format!("Preview {}:", display_path(p)));
            lines.extend(preview.lines().map(|l| format!("  {l}")));
        }
    }

    lines.join("\n")
}

fn push_change_lines(lines: &mut Vec<String>, label: &str, paths: &[&PathBuf], marker: char) {
    if paths.is_empty() {
        return;
    }
    lines.push(format!("  {label} ({})", paths.len()));
    for path in paths.iter().take(MAX_DIFF_PER_KIND) {
        lines.push(format!("    {marker} {}", display_path(path)));
    }
    if paths.len() > MAX_DIFF_PER_KIND {
        lines.push(format!(
            "    ... and {} more",
            paths.len() - MAX_DIFF_PER_KIND
        ));
    }
}

fn display_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn read_preview(root: &Path, rel: &Path) -> Option<String> {
    let full = root.join(rel);
    let content = fs::read_to_string(full).ok()?;
    let mut out = String::new();
    for (i, line) in content.lines().take(MAX_PREVIEW_LINES).enumerate() {
        if i > 0 {
            out.push('\n');
        }
        if line.chars().count() > MAX_PREVIEW_CHARS_PER_LINE {
            let truncated: String = line.chars().take(MAX_PREVIEW_CHARS_PER_LINE).collect();
            out.push_str(&truncated);
            out.push_str("...");
        } else {
            out.push_str(line);
        }
    }
    if out.is_empty() {
        out.push_str("(empty file)");
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::{OperationKind, classify_command};

    #[test]
    fn classify_read() {
        let intent = classify_command("cat README.md");
        assert_eq!(intent.kind, OperationKind::Read);
        assert_eq!(intent.label(), "Read(README.md)");
    }

    #[test]
    fn classify_write_redirect() {
        let intent = classify_command("cat > README_EN.md << 'EOF'");
        assert_eq!(intent.kind, OperationKind::Write);
        assert_eq!(intent.label(), "Write(README_EN.md)");
    }

    #[test]
    fn classify_update_rm() {
        let intent = classify_command("rm README_EN.md");
        assert_eq!(intent.kind, OperationKind::Update);
        assert_eq!(intent.label(), "Update(README_EN.md)");
    }
}
