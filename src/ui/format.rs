use std::path::Path;

use crossterm::style::Stylize;
use unicode_width::UnicodeWidthStr;

use crate::types::Event;
use crate::ui::symbols::Symbols;

pub(crate) fn format_event(event: &Event) -> Vec<String> {
    let sym = Symbols::current();
    match event {
        Event::UserTask { text } => lines_with(text, |i, line| {
            if i == 0 {
                format!("{} {}", sym.prompt, line).bold().to_string()
            } else {
                format!("  {}", line)
            }
        }),
        Event::Thinking { text } => lines_with(text, |_, line| {
            format!("  {}", line).grey().to_string()
        }),
        Event::ToolCall { label, command, .. } => {
            let mut lines = vec![format!("  {} {}", sym.record, label).cyan().to_string()];
            lines.extend(lines_with(command, |_, line| {
                format!("    {}", line).grey().to_string()
            }));
            lines
        }
        Event::ToolResult { output, exit_code } => {
            let ok = *exit_code == 0;
            lines_with(output, |i, line| {
                let pfx = if i == 0 { format!("  {} ", sym.corner) } else { "    ".to_string() };
                if !ok {
                    format!("{}{}", pfx, line).red().to_string()
                } else {
                    style_tool_result_line(&pfx, line)
                }
            })
        }
        Event::NeedsConfirmation { command, reason } => {
            let mut lines = vec![
                format!("  {} {}", sym.record, reason).cyan().bold().to_string(),
                format!("  {} 需要确认", sym.warning).dark_yellow().to_string(),
            ];
            for line in command.lines().take(6) {
                lines.push(format!("    {}", line).cyan().to_string());
            }
            if command.lines().count() > 6 {
                lines.push(format!("    {}", sym.ellipsis).grey().to_string());
            }
            lines
        }
        Event::Final { summary } => format_final_lines(summary),
    }
}

pub(crate) fn format_event_live(event: &Event) -> Vec<String> {
    let sym = Symbols::current();
    match event {
        Event::UserTask { .. } | Event::Final { .. } => format_event(event),
        Event::Thinking { text } => {
            text.lines()
                .filter(|l| !l.trim().is_empty())
                .take(3)
                .map(|line| format!("  {}", shorten_text(line, 110)).grey().to_string())
                .collect()
        }
        Event::ToolCall { label, command, multiline } => {
            let mut lines = vec![format!("  {} {}", sym.record, label).cyan().to_string()];
            if *multiline {
                for line in command.lines() {
                    lines.push(format!("    {}", line).grey().to_string());
                }
            } else if let Some(first) = command.lines().next() {
                lines.push(format!("    {}", first).grey().to_string());
            }
            lines
        }
        Event::ToolResult { output, exit_code } => compact_tool_result_lines(*exit_code, output),
        Event::NeedsConfirmation { .. } => format_event(event),
    }
}

pub(crate) fn emit_live_event(screen: &mut super::screen::Screen, event: &Event) {
    screen.emit(&format_event_live(event));
}

pub(crate) fn format_event_compact(event: &Event) -> Vec<String> {
    let sym = Symbols::current();
    match event {
        Event::Thinking { .. } => Vec::new(),
        Event::ToolCall { label, .. } => vec![format!("  {} {}", sym.bullet, label).cyan().to_string()],
        Event::ToolResult { output, exit_code } => compact_tool_result_lines(*exit_code, output),
        Event::NeedsConfirmation { command, reason } => {
            let mut lines = vec![
                format!("  {} {}", sym.record, reason).cyan().bold().to_string(),
                format!("    {} 需要确认", sym.warning).dark_yellow().to_string(),
            ];
            for line in command.lines().take(4) {
                lines.push(format!("      {}", shorten_text(line, 72)).cyan().to_string());
            }
            if command.lines().count() > 4 {
                lines.push(format!("      {}", sym.ellipsis).grey().to_string());
            }
            lines
        }
        Event::Final { .. } | Event::UserTask { .. } => Vec::new(),
    }
}

