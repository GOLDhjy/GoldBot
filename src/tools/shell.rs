use std::{
    collections::{BTreeSet, HashMap},
    fs,
    io::Read,
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
const MAX_COMPARE_CAPTURE_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperationKind {
    Search,
    Read,
    Write,
    Update,
    Bash,
}

impl OperationKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Search => "Search",
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

    if let Some(search_desc) = extract_search_descriptor(trimmed) {
        return CommandIntent {
            kind: OperationKind::Search,
            target: Some(search_desc),
        };
    }

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

    let target = target
        .or_else(|| matches!(kind, OperationKind::Read).then(|| ".".to_string()))
        .map(|t| absolutize_target_for_display(&t));

    CommandIntent { kind, target }
}

fn extract_search_descriptor(cmd: &str) -> Option<String> {
    let tokens = tokenize_shell_like(cmd);
    if tokens.is_empty() {
        return None;
    }

    let head = normalize_command_token(tokens.first()?);
    match head.as_str() {
        "rg" | "grep" => extract_grep_like_descriptor(&tokens),
        "find" => extract_find_descriptor(&tokens),
        _ => None,
    }
}

fn normalize_command_token(token: &str) -> String {
    token.rsplit('/').next().unwrap_or(token).to_lowercase()
}

fn extract_grep_like_descriptor(tokens: &[String]) -> Option<String> {
    let mut args: Vec<&str> = Vec::new();
    let mut i = 1usize;
    while i < tokens.len() {
        let t = tokens[i].as_str();
        if t == "--" {
            args.extend(tokens[i + 1..].iter().map(String::as_str));
            break;
        }

        if t == "-e" || t == "--regexp" {
            if let Some(pat) = tokens.get(i + 1) {
                args.push(pat.as_str());
                i += 2;
                continue;
            }
            break;
        }

        if t.starts_with('-') {
            i += 1;
            continue;
        }

        args.push(t);
        i += 1;
    }

    if args.is_empty() {
        return Some(format!("path: {}", absolutize_target_for_display(".")));
    }

    let pattern = args[0];
    let path = if args.len() >= 2 {
        args[args.len() - 1]
    } else {
        "."
    };
    Some(format!(
        "pattern: \"{}\", path: {}",
        truncate_chars(pattern, 40),
        absolutize_target_for_display(path)
    ))
}

fn extract_find_descriptor(tokens: &[String]) -> Option<String> {
    let mut i = 1usize;
    let mut path = ".";
    if i < tokens.len() && !tokens[i].starts_with('-') {
        path = tokens[i].as_str();
        i += 1;
    }

    let mut pattern: Option<&str> = None;
    while i < tokens.len() {
        let t = tokens[i].as_str();
        if matches!(
            t,
            "-name" | "-iname" | "-path" | "-ipath" | "-regex" | "-iregex" | "-wholename"
        ) {
            if let Some(p) = tokens.get(i + 1) {
                pattern = Some(p.as_str());
            }
            break;
        }
        i += 1;
    }

    let abs_path = absolutize_target_for_display(path);
    Some(match pattern {
        Some(p) => format!("pattern: \"{}\", path: {}", truncate_chars(p, 40), abs_path),
        None => format!("path: {}", abs_path),
    })
}

