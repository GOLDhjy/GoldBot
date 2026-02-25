use std::io::Read;
use std::path::{Path, PathBuf};
use std::{fs, path};

use regex::Regex;

const MAX_MATCHES: usize = 300;
const MAX_LINE_CHARS: usize = 200;
const MAX_OUTPUT_CHARS: usize = 10_000;
const MAX_FILE_BYTES: u64 = 512 * 1024;

pub struct SearchResult {
    pub output: String,
    pub match_count: usize,
    pub file_count: usize,
}

/// 在文件树中搜索匹配 `pattern` 的行，`path` 为相对或绝对路径（默认 "."）。
pub fn search_files(pattern: &str, path: &str) -> anyhow::Result<SearchResult> {
    let re = Regex::new(pattern)?;
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));

    let search_root = {
        let p = PathBuf::from(path);
        if p.as_os_str().is_empty() || path == "." {
            cwd.clone()
        } else if p.is_absolute() {
            p
        } else {
            cwd.join(p)
        }
    };

    let mut output = String::new();
    let mut match_count = 0usize;
    let mut file_count = 0usize;
    let mut truncated = false;

    if search_root.is_file() {
        search_file(
            &search_root,
            &search_root
                .file_name()
                .map(|n| PathBuf::from(n))
                .unwrap_or_else(|| search_root.clone()),
            &re,
            &mut output,
            &mut match_count,
            &mut file_count,
            &mut truncated,
        );
    } else {
        search_dir(
            &search_root,
            &search_root,
            &re,
            &mut output,
            &mut match_count,
            &mut file_count,
            &mut truncated,
        );
    }

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

fn search_dir(
    root: &Path,
    dir: &Path,
    re: &Regex,
    output: &mut String,
    match_count: &mut usize,
    file_count: &mut usize,
    truncated: &mut bool,
) {
    if *truncated {
        return;
    }

    let mut entries: Vec<_> = match fs::read_dir(dir) {
        Ok(e) => e.flatten().collect(),
        Err(_) => return,
    };
    entries.sort_by_key(|e| e.file_name());

    for entry in entries {
        if *truncated {
            return;
        }

        let path = entry.path();
        let rel = match path.strip_prefix(root) {
            Ok(p) => p.to_path_buf(),
            Err(_) => continue,
        };

        if should_skip(&rel) {
            continue;
        }

        let ft = match entry.file_type() {
            Ok(ft) => ft,
            Err(_) => continue,
        };

        if ft.is_dir() {
            search_dir(root, &path, re, output, match_count, file_count, truncated);
        } else if ft.is_file() {
            if let Ok(meta) = entry.metadata() {
                if meta.len() > MAX_FILE_BYTES {
                    continue;
                }
            }
            search_file(&path, &rel, re, output, match_count, file_count, truncated);
        }
    }
}

fn search_file(
    path: &Path,
    rel: &Path,
    re: &Regex,
    output: &mut String,
    match_count: &mut usize,
    file_count: &mut usize,
    truncated: &mut bool,
) {
    if *truncated {
        return;
    }
    let text = match read_text(path) {
        Some(t) => t,
        None => return,
    };

    let mut file_had_match = false;
    for (lineno, line) in text.lines().enumerate() {
        if !re.is_match(line) {
            continue;
        }
        if !file_had_match {
            file_had_match = true;
            *file_count += 1;
        }
        *match_count += 1;

        let rel_str = rel.to_string_lossy().replace(path::MAIN_SEPARATOR, "/");
        let line_display = if line.len() > MAX_LINE_CHARS {
            format!("{}…", &line[..line.floor_char_boundary(MAX_LINE_CHARS)])
        } else {
            line.to_string()
        };
        output.push_str(&format!("{}:{}: {}\n", rel_str, lineno + 1, line_display));

        if output.len() >= MAX_OUTPUT_CHARS || *match_count >= MAX_MATCHES {
            *truncated = true;
            return;
        }
    }
}

fn should_skip(rel: &Path) -> bool {
    let first = rel.iter().next().and_then(|s| s.to_str()).unwrap_or("");
    if first.starts_with('.') {
        return true;
    }
    matches!(
        first,
        "target"
            | "node_modules"
            | "dist"
            | "build"
            | "out"
            | "obj"
            | "vendor"
            | "__pycache__"
            | "Binaries"
            | "Saved"
            | "Intermediate"
            | "DerivedDataCache"
    )
}

/// 读取文本文件；如果是二进制文件则返回 None。
/// 自动去除 UTF-8 BOM，统一换行为 LF。
fn read_text(path: &Path) -> Option<String> {
    let mut file = fs::File::open(path).ok()?;
    let mut buf = Vec::new();
    file.by_ref()
        .take(MAX_FILE_BYTES)
        .read_to_end(&mut buf)
        .ok()?;
    // 简单二进制检测：含空字节视为二进制
    if buf.contains(&0u8) {
        return None;
    }
    let s = String::from_utf8(buf).ok()?;
    // Strip UTF-8 BOM and normalize CRLF → LF
    let s = s.trim_start_matches('\u{FEFF}');
    Some(s.replace("\r\n", "\n"))
}
