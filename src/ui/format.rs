use std::path::Path;

use crossterm::style::Stylize;

use crate::types::Event;

pub(crate) fn format_event(event: &Event) -> Vec<String> {
    match event {
        Event::UserTask { text } => lines_with(text, |i, line| {
            if i == 0 {
                format!("❯ {}", line).bold().to_string()
            } else {
                format!("  {}", line)
            }
        }),
        Event::Thinking { text } => lines_with(text, |_, line| {
            format!("  {}", line).grey().to_string()
        }),
        Event::ToolCall { label, command } => {
            let mut lines = vec![format!("  ⏺ {}", label).cyan().to_string()];
            lines.extend(lines_with(command, |_, line| {
                format!("    {}", line).grey().to_string()
            }));
            lines
        }
        Event::ToolResult { output, exit_code } => {
            let ok = *exit_code == 0;
            lines_with(output, |i, line| {
                let pfx = if i == 0 { "  ⎿ " } else { "    " };
                let s = format!("{}{}", pfx, line);
                if !ok {
                    s.red().to_string()
                } else {
                    style_tool_result_line(line, s)
                }
            })
        }
        Event::NeedsConfirmation { command, reason } => {
            let mut lines = vec![
                format!("  ⏺ {}", reason).cyan().bold().to_string(),
                "  ⚠ 需要确认".dark_yellow().to_string(),
            ];
            for line in command.lines().take(6) {
                lines.push(format!("    {}", line).cyan().to_string());
            }
            if command.lines().count() > 6 {
                lines.push("    …".grey().to_string());
            }
            lines
        }
        Event::Final { summary } => format_final_lines(summary),
    }
}

pub(crate) fn format_event_live(event: &Event) -> Vec<String> {
    match event {
        Event::UserTask { .. } | Event::Final { .. } => format_event(event),
        Event::Thinking { text } => {
            let line = first_meaningful_line(text).unwrap_or("");
            vec![
                format!("  {}", shorten_text(line, 110))
                    .grey()
                    .to_string(),
            ]
        }
        Event::ToolCall { label, command } => {
            let first = command.lines().next().unwrap_or(command.as_str());
            vec![
                format!("  ⏺ {}", label).cyan().to_string(),
                format!("    {}", shorten_text(first, 120))
                    .grey()
                    .to_string(),
            ]
        }
        Event::ToolResult { output, exit_code } => compact_tool_result_lines(*exit_code, output),
        Event::NeedsConfirmation { .. } => format_event(event),
    }
}

pub(crate) fn emit_live_event(screen: &mut super::screen::Screen, event: &Event) {
    screen.emit(&format_event_live(event));
}