pub fn run_command(cmd: &str) -> Result<CommandResult> {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let before_compare = capture_before_compare(&cwd, cmd);
    let before = snapshot_files(&cwd);

    let output = if cfg!(target_os = "windows") {
        Command::new("powershell")
            .args(["-NoProfile", "-Command", cmd])
            .output()?
    } else {
        Command::new("bash").args(["-lc", cmd]).output()?
    };

    let after = snapshot_files(&cwd);
    let fs_summary = build_fs_summary(&cwd, &before, &after, &before_compare);

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
    if is_read_only_sed(trimmed) {
        return true;
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
        || matches_any_prefix(lower, &["tee "])
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

fn is_read_only_sed(cmd: &str) -> bool {
    let tokens = tokenize_shell_like(cmd);
    if tokens.is_empty() {
        return false;
    }
    if normalize_command_token(&tokens[0]) != "sed" {
        return false;
    }
    !tokens
        .iter()
        .skip(1)
        .any(|t| t == "-i" || t.starts_with("-i"))
}

fn matches_any_prefix(lower: &str, prefixes: &[&str]) -> bool {
    prefixes.iter().any(|p| lower.starts_with(p))
}

fn contains_write_redirection(cmd: &str) -> bool {
    let bytes = cmd.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'>' {
            let mut j = i + 1;
            if j < bytes.len() && (bytes[j] == b'>' || bytes[j] == b'|') {
                j += 1;
            }
            while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                j += 1;
            }
            if j >= bytes.len() {
                return true;
            }
            if bytes[j] == b'&' {
                i = j + 1;
                continue;
            }

            let start = j;
            while j < bytes.len() {
                let ch = bytes[j];
                if ch.is_ascii_whitespace() || matches!(ch, b';' | b'|' | b'&') {
                    break;
                }
                j += 1;
            }
            let target = cmd[start..j].trim_matches(|ch| matches!(ch, '"' | '\''));
            if is_non_mutating_redirection_target(target) {
                i = j;
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
        "pwd" => Some("."),
        "ls" | "find" => tokens
            .iter()
            .skip(1)
            .find(|t| !t.starts_with('-'))
            .copied()
            .or(Some(".")),
        "grep" | "rg" => {
            let args: Vec<&&str> = tokens
                .iter()
                .skip(1)
                .filter(|t| !t.starts_with('-'))
                .collect();
            if args.len() >= 2 {
                args.last().copied().copied()
            } else {
                None
            }
        }
        "git" => extract_git_target(&tokens),
        "cat" | "less" | "more" | "head" | "tail" | "stat" => {
            tokens.iter().skip(1).find(|t| !t.starts_with('-')).copied()
        }
        "sed" => tokens.last().copied(),
        "rm" | "mkdir" | "rmdir" | "touch" | "chmod" | "chown" => {
            tokens.iter().skip(1).find(|t| !t.starts_with('-')).copied()
        }
        "mv" | "cp" => tokens.last().copied(),
        "python" | "python3" => extract_python_script_target(cmd),
        _ => None,
    }?;

    normalize_target(candidate)
}

fn extract_git_target<'a>(tokens: &'a [&'a str]) -> Option<&'a str> {
    let mut i = 1usize;
    while i < tokens.len() && tokens[i].starts_with('-') {
        i += 1;
    }
    let sub = *tokens.get(i)?;
    match sub {
        "status" => Some("."),
        "diff" => {
            if let Some(pos) = tokens.iter().position(|t| *t == "--") {
                return tokens.get(pos + 1).copied();
            }
            Some(".")
        }
        _ => None,
    }
}

fn absolutize_target_for_display(target: &str) -> String {
    if target == "." {
        return std::env::current_dir()
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .unwrap_or_else(|_| ".".to_string());
    }
    let path = expand_tilde(target).unwrap_or_else(|| PathBuf::from(target));
    let abs = if path.is_absolute() {
        path
    } else {
        match std::env::current_dir() {
            Ok(cwd) => cwd.join(&path),
            Err(_) => path,
        }
    };
    abs.to_string_lossy().replace('\\', "/")
}

fn expand_tilde(target: &str) -> Option<PathBuf> {
    if target == "~" {
        return crate::tools::home_dir();
    }
    if let Some(rest) = target.strip_prefix("~/") {
        return crate::tools::home_dir().map(|home| home.join(rest));
    }
    None
}

fn extract_target_from_redirection(cmd: &str) -> Option<String> {
    let tokens: Vec<&str> = cmd.split_whitespace().collect();
    for (idx, token) in tokens.iter().enumerate() {
        if *token == ">" || *token == ">>" {
            if let Some(next) = tokens.get(idx + 1) {
                let target = next.trim_matches(|ch| matches!(ch, '"' | '\''));
                if is_non_mutating_redirection_target(target) {
                    continue;
                }
                return normalize_target(target);
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
                let target = cmd[start..j].trim_matches(|ch| matches!(ch, '"' | '\''));
                if is_non_mutating_redirection_target(target) {
                    continue;
                }
                return normalize_target(target);
            }
        }
    }
    None
}

fn is_non_mutating_redirection_target(target: &str) -> bool {
    if target == "/dev/null" {
        return true;
    }
    cfg!(target_os = "windows") && target.eq_ignore_ascii_case("nul")
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

fn truncate_chars(s: &str, max_chars: usize) -> String {
    let total = s.chars().count();
    if total <= max_chars {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max_chars.saturating_sub(1)).collect();
    out.push('…');
    out
}

fn tokenize_shell_like(segment: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_single = false;
    let mut in_double = false;
    let mut escaped = false;

    for ch in segment.chars() {
        if escaped {
            cur.push(ch);
            escaped = false;
            continue;
        }
        match ch {
            '\\' if !in_single => escaped = true,
            '\'' if !in_double => in_single = !in_single,
            '"' if !in_single => in_double = !in_double,
            c if c.is_whitespace() && !in_single && !in_double => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            c => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
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
    before_compare: &HashMap<PathBuf, String>,
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
        let path_label = display_path(p);
        if updated.contains(&p) {
            let before_text = before_compare.get(p.as_path());
            let after_text = read_preview(root, p);
            if let (Some(before_text), Some(after_text)) = (before_text, after_text) {
                let snippet = render_before_after_snippet(before_text, &after_text);
                lines.push(format!("Compare {}:", path_label));
                lines.extend(snippet.lines().map(|l| format!("  {l}")));
                continue;
            }
        }
        if let Some(preview) = read_preview(root, p) {
            lines.push(format!("Preview {}:", path_label));
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
    let content = read_text_limited(&full, MAX_COMPARE_CAPTURE_BYTES)?;
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

fn capture_before_compare(root: &Path, cmd: &str) -> HashMap<PathBuf, String> {
    let mut out = HashMap::new();
    let Some(target) = extract_target(cmd) else {
        return out;
    };
    let abs = absolutize_for_runtime(root, &target);
    let Ok(meta) = fs::metadata(&abs) else {
        return out;
    };
    if !meta.is_file() {
        return out;
    }

    let Ok(rel) = abs.strip_prefix(root).map(PathBuf::from) else {
        return out;
    };
    if should_skip(&rel) {
        return out;
    }
    if let Some(text) = read_text_limited(&abs, MAX_COMPARE_CAPTURE_BYTES) {
        out.insert(rel, text);
    }
    out
}

fn absolutize_for_runtime(root: &Path, target: &str) -> PathBuf {
    let path = expand_tilde(target).unwrap_or_else(|| PathBuf::from(target));
    if path.is_absolute() {
        path
    } else {
        root.join(path)
    }
}

fn read_text_limited(path: &Path, limit: usize) -> Option<String> {
    let mut file = fs::File::open(path).ok()?;
    let mut buf = Vec::with_capacity(limit.min(8192));
    let mut take = file.by_ref().take(limit as u64);
    if take.read_to_end(&mut buf).is_err() {
        return None;
    }
    String::from_utf8(buf).ok()
}

fn render_before_after_snippet(before: &str, after: &str) -> String {
    let before_lines: Vec<&str> = before.lines().collect();
    let after_lines: Vec<&str> = after.lines().collect();

    let mut i = 0usize;
    let max_eq = before_lines.len().min(after_lines.len());
    while i < max_eq && before_lines[i] == after_lines[i] {
        i += 1;
    }

    let start = i.saturating_sub(2);
    let end_before = (start + MAX_PREVIEW_LINES).min(before_lines.len());
    let end_after = (start + MAX_PREVIEW_LINES).min(after_lines.len());

    let mut out = String::new();
    out.push_str("Before:\n");
    if start >= end_before {
        out.push_str("  - (empty)\n");
    } else {
        for line in &before_lines[start..end_before] {
            out.push_str("  - ");
            out.push_str(&truncate_line_for_preview(line));
            out.push('\n');
        }
    }
    out.push_str("After:\n");
    if start >= end_after {
        out.push_str("  + (empty)");
    } else {
        for (idx, line) in after_lines[start..end_after].iter().enumerate() {
            out.push_str("  + ");
            out.push_str(&truncate_line_for_preview(line));
            if idx + 1 < end_after - start {
                out.push('\n');
            }
        }
    }
    out
}

fn truncate_line_for_preview(line: &str) -> String {
    if line.chars().count() > MAX_PREVIEW_CHARS_PER_LINE {
        let truncated: String = line.chars().take(MAX_PREVIEW_CHARS_PER_LINE).collect();
        format!("{truncated}...")
    } else {
        line.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::{OperationKind, classify_command};

    #[test]
    fn classify_read() {
        let intent = classify_command("cat README.md");
        assert_eq!(intent.kind, OperationKind::Read);
        let cwd = std::env::current_dir()
            .expect("cwd")
            .to_string_lossy()
            .replace('\\', "/");
        assert_eq!(intent.label(), format!("Read({cwd}/README.md)"));
    }

    #[test]
    fn classify_write_redirect() {
        let intent = classify_command("cat > README_EN.md << 'EOF'");
        assert_eq!(intent.kind, OperationKind::Write);
        let cwd = std::env::current_dir()
            .expect("cwd")
            .to_string_lossy()
            .replace('\\', "/");
        assert_eq!(intent.label(), format!("Write({cwd}/README_EN.md)"));
    }

    #[test]
    fn classify_update_rm() {
        let intent = classify_command("rm README_EN.md");
        assert_eq!(intent.kind, OperationKind::Update);
        let cwd = std::env::current_dir()
            .expect("cwd")
            .to_string_lossy()
            .replace('\\', "/");
        assert_eq!(intent.label(), format!("Update({cwd}/README_EN.md)"));
    }

    #[test]
    fn classify_git_status_has_cwd_target() {
        let intent = classify_command("git status");
        assert_eq!(intent.kind, OperationKind::Read);
        let cwd = std::env::current_dir()
            .expect("cwd")
            .to_string_lossy()
            .replace('\\', "/");
        assert_eq!(intent.label(), format!("Read({cwd})"));
    }

    #[test]
    fn classify_rg_is_search() {
        let intent = classify_command("rg plan_from_codex_or_sample src");
        assert_eq!(intent.kind, OperationKind::Search);
        let cwd = std::env::current_dir()
            .expect("cwd")
            .to_string_lossy()
            .replace('\\', "/");
        assert_eq!(
            intent.label(),
            format!("Search(pattern: \"plan_from_codex_or_sample\", path: {cwd}/src)")
        );
    }

    #[test]
    fn classify_sed_print_is_read() {
        let intent = classify_command("sed -n '738,920p' src/main.rs");
        assert_eq!(intent.kind, OperationKind::Read);
    }

    #[test]
    fn classify_sed_in_place_is_update() {
        let intent = classify_command("sed -i '' 's/foo/bar/g' src/main.rs");
        assert_eq!(intent.kind, OperationKind::Update);
    }

    #[test]
    fn classify_ls_stderr_to_dev_null_is_read() {
        let intent =
            classify_command("ls -la .github/ 2>/dev/null || echo \"No .github directory\"");
        assert_eq!(intent.kind, OperationKind::Read);
        let cwd = std::env::current_dir()
            .expect("cwd")
            .to_string_lossy()
            .replace('\\', "/");
        assert_eq!(intent.label(), format!("Read({cwd}/.github/)"));
    }
}
