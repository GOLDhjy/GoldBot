use std::path::{Path, PathBuf};

use anyhow::Result;

const MAX_GLOB_RESULTS: usize = 200;
const MAX_OUTPUT_CHARS: usize = 10_000;

#[derive(Debug)]
pub struct GlobResult {
    pub output: String,
    pub match_count: usize,
}

pub fn glob_files(pattern: &str, path: &str) -> Result<GlobResult> {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let root = if Path::new(path).is_absolute() {
        PathBuf::from(path)
    } else {
        cwd.join(path)
    };

    if !root.exists() {
        anyhow::bail!("path does not exist: {}", root.display());
    }

    let gp = glob::Pattern::new(pattern)
        .map_err(|e| anyhow::anyhow!("invalid glob pattern '{pattern}': {e}"))?;

    let walker = ignore::WalkBuilder::new(&root)
        .hidden(true)
        .git_ignore(true)
        .build();

    let mut matched: Vec<(String, std::time::SystemTime)> = Vec::new();

    for entry in walker {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };

        if !entry.file_type().is_some_and(|ft| ft.is_file()) {
            continue;
        }

        let rel = entry.path().strip_prefix(&root).unwrap_or(entry.path());
        let rel_str = rel.to_string_lossy().replace('\\', "/");

        if gp.matches(&rel_str) {
            let mtime = entry
                .metadata()
                .ok()
                .and_then(|m| m.modified().ok())
                .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
            matched.push((rel_str, mtime));
        }
    }

    matched.sort_by(|a, b| b.1.cmp(&a.1));

    let total = matched.len();
    let display = total.min(MAX_GLOB_RESULTS);

    let mut output = format!(
        "Found {total} file{} matching '{pattern}':\n",
        if total == 1 { "" } else { "s" }
    );
    for (p, _) in &matched[..display] {
        output.push_str(p);
        output.push('\n');
    }
    if total > MAX_GLOB_RESULTS {
        output.push_str(&format!("... ({} more files)\n", total - MAX_GLOB_RESULTS));
    }
    if output.len() > MAX_OUTPUT_CHARS {
        output.truncate(MAX_OUTPUT_CHARS);
        output.push_str("\n... (output truncated)\n");
    }

    Ok(GlobResult {
        output,
        match_count: total,
    })
}