fn lines_with(text: &str, f: impl Fn(usize, &str) -> String) -> Vec<String> {
    let v: Vec<&str> = text.lines().collect();
    if v.is_empty() {
        return vec![f(0, "")];
    }
    v.iter().enumerate().map(|(i, l)| f(i, l)).collect()
}

fn format_final_lines(summary: &str) -> Vec<String> {
    let lines: Vec<&str> = summary.lines().collect();
    if lines.is_empty() {
        return vec!["  ✓ Done".green().bold().to_string()];
    }

    let has_diff = looks_like_diff_block(&lines);
    let mut out = Vec::with_capacity(lines.len());
    let mut in_code_fence = false;

    for (i, line) in lines.iter().enumerate() {
        let trimmed = line.trim_start();

        // First line: task result header.
        if i == 0 {
            out.push(format!(
                "  {} {}",
                "✓".green().bold(),
                render_inline_markdown(trimmed).bold()
            ));
            continue;
        }

        // Code fence toggle.
        if trimmed.starts_with("```") {
            if in_code_fence {
                in_code_fence = false;
                // Blank line after code block as visual separator.
                out.push(String::new());
            } else {
                in_code_fence = true;
                let lang = trimmed.trim_start_matches('`').trim();
                if !lang.is_empty() {
                    out.push(format!("    {}", lang).grey().to_string());
                }
            }
            continue;
        }

        // Inside a code block: no background fill (avoid uneven per-line block width).
        if in_code_fence {
            out.push(format!("    {}", line).grey().to_string());
            continue;
        }

        let rendered = format!("    {}", line);

        if has_diff {
            if trimmed.starts_with('+') && !trimmed.starts_with("+++") {
                out.push(rendered.green().to_string());
                continue;
            }
            if trimmed.starts_with('-') && !trimmed.starts_with("---") {
                out.push(rendered.red().to_string());
                continue;
            }
            if trimmed.starts_with("@@")
                || trimmed.starts_with("diff --git")
                || trimmed.starts_with("index ")
                || trimmed.starts_with("--- ")
                || trimmed.starts_with("+++ ")
            {
                out.push(rendered.dark_yellow().to_string());
                continue;
            }
        }

        if is_markdown_rule(trimmed) {
            out.push("    ─────────────────────────────".grey().to_string());
            continue;
        }
        if let Some((level, heading)) = parse_markdown_heading(trimmed) {
            let r = format!("    {}", render_inline_markdown(heading));
            out.push(match level {
                1 => r.bold().green().to_string(),
                2 => r.bold().yellow().to_string(),
                _ => r.bold().dark_yellow().to_string(),
            });
            continue;
        }
        if let Some(item) = parse_markdown_list_item(trimmed) {
            out.push(format_bullet_line(item));
            continue;
        }
        if is_markdown_table_separator(trimmed) {
            out.push("    ─────────────────────────────".grey().to_string());
            continue;
        }
        if is_markdown_table_row(trimmed) {
            out.push(format_table_row(trimmed));
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("> ") {
            out.push(format!("    {}", render_inline_markdown(rest)).grey().to_string());
            continue;
        }
        if let Some(section) = split_trailing_section_title(trimmed) {
            out.push(format!("    {}", render_inline_markdown(section)).bold().yellow().to_string());
            continue;
        }
        if let Some((key, sep, value)) = split_key_value_parts(trimmed) {
            let key = render_inline_markdown(key);
            let value = render_inline_markdown(value);
            out.push(format!("    {}{} {}", key.bold().yellow(), sep, value));
            continue;
        }

        out.push(format!("    {}", render_inline_markdown(trimmed)));
    }

    out
}

fn looks_like_diff_block(lines: &[&str]) -> bool {
    lines.iter().any(|line| {
        let t = line.trim_start();
        t.starts_with("diff --git")
            || t.starts_with("@@")
            || t.starts_with("--- ")
            || t.starts_with("+++ ")
            || t.starts_with("index ")
    })
}

pub(crate) fn is_markdown_rule_pub(line: &str) -> bool {
    is_markdown_rule(line)
}

fn is_markdown_rule(line: &str) -> bool {
    let t = line.trim();
    t.len() >= 3 && t.chars().all(|c| matches!(c, '-' | '*' | '_'))
}

/// `|------|------|` separator row
pub(crate) fn is_markdown_table_separator(line: &str) -> bool {
    let t = line.trim();
    if !t.starts_with('|') || !t.ends_with('|') {
        return false;
    }
    t.chars().all(|c| matches!(c, '|' | '-' | ':' | ' '))
}

/// Any `| col | col |` table data row
pub(crate) fn is_markdown_table_row(line: &str) -> bool {
    let t = line.trim();
    t.starts_with('|') && t.ends_with('|') && t.contains(" | ")
}

pub(crate) fn format_table_row_pub(line: &str) -> String {
    format_table_row(line)
}

fn format_table_row(line: &str) -> String {
    let cells: Vec<&str> = line
        .trim()
        .trim_start_matches('|')
        .trim_end_matches('|')
        .split('|')
        .map(str::trim)
        .collect();
    let parts: Vec<String> = cells
        .iter()
        .map(|c| render_inline_markdown(c))
        .collect();
    format!("    {}", parts.join("  │  "))
}

fn parse_markdown_heading(line: &str) -> Option<(usize, &str)> {
    let t = line.trim_start();
    let bytes = t.as_bytes();
    let mut level = 0usize;
    while level < bytes.len() && bytes[level] == b'#' {
        level += 1;
    }
    if level == 0 || level > 6 || level >= bytes.len() || bytes[level] != b' ' {
        return None;
    }
    Some((level, t[level + 1..].trim()))
}

fn parse_markdown_list_item(line: &str) -> Option<&str> {
    if let Some(rest) = line.strip_prefix("• ") {
        return Some(rest.trim());
    }
    if let Some(rest) = line.strip_prefix("- ").or_else(|| line.strip_prefix("* ")) {
        return Some(rest.trim());
    }
    strip_ordered_marker(line).map(str::trim)
}

fn split_trailing_section_title(line: &str) -> Option<&str> {
    let t = line.trim();
    if t.is_empty() {
        return None;
    }
    if let Some(section) = t.strip_suffix(':').or_else(|| t.strip_suffix('：')) {
        let section = section.trim();
        if !section.is_empty() {
            return Some(section);
        }
    }
    None
}

pub(crate) fn split_key_value_parts_pub(line: &str) -> Option<(&str, char, &str)> {
    split_key_value_parts(line)
}

fn split_key_value_parts(line: &str) -> Option<(&str, char, &str)> {
    let t = line.trim();
    for (idx, ch) in t.char_indices() {
        if ch != ':' && ch != '：' {
            continue;
        }
        let key = t[..idx].trim_end();
        let value = t[idx + ch.len_utf8()..].trim_start();
        if key.is_empty() || value.is_empty() {
            return None;
        }
        if key.contains("://") || value.starts_with("//") || key.len() > 40 {
            return None;
        }
        return Some((key, ch, value));
    }
    None
}

fn format_bullet_line(item: &str) -> String {
    // Checkbox: - [ ] or - [x]
    if let Some(rest) = item.strip_prefix("[ ] ") {
        return format!("    {} {}", "☐".grey(), render_inline_markdown(rest));
    }
    if let Some(rest) = item.strip_prefix("[x] ").or_else(|| item.strip_prefix("[X] ")) {
        return format!("    {} {}", "☑".green(), render_inline_markdown(rest).green().to_string().as_str());
    }
    if let Some((key, sep, value)) = split_key_value_parts(item) {
        let key = render_inline_markdown(key);
        let value = render_inline_markdown(value);
        return format!(
            "    {} {}{} {}",
            crate::ui::symbols::Symbols::current().bullet.grey(),
            key.bold().yellow(),
            sep,
            value
        );
    }
    format!("    {} {}", crate::ui::symbols::Symbols::current().bullet.grey(), render_inline_markdown(item))
}

/// Detects our line-numbered diff format: `"NNN - content"` or `"NNN + content"`.
/// Returns `Some('-')` / `Some('+')` on a match, `None` otherwise.
fn numbered_diff_marker(t: &str) -> Option<char> {
    let rest = t.trim_start_matches(|c: char| c.is_ascii_digit());
    if rest.len() == t.len() {
        return None; // no leading digits
    }
    if rest.starts_with(" - ") {
        Some('-')
    } else if rest.starts_with(" + ") {
        Some('+')
    } else {
        None
    }
}


/// Render a diff line with colored background starting at `content` (after `prefix`).
/// The prefix is left uncolored; the content + trailing padding fill the rest of the
/// terminal width with a dark red (delete) or dark green (insert) background.
/// White foreground is applied so text is readable on both dark backgrounds.
fn bg_fill_after_prefix(prefix: &str, content: &str, is_delete: bool) -> String {
    let term_width = crossterm::terminal::size().map(|(w, _)| w as usize).unwrap_or(80);
    let prefix_width = UnicodeWidthStr::width(prefix);
    let content_width = UnicodeWidthStr::width(content);
    let total_used = prefix_width + content_width;
    let padding = if term_width > total_used {
        " ".repeat(term_width - total_used)
    } else {
        String::new()
    };
    let padded_content = format!("{content}{padding}");
    let colored = if is_delete {
        padded_content.white().on_dark_red().to_string()
    } else {
        padded_content.white().on_dark_green().to_string()
    };
    format!("{prefix}{colored}")
}

/// Style a single tool-result line.
/// `pfx` is the indent prefix (e.g. `"  ⎿ "` / `"    "`); `raw_line` is the
/// content without the prefix.  Returns the complete styled string.
fn style_tool_result_line(pfx: &str, raw_line: &str) -> String {
    let rendered = format!("{pfx}{raw_line}");
    let t = raw_line.trim_start();
    // Numbered diff lines: prefix uncolored, content filled with red/green bg + white text.
    match numbered_diff_marker(t) {
        Some('-') => return bg_fill_after_prefix(pfx, raw_line, true),
        Some('+') => return bg_fill_after_prefix(pfx, raw_line, false),
        _ => {}
    }
    // Hunk separator, plain diff markers, fs summary, diff headers — keep their original styling.
    if t.starts_with('─') {
        return rendered.dark_grey().to_string();
    }
    if t.starts_with('+') {
        return rendered.green().to_string();
    }
    if t.starts_with('-') {
        return rendered.red().to_string();
    }
    if t.starts_with('~') {
        return rendered.cyan().to_string();
    }
    if t == "Filesystem changes:" {
        return rendered.dark_yellow().bold().to_string();
    }
    if t.starts_with("created (") {
        return rendered.green().to_string();
    }
    if t.starts_with("updated (") {
        return rendered.cyan().to_string();
    }
    if t.starts_with("deleted (") {
        return rendered.red().to_string();
    }
    if t.starts_with("Diff ") || t.starts_with("Preview ") {
        return rendered.dark_yellow().to_string();
    }
    // All other lines (including unchanged diff context): white text, no background.
    rendered.white().to_string()
}

pub(crate) fn render_inline_markdown_pub(line: &str) -> String {
    render_inline_markdown(line)
}

fn render_inline_markdown(line: &str) -> String {
    let chars: Vec<char> = line.chars().collect();
    let mut out = String::new();
    let mut i = 0usize;

    while i < chars.len() {
        if i + 1 < chars.len() && chars[i] == '*' && chars[i + 1] == '*' {
            let mut j = i + 2;
            while j + 1 < chars.len() {
                if chars[j] == '*' && chars[j + 1] == '*' {
                    break;
                }
                j += 1;
            }
            if j + 1 < chars.len() {
                let segment: String = chars[i + 2..j].iter().collect();
                out.push_str(&segment.bold().to_string());
                i = j + 2;
                continue;
            }
        }

        if chars[i] == '`' {
            let mut j = i + 1;
            while j < chars.len() && chars[j] != '`' {
                j += 1;
            }
            if j < chars.len() {
                let segment: String = chars[i + 1..j].iter().collect();
                out.push_str(&segment.cyan().to_string());
                i = j + 1;
                continue;
            }
        }

        // Single-star italic: *text* — strip the markers, keep plain text
        if chars[i] == '*' {
            let mut j = i + 1;
            while j < chars.len() && chars[j] != '*' && chars[j] != '\n' {
                j += 1;
            }
            if j < chars.len() && chars[j] == '*' && j > i + 1 {
                let segment: String = chars[i + 1..j].iter().collect();
                out.push_str(&segment);
                i = j + 1;
                continue;
            }
        }

        out.push(chars[i]);
        i += 1;
    }

    out
}

pub(crate) fn sanitize_final_summary_for_tui(text: &str) -> String {
    let mut out = Vec::<String>::new();
    let mut in_code_fence = false;

    for raw in text.lines() {
        let fence_probe = raw.trim_start();
        if fence_probe.starts_with("```") {
            in_code_fence = !in_code_fence;
            // Keep the fence line so format_final_lines can render code blocks properly.
            out.push(fence_probe.to_string());
            continue;
        }

        let line = if in_code_fence {
            raw.trim_end().to_string()
        } else {
            normalize_emoji_spacing(raw.trim())
        };
        out.push(line);
    }

    let mut compact = Vec::<String>::new();
    let mut prev_blank = true;
    for line in out {
        if line.is_empty() {
            if !prev_blank {
                compact.push(String::new());
            }
            prev_blank = true;
        } else {
            compact.push(line);
            prev_blank = false;
        }
    }
    while compact.last().is_some_and(|l| l.is_empty()) {
        compact.pop();
    }
    compact.join("\n")
}

pub(crate) fn strip_ordered_marker_pub(line: &str) -> Option<&str> {
    strip_ordered_marker(line)
}

fn strip_ordered_marker(line: &str) -> Option<&str> {
    let bytes = line.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        i += 1;
    }
    if i > 0 && i + 1 < bytes.len() && bytes[i] == b'.' && bytes[i + 1] == b' ' {
        return Some(&line[i + 2..]);
    }
    None
}

