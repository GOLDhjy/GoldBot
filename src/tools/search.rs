use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use std::{fs, path};

use ignore::WalkBuilder;
use regex::Regex;

const MAX_MATCHES: usize = 300;
const MAX_LINE_CHARS: usize = 200;
const MAX_OUTPUT_CHARS: usize = 10_000;
const MAX_FILE_BYTES: u64 = 512 * 1024;
const DEFAULT_SEARCH_TIMEOUT_SECS: u64 = 30;

#[derive(Debug)]
pub struct SearchResult {
    pub output: String,
    pub match_count: usize,
    pub file_count: usize,
}

/// 在文件树中搜索匹配 `pattern` 的行，`path` 为相对或绝对路径（默认 "."）。
/// 使用 ignore crate 实现并行遍历，自动尊重 .gitignore。
/// 超时时间由环境变量 `GOLDBOT_SEARCH_TIMEOUT` 控制（秒），默认 30s。
pub fn search_files(pattern: &str, path: &str) -> anyhow::Result<SearchResult> {
    let re = Regex::new(pattern)?;

    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let search_root = {
        let p = PathBuf::from(path);
        if p.as_os_str().is_empty() || path == "." {
            cwd
        } else if p.is_absolute() {
            p
        } else {
            cwd.join(p)
        }
    };

    let timeout_secs: u64 = std::env::var("GOLDBOT_SEARCH_TIMEOUT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_SEARCH_TIMEOUT_SECS);
    let deadline = Instant::now() + Duration::from_secs(timeout_secs);

    if search_root.is_file() {
        return Ok(search_single_file(&search_root, &re));
    }

    let re = Arc::new(re);
    let root = Arc::new(search_root.clone());
    let output: Arc<Mutex<String>> = Arc::new(Mutex::new(String::new()));
    let match_count = Arc::new(AtomicUsize::new(0));
    let file_count = Arc::new(AtomicUsize::new(0));
    let truncated = Arc::new(AtomicBool::new(false));

    WalkBuilder::new(&search_root)
        .standard_filters(true) // 尊重 .gitignore / .ignore，跳过隐藏文件
        .build_parallel()
        .run(|| {
            let re = Arc::clone(&re);
            let root = Arc::clone(&root);
            let output = Arc::clone(&output);
            let match_count = Arc::clone(&match_count);
            let file_count = Arc::clone(&file_count);
            let truncated = Arc::clone(&truncated);

            Box::new(move |result| {
                use ignore::WalkState;

                if truncated.load(Ordering::Relaxed)
                    || crate::tools::shell::is_cancel_requested()
                    || Instant::now() >= deadline
                {
                    return WalkState::Quit;
                }

                let entry = match result {
                    Ok(e) => e,
                    Err(_) => return WalkState::Continue,
                };

                // 只处理普通文件
                if !entry.file_type().map_or(false, |ft| ft.is_file()) {
                    return WalkState::Continue;
                }

                // 跳过超大文件
                if entry.metadata().map_or(false, |m| m.len() > MAX_FILE_BYTES) {
                    return WalkState::Continue;
                }

                // 跳过 UE5 特有的大目录（.gitignore 未覆盖时的保底）
                if should_skip_entry(entry.path(), &root) {
                    return WalkState::Continue;
                }

                let text = match read_text(entry.path()) {
                    Some(t) => t,
                    None => return WalkState::Continue,
                };

                let rel = entry
                    .path()
                    .strip_prefix(root.as_ref())
                    .unwrap_or(entry.path());
                let rel_str = rel.to_string_lossy().replace(path::MAIN_SEPARATOR, "/");

                let mut local_lines = String::new();
                let mut local_matches = 0usize;
                let mut file_had_match = false;

                for (lineno, line) in text.lines().enumerate() {
                    if !re.is_match(line) {
                        continue;
                    }
                    file_had_match = true;
                    local_matches += 1;

                    let line_display = if line.chars().count() > MAX_LINE_CHARS {
                        let cut = line
                            .char_indices()
                            .nth(MAX_LINE_CHARS)
                            .map(|(i, _)| i)
                            .unwrap_or(line.len());
                        format!("{}…", &line[..cut])
                    } else {
                        line.to_string()
                    };
                    local_lines.push_str(&format!(
                        "{}:{}: {}\n",
                        rel_str,
                        lineno + 1,
                        line_display
                    ));
                }

                if !file_had_match {
                    return WalkState::Continue;
                }

                file_count.fetch_add(1, Ordering::Relaxed);
                let total = match_count.fetch_add(local_matches, Ordering::Relaxed) + local_matches;

                {
                    let mut out = output.lock().unwrap();
                    out.push_str(&local_lines);
                    if total >= MAX_MATCHES || out.len() >= MAX_OUTPUT_CHARS {
                        truncated.store(true, Ordering::Relaxed);
                        return WalkState::Quit;
                    }
                }

                WalkState::Continue
            })
        });

    let mut output = Arc::try_unwrap(output).unwrap().into_inner().unwrap();
    let match_count = match_count.load(Ordering::Relaxed);
    let file_count = file_count.load(Ordering::Relaxed);
    let truncated = truncated.load(Ordering::Relaxed);

    if truncated {
        output.push_str("... (results truncated)\n");
    }
    if output.is_empty() {
        output = "(no matches found)".to_string();
    }

    Ok(SearchResult {
        output,
        match_count,
        file_count,
    })
}

/// 单文件搜索（search_root 本身是文件时）
fn search_single_file(path: &Path, re: &Regex) -> SearchResult {
    let Some(text) = read_text(path) else {
        return SearchResult {
            output: "(no matches found)".to_string(),
            match_count: 0,
            file_count: 0,
        };
    };

    let name = path
        .file_name()
        .map(PathBuf::from)
        .unwrap_or_else(|| path.to_path_buf());
    let rel_str = name.to_string_lossy().replace(path::MAIN_SEPARATOR, "/");
    let mut output = String::new();
    let mut match_count = 0usize;
    let mut file_had_match = false;

    for (lineno, line) in text.lines().enumerate() {
        if !re.is_match(line) {
            continue;
        }
        file_had_match = true;
        match_count += 1;
        let line_display = if line.len() > MAX_LINE_CHARS {
            format!("{}…", &line[..line.floor_char_boundary(MAX_LINE_CHARS)])
        } else {
            line.to_string()
        };
        output.push_str(&format!("{}:{}: {}\n", rel_str, lineno + 1, line_display));
        if output.len() >= MAX_OUTPUT_CHARS || match_count >= MAX_MATCHES {
            output.push_str("... (results truncated)\n");
            break;
        }
    }

    if output.is_empty() {
        output = "(no matches found)".to_string();
    }

    SearchResult {
        output,
        match_count,
        file_count: if file_had_match { 1 } else { 0 },
    }
}

/// 对 ignore crate 未能过滤的 UE5/大型项目目录做额外保底跳过
fn should_skip_entry(path: &Path, root: &Path) -> bool {
    let rel = match path.strip_prefix(root) {
        Ok(r) => r,
        Err(_) => return false,
    };
    let first = rel.iter().next().and_then(|s| s.to_str()).unwrap_or("");
    matches!(
        first,
        "Binaries" | "Saved" | "Intermediate" | "DerivedDataCache" | "target" | "node_modules"
    )
}

/// 读取文本文件；二进制文件返回 None。去除 UTF-8 BOM，统一换行为 LF。
fn read_text(path: &Path) -> Option<String> {
    let mut file = fs::File::open(path).ok()?;
    let mut buf = Vec::new();
    file.by_ref()
        .take(MAX_FILE_BYTES)
        .read_to_end(&mut buf)
        .ok()?;
    if buf.contains(&0u8) {
        return None;
    }
    let s = String::from_utf8(buf).ok()?;
    let s = s.trim_start_matches('\u{FEFF}');
    Some(s.replace("\r\n", "\n"))
}