pub(crate) fn format_event_compact(event: &Event) -> Vec<String> {
    match event {
        Event::Thinking { .. } => Vec::new(),
        Event::ToolCall { label, .. } => vec![format!("  • {}", label).cyan().to_string()],
        Event::ToolResult { output, exit_code } => compact_tool_result_lines(*exit_code, output),
        Event::NeedsConfirmation { command, reason } => {
            let mut lines = vec![
                format!("  ⏺ {}", reason).cyan().bold().to_string(),
                "    ⚠ 需要确认".dark_yellow().to_string(),
            ];
            for line in command.lines().take(4) {
                lines.push(format!("      {}", shorten_text(line, 72)).cyan().to_string());
            }
            if command.lines().count() > 4 {
                lines.push("      …".grey().to_string());
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
    lines
        .iter()
        .enumerate()
        .map(|(i, line)| {
            let trimmed = line.trim_start();
            if i == 0 {
                return format!(
                    "  {} {}",
                    "✓".green().bold(),
                    render_inline_markdown(trimmed).bold()
                );
            }

            let rendered = format!("    {}", line);

            if has_diff {
                if trimmed.starts_with('+') && !trimmed.starts_with("+++") {
                    return rendered.green().to_string();
                }
                if trimmed.starts_with('-') && !trimmed.starts_with("---") {
                    return rendered.red().to_string();
                }
                if trimmed.starts_with("@@")
                    || trimmed.starts_with("diff --git")
                    || trimmed.starts_with("index ")
                    || trimmed.starts_with("--- ")
                    || trimmed.starts_with("+++ ")
                {
                    return rendered.dark_yellow().to_string();
                }
            }

            if is_markdown_rule(trimmed) {
                return "    ─────────────────────────────".grey().to_string();
            }
            if let Some((level, heading)) = parse_markdown_heading(trimmed) {
                let rendered = format!("    {}", render_inline_markdown(heading));
                return match level {
                    1 => rendered.bold().green().to_string(),
                    2 => rendered.bold().yellow().to_string(),
                    _ => rendered.bold().dark_yellow().to_string(),
                };
            }
            if let Some(item) = parse_markdown_list_item(trimmed) {
                return format_bullet_line(item);
            }
            if is_markdown_table_separator(trimmed) {
                return "    ─────────────────────────────".grey().to_string();
            }
            if is_markdown_table_row(trimmed) {
                return format_table_row(trimmed);
            }
            if let Some(rest) = trimmed.strip_prefix("> ") {
                return format!("    {}", render_inline_markdown(rest))
                    .grey()
                    .to_string();
            }
            if let Some(section) = split_trailing_section_title(trimmed) {
                return format!("    {}", render_inline_markdown(section))
                    .bold()
                    .yellow()
                    .to_string();
            }
            if let Some((key, sep, value)) = split_key_value_parts(trimmed) {
                let key = render_inline_markdown(key);
                let value = render_inline_markdown(value);
                return format!("    {}{} {}", key.bold().yellow(), sep, value);
            }

            format!("    {}", render_inline_markdown(trimmed))
        })
        .collect()
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
            "•".grey(),
            key.bold().yellow(),
            sep,
            value
        );
    }
    format!("    {} {}", "•".grey(), render_inline_markdown(item))
}

fn style_tool_result_line(raw_line: &str, rendered: String) -> String {
    let t = raw_line.trim_start();
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
    if t.starts_with("Compare ") || t.starts_with("Preview ") {
        return rendered.dark_yellow().to_string();
    }
    if t == "Before:" {
        return rendered.dark_red().to_string();
    }
    if t == "After:" {
        return rendered.dark_green().to_string();
    }
    rendered.grey().to_string()
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
    let mut raw = Vec::new();
    if let Some(fs) = parse_fs_changes(output) {
        if !fs.created.is_empty() {
            raw.push(format!("    ⎿ created: {}", summarize_paths(&fs.created)));
        }
        if !fs.updated.is_empty() {
            raw.push(format!("    ⎿ updated: {}", summarize_paths(&fs.updated)));
        }
        if !fs.deleted.is_empty() {
            raw.push(format!("    ⎿ deleted: {}", summarize_paths(&fs.deleted)));
        }
    }

    if raw.is_empty() {
        if let Some(line) = first_non_empty_line(output) {
            raw.push(format!("    ⎿ {}", shorten_text(line, 110)));
        } else {
            raw.push("    ⎿ (no output)".to_string());
        }
    }

    raw.into_iter()
        .map(|line| {
            if exit_code == 0 {
                line.grey().to_string()
            } else {
                line.red().to_string()
            }
        })
        .collect()
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

        if t.starts_with("Preview ") {
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
/// punctuation and no CJK / Latin letters — avoids showing a lone emoji as
/// the entire live-thinking preview.
fn first_meaningful_line(output: &str) -> Option<&str> {
    output.lines().map(str::trim).find(|line| {
        if line.is_empty() {
            return false;
        }
        // Accept the line only if it has at least one letter or CJK character.
        line.chars().any(|c| c.is_alphabetic())
    })
}

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
    out.push('…');
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
                    let summary = format!("  • Reading {count} files... (Ctrl+d 查看详情)");
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
            },
            Event::ToolResult {
                exit_code: 0,
                output: "Read 10 lines".to_string(),
            },
            Event::ToolCall {
                label: "Read(/tmp/b.rs)".to_string(),
                command: "cat /tmp/b.rs".to_string(),
            },
            Event::ToolResult {
                exit_code: 0,
                output: "Read 20 lines".to_string(),
            },
            Event::ToolCall {
                label: "Read(/tmp/c.rs)".to_string(),
                command: "cat /tmp/c.rs".to_string(),
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