#[derive(Default)]
pub(crate) struct FsChangeSummary {
    pub created: Vec<String>,
    pub updated: Vec<String>,
    pub deleted: Vec<String>,
}

fn compact_tool_result_lines(exit_code: i32, output: &str) -> Vec<String> {
    let mut summary = Vec::new();
    if let Some(fs) = parse_fs_changes(output) {
        if !fs.created.is_empty() {
            summary.push(format!("    {} created: {}", crate::ui::symbols::Symbols::current().corner, summarize_paths(&fs.created)));
        }
        if !fs.updated.is_empty() {
            summary.push(format!("    {} updated: {}", crate::ui::symbols::Symbols::current().corner, summarize_paths(&fs.updated)));
        }
        if !fs.deleted.is_empty() {
            summary.push(format!("    {} deleted: {}", crate::ui::symbols::Symbols::current().corner, summarize_paths(&fs.deleted)));
        }
    }

    if summary.is_empty() {
        if let Some(line) = first_non_empty_line(output) {
            summary.push(format!("    {} {}", crate::ui::symbols::Symbols::current().corner, shorten_text(line, 110)));
        } else {
            summary.push(format!("    {} (no output)", crate::ui::symbols::Symbols::current().corner));
        }
    }

    // Summary lines: colored by exit code.
    let mut result: Vec<String> = summary
        .into_iter()
        .map(|line| {
            if exit_code == 0 {
                line.grey().to_string()
            } else {
                line.red().to_string()
            }
        })
        .collect();

    // Append diff blocks with per-line coloring (red/green).
    // render_unified_diff already caps at MAX_DIFF_LINES, so no second limit needed.
    for diff_lines in parse_diff_blocks(output) {
        for dl in &diff_lines {
            let prefix = "      ";
            let t = dl.trim_start();
            let colored = match numbered_diff_marker(t) {
                Some('-') => bg_fill_after_prefix(prefix, dl, true),
                Some('+') => bg_fill_after_prefix(prefix, dl, false),
                _ if t.starts_with('─') => format!("{prefix}{dl}").dark_grey().to_string(),
                _ => format!("{prefix}{dl}").white().to_string(),
            };
            result.push(colored);
        }
    }

    result
}

/// Extract diff line-blocks from command output.
/// Each `Diff <file>:` section's lines are returned as a separate `Vec<String>`.
fn parse_diff_blocks(output: &str) -> Vec<Vec<String>> {
    let mut result = Vec::new();
    let mut current: Option<Vec<String>> = None;

    for line in output.lines() {
        let t = line.trim();
        if t.starts_with("Diff ") && t.ends_with(':') {
            if let Some(block) = current.take() {
                result.push(block);
            }
            current = Some(Vec::new());
        } else if let Some(block) = current.as_mut() {
            block.push(t.to_string());
        }
    }
    if let Some(block) = current {
        result.push(block);
    }
    result
}

fn parse_fs_changes(output: &str) -> Option<FsChangeSummary> {
    let mut in_section = false;
    let mut fs = FsChangeSummary::default();

    for line in output.lines() {
        let t = line.trim();
        if !in_section {
            if t == "Filesystem changes:" {
                in_section = true;
            }
            continue;
        }

        if t.starts_with("Preview ") || t.starts_with("Diff ") {
            break;
        }
        if let Some(path) = t.strip_prefix("+ ") {
            fs.created.push(path.to_string());
            continue;
        }
        if let Some(path) = t.strip_prefix("~ ") {
            fs.updated.push(path.to_string());
            continue;
        }
        if let Some(path) = t.strip_prefix("- ") {
            fs.deleted.push(path.to_string());
        }
    }

    if !in_section {
        return None;
    }
    Some(fs)
}

fn summarize_paths(paths: &[String]) -> String {
    const MAX_SHOWN: usize = 2;
    let mut out = paths
        .iter()
        .take(MAX_SHOWN)
        .map(|p| absolutize_path_for_display(p))
        .collect::<Vec<_>>()
        .join(", ");
    if paths.len() > MAX_SHOWN {
        out.push_str(&format!(" (+{} more)", paths.len() - MAX_SHOWN));
    }
    out
}

fn absolutize_path_for_display(path: &str) -> String {
    let p = Path::new(path);
    let abs = if p.is_absolute() {
        p.to_path_buf()
    } else {
        std::env::current_dir()
            .map(|cwd| cwd.join(p))
            .unwrap_or_else(|_| p.to_path_buf())
    };
    abs.to_string_lossy().replace('\\', "/")
}

pub(crate) fn first_non_empty_line(output: &str) -> Option<&str> {
    output.lines().map(str::trim).find(|line| !line.is_empty())
}

/// Like `first_non_empty_line` but skips lines that contain only emoji /

/// Ensure every emoji is followed by at least one space so that wide-character
/// glyphs don't bleed into the next character in terminals that count them as
/// two columns wide.
fn normalize_emoji_spacing(line: &str) -> String {
    let mut out = String::with_capacity(line.len() + 8);
    let mut chars = line.chars().peekable();
    while let Some(ch) = chars.next() {
        out.push(ch);
        if is_emoji(ch) {
            // Skip any variation selectors / zero-width joiners that follow.
            while let Some(&next) = chars.peek() {
                if matches!(next, '\u{FE00}'..='\u{FE0F}' | '\u{200D}' | '\u{20E3}') {
                    out.push(next);
                    chars.next();
                } else {
                    break;
                }
            }
            // Add a space if there isn't one already.
            if chars.peek().is_some_and(|&c| c != ' ') {
                out.push(' ');
            }
        }
    }
    out
}

fn is_emoji(c: char) -> bool {
    matches!(c,
        '\u{1F300}'..='\u{1FAFF}' // Misc Symbols and Pictographs, Emoticons, etc.
        | '\u{2600}'..='\u{27BF}'  // Misc Symbols, Dingbats
        | '\u{2B00}'..='\u{2BFF}'  // Misc Symbols and Arrows
        | '\u{FE00}'..='\u{FE0F}'  // Variation Selectors (treat as part of emoji run)
    )
}


pub(crate) fn shorten_text(s: &str, max_chars: usize) -> String {
    let trimmed = s.trim();
    if trimmed.chars().count() <= max_chars {
        return trimmed.to_string();
    }
    let mut out: String = trimmed.chars().take(max_chars.saturating_sub(1)).collect();
    out.push_str(Symbols::current().ellipsis);
    out
}

fn parse_tool_label(label: &str) -> (&str, Option<&str>) {
    let Some(open) = label.find('(') else {
        return (label, None);
    };
    if !label.ends_with(')') || open == 0 || open + 1 >= label.len() {
        return (label, None);
    }
    let kind = &label[..open];
    let target = &label[open + 1..label.len() - 1];
    (kind, Some(target))
}

pub(crate) fn collapsed_task_event_lines(events: &[Event]) -> Vec<String> {
    let mut lines = Vec::new();
    let mut i = 0usize;

    while i < events.len() {
        if let Event::ToolCall { label, .. } = &events[i] {
            let (kind, target) = parse_tool_label(label);
            if kind == "Read" {
                let mut count = 1usize;
                let mut j = i + 1;
                let mut had_error = false;
                let mut last_target = target.map(str::to_string);

                while j < events.len() {
                    match &events[j] {
                        Event::ToolResult { exit_code, .. } => {
                            had_error |= *exit_code != 0;
                            j += 1;
                        }
                        Event::ToolCall { label, .. } => {
                            let (next_kind, next_target) = parse_tool_label(label);
                            if next_kind != "Read" {
                                break;
                            }
                            count += 1;
                            if let Some(target) = next_target {
                                last_target = Some(target.to_string());
                            }
                            j += 1;
                        }
                        Event::Thinking { .. } => j += 1,
                        Event::NeedsConfirmation { .. }
                        | Event::Final { .. }
                        | Event::UserTask { .. } => break,
                    }
                }

                if count >= 2 {
                    let summary = format!("  {} Reading {count} files... (Ctrl+d 查看详情)", crate::ui::symbols::Symbols::current().bullet);
                    if had_error {
                        lines.push(summary.red().to_string());
                    } else {
                        lines.push(summary.cyan().to_string());
                    }
                    if let Some(target) = last_target {
                        lines.push(
                            format!("    └ {}", shorten_text(&target, 110))
                                .grey()
                                .to_string(),
                        );
                    }
                    i = j;
                    continue;
                }
            }
        }

        lines.extend(format_event_compact(&events[i]));
        i += 1;
    }

    lines
}

// ── Collapse / expand ─────────────────────────────────────────────────────────

pub(crate) fn collapsed_lines(app: &crate::App) -> Vec<String> {
    let summary = app.final_summary.as_deref().unwrap_or("");
    let mut lines = format_event(&Event::UserTask {
        text: app.task.clone(),
    });
    lines.push(String::new());
    lines.extend(collapsed_task_event_lines(&app.task_events));
    if !app.task_events.is_empty() {
        lines.push(String::new());
    }
    lines.extend(format_event(&Event::Final {
        summary: summary.to_string(),
    }));
    lines.push(String::new());
    lines
}

pub(crate) fn expanded_lines(app: &crate::App) -> Vec<String> {
    let summary = app.final_summary.as_deref().unwrap_or("");
    let mut lines = format_event(&Event::UserTask {
        text: app.task.clone(),
    });
    lines.push(String::new());
    for ev in &app.task_events {
        lines.extend(format_event(ev));
    }
    lines.push(String::new());
    lines.extend(format_event(&Event::Final {
        summary: summary.to_string(),
    }));
    lines.push(String::new());
    lines
}

pub(crate) fn toggle_collapse(app: &mut crate::App, screen: &mut super::screen::Screen) {
    if app.final_summary.is_none() {
        return;
    }
    if app.task_collapsed {
        screen.collapse_to(&expanded_lines(app));
        app.task_collapsed = false;
        screen.status = "[Ctrl+d] compact view".grey().to_string();
    } else {
        screen.collapse_to(&collapsed_lines(app));
        app.task_collapsed = true;
        screen.status = "[Ctrl+d] full details".grey().to_string();
    }
    screen.refresh();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_final_keeps_markdown_structure_but_drops_fences() {
        let raw = "## Title\n- **a**\n- `b`\n```bash\nls -la\n```\n1. item";
        let got = sanitize_final_summary_for_tui(raw);
        assert!(got.contains("## Title"));
        assert!(got.contains("- **a**"));
        assert!(got.contains("- `b`"));
        assert!(got.contains("ls -la"));
        assert!(got.contains("1. item"));
        assert!(!got.contains("```"));
    }

    #[test]
    fn final_diff_has_red_green_semantics() {
        let lines = format_final_lines(
            "变更摘要\ndiff --git a/foo b/foo\n@@ -1,2 +1,2 @@\n-old line\n+new line",
        );
        assert!(lines.iter().any(|l| l.contains("-old line")));
        assert!(lines.iter().any(|l| l.contains("+new line")));
    }

    #[test]
    fn collapsed_groups_consecutive_reads() {
        let events = vec![
            Event::ToolCall {
                label: "Read(/tmp/a.rs)".to_string(),
                command: "cat /tmp/a.rs".to_string(),
                multiline: false,
            },
            Event::ToolResult {
                exit_code: 0,
                output: "Read 10 lines".to_string(),
            },
            Event::ToolCall {
                label: "Read(/tmp/b.rs)".to_string(),
                command: "cat /tmp/b.rs".to_string(),
                multiline: false,
            },
            Event::ToolResult {
                exit_code: 0,
                output: "Read 20 lines".to_string(),
            },
            Event::ToolCall {
                label: "Read(/tmp/c.rs)".to_string(),
                command: "cat /tmp/c.rs".to_string(),
                multiline: false,
            },
            Event::ToolResult {
                exit_code: 0,
                output: "Read 30 lines".to_string(),
            },
        ];

        let lines = collapsed_task_event_lines(&events).join("\n");
        assert!(lines.contains("Reading 3 files"));
        assert!(lines.contains("/tmp/c.rs"));
        assert!(!lines.contains("Read(/tmp/a.rs)"));
    }

    #[test]
    fn collapsed_keeps_single_read_detail() {
        let events = vec![
            Event::ToolCall {
                label: "Read(/tmp/only.rs)".to_string(),
                command: "cat /tmp/only.rs".to_string(),
                multiline: false,
            },
            Event::ToolResult {
                exit_code: 0,
                output: "Read 8 lines".to_string(),
            },
        ];

        let lines = collapsed_task_event_lines(&events).join("\n");
        assert!(lines.contains("Read(/tmp/only.rs)"));
        assert!(!lines.contains("Reading 1 files"));
    }
}
