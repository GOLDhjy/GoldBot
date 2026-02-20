mod agent;
mod memory;
mod tools;
mod types;

use std::{
    io::{self, Write},
    path::Path,
    time::Duration,
};

use agent::{
    provider::{Message, Role, build_http_client, chat_stream_with},
    react::{SYSTEM_PROMPT, parse_llm_response},
};
use crossterm::{
    cursor,
    event::{self, Event as CEvent, KeyCode, KeyEventKind, KeyModifiers},
    execute,
    style::{Print, Stylize},
    terminal::{Clear, ClearType, disable_raw_mode, enable_raw_mode},
};
use memory::store::MemoryStore;
use tokio::sync::mpsc;
use tools::safety::{RiskLevel, assess_command};
use types::{Event, LlmAction};
use unicode_width::UnicodeWidthChar;

const MAX_MESSAGES_BEFORE_COMPACTION: usize = 48;
const KEEP_RECENT_MESSAGES_AFTER_COMPACTION: usize = 18;
const MAX_COMPACTION_SUMMARY_ITEMS: usize = 8;

// ── Screen ────────────────────────────────────────────────────────────────────
// The bottom of the terminal is a "managed" area that is redrawn in place.
//
// Normal mode  (managed_lines = 2):
//   ─ status line  : "  ⏳ Thinking…"  or blank
//   ─ input line   : "❯ <text>"
//
// Confirmation mode  (managed_lines = 5):
//   ─ "  ❯ Execute"   (selected)
//   ─ "    Skip"
//   ─ "    Abort"
//   ─ "    Add Note"
//   ─ "❯ ..."         (type note directly, or use ↑/↓ + Enter)
//
// task_lines counts rendered rows currently shown for the active task so that
// collapse_to() can erase and replace them without touching earlier history.
const TITLE_BANNER: [&str; 5] = [
    "   ____       _     _ ____        _   ",
    "  / ___| ___ | | __| | __ )  ___ | |_ ",
    " | |  _ / _ \\| |/ _` |  _ \\ / _ \\| __|",
    " | |_| | (_) | | (_| | |_) | (_) | |_ ",
    "  \\____|\\___/|_|\\__,_|____/ \\___/ \\__|",
];

struct Screen {
    stdout: io::Stdout,
    pub status: String,
    pub input: String,
    task_lines: usize,
    task_rendered: Vec<String>, // raw rendered task lines currently visible above managed area
    managed_lines: usize,       // lines currently on screen in managed area
    pub confirm_selected: Option<usize>, // Some(n) → in confirmation mode, n selected
    pub input_focused: bool,
}

impl Screen {
    fn new() -> io::Result<Self> {
        let mut s = Self {
            stdout: io::stdout(),
            status: String::new(),
            input: String::new(),
            task_lines: 0,
            task_rendered: Vec::new(),
            managed_lines: 2,
            confirm_selected: None,
            input_focused: true,
        };
        // Cargo/launcher output may leave cursor mid-line; force a clean line first.
        execute!(s.stdout, cursor::MoveToColumn(0), Print("\r\n"))?;
        for line in TITLE_BANNER {
            execute!(
                s.stdout,
                cursor::MoveToColumn(0),
                Clear(ClearType::CurrentLine),
                Print(format!("{}\r\n", line.cyan().bold()))
            )?;
        }

        let cols = crossterm::terminal::size()
            .map(|(c, _)| c.max(1) as usize)
            .unwrap_or(80);
        let subtitle_budget = cols.saturating_sub(rendered_text_width("  "));
        for (i, line) in startup_subtitle_lines().iter().enumerate() {
            let line = fit_single_line_tail(line, subtitle_budget);
            let styled = match i {
                0 => line.bold().to_string(),
                1 => line.dark_grey().to_string(),
                _ => line.dark_grey().to_string(),
            };
            execute!(
                s.stdout,
                cursor::MoveToColumn(0),
                Clear(ClearType::CurrentLine),
                Print(format!("  {}\r\n", styled))
            )?;
        }

        execute!(
            s.stdout,
            cursor::MoveToColumn(0),
            Clear(ClearType::CurrentLine),
            Print("\r\n"),
            Print("❯ ")
        )?;
        s.stdout.flush()?;
        Ok(s)
    }

    /// Erase the managed area; cursor ends at col 0 of the first managed line.
    fn clear_managed(&mut self) {
        let up = self.managed_lines.saturating_sub(1).min(u16::MAX as usize) as u16;
        let _ = execute!(
            self.stdout,
            cursor::MoveToColumn(0),
            cursor::MoveUp(up),
            Clear(ClearType::FromCursorDown),
        );
    }

    /// Draw the managed area and update managed_lines to match what was drawn.
    fn draw_managed(&mut self) {
        let cols = crossterm::terminal::size()
            .map(|(c, _)| c.max(1) as usize)
            .unwrap_or(80);
        if let Some(selected) = self.confirm_selected {
            // Vertical confirmation menu (4 options + hint line = 5 lines).
            let labels = ["Execute", "Skip", "Abort", "Add Note"];
            for (i, label) in labels.iter().enumerate() {
                let line = if i == selected {
                    format!("  {} {}\r\n", "❯".cyan().bold(), label.bold().white())
                } else {
                    format!("    {}\r\n", label).dark_grey().to_string()
                };
                let _ = execute!(self.stdout, Print(line));
            }
            let hint = fit_single_line_tail("❯ 直接输入补充说明，或 ↑/↓ 选择后 Enter", cols);
            let _ = execute!(self.stdout, Print(hint.dark_yellow().to_string()));
            self.managed_lines = 5;
        } else {
            // Normal: status + input.
            let status_budget = cols.saturating_sub(rendered_text_width("  "));
            let max_status_lines = if self.status.starts_with("⏳ ") {
                3
            } else {
                1
            };
            let status_lines =
                split_tail_lines_by_width(&self.status, status_budget, max_status_lines);
            let status_rows = if status_lines.is_empty() {
                1
            } else {
                status_lines.len()
            };
            if status_lines.is_empty() {
                let _ = execute!(self.stdout, Print("\r\n"));
            } else {
                for line in status_lines {
                    let _ = execute!(self.stdout, Print(format!("  {}\r\n", line)));
                }
            }
            let input_budget = cols.saturating_sub(rendered_text_width("❯ "));
            let shown_input = fit_single_line_tail(&self.input, input_budget);
            let prompt = if self.input_focused {
                format!("❯ {}", shown_input)
            } else {
                format!("❯ {}", shown_input).dark_grey().to_string()
            };
            let _ = execute!(self.stdout, Print(prompt));
            self.managed_lines = status_rows + 1;
        }
        let _ = self.stdout.flush();
    }

    /// Emit event lines into the scrolling area above, then redraw managed area.
    fn emit(&mut self, lines: &[String]) {
        self.task_lines += lines.iter().map(|l| self.rendered_rows(l)).sum::<usize>();
        self.task_rendered.extend(lines.iter().cloned());
        self.clear_managed();
        for line in lines {
            let _ = execute!(self.stdout, Print(format!("{}\r\n", line)));
        }
        self.draw_managed();
    }

    /// Redraw managed area in place (state changed without new event lines).
    fn refresh(&mut self) {
        self.clear_managed();
        self.draw_managed();
    }

    fn reset_task_lines(&mut self) {
        self.task_lines = 0;
        self.task_rendered.clear();
    }

    fn rendered_rows(&self, line: &str) -> usize {
        let cols = crossterm::terminal::size()
            .map(|(c, _)| c.max(1) as usize)
            .unwrap_or(80);
        let plain = strip_ansi(line);
        let width = rendered_text_width(plain.as_str());
        width.saturating_sub(1) / cols + 1
    }

    /// Erase all task lines + managed area and replace with `kept`.
    fn collapse_to(&mut self, kept: &[String]) {
        let rendered_rows = self
            .task_rendered
            .iter()
            .map(|line| self.rendered_rows(line))
            .sum::<usize>();
        let up = rendered_rows
            .saturating_add(self.managed_lines.saturating_sub(1))
            .min(u16::MAX as usize) as u16;
        let _ = execute!(
            self.stdout,
            cursor::MoveToColumn(0),
            cursor::MoveUp(up),
            Clear(ClearType::FromCursorDown),
        );
        self.task_lines = 0;
        self.task_rendered.clear();
        for line in kept {
            let _ = execute!(self.stdout, Print(format!("{}\r\n", line)));
            self.task_lines += self.rendered_rows(line);
            self.task_rendered.push(line.clone());
        }
        self.draw_managed();
    }
}

fn startup_subtitle_lines() -> Vec<String> {
    let version = env!("CARGO_PKG_VERSION");
    let model =
        std::env::var("ANTHROPIC_DEFAULT_SONNET_MODEL").unwrap_or_else(|_| "GLM-4.7".to_string());
    let base_url = std::env::var("ANTHROPIC_BASE_URL")
        .unwrap_or_else(|_| "https://open.bigmodel.cn/api/anthropic".to_string());
    let provider = extract_host_from_url(&base_url).unwrap_or(base_url);
    let cwd = std::env::current_dir()
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .unwrap_or_else(|_| ".".to_string());

    vec![
        format!("GoldBot v{version}"),
        format!("{model} · {provider}"),
        cwd,
    ]
}

fn extract_host_from_url(url: &str) -> Option<String> {
    let no_scheme = url.split("://").nth(1).unwrap_or(url);
    let host = no_scheme.split('/').next().unwrap_or(no_scheme).trim();
    if host.is_empty() {
        None
    } else {
        Some(host.to_string())
    }
}

fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' && matches!(chars.peek(), Some('[')) {
            let _ = chars.next();
            for c in chars.by_ref() {
                if ('@'..='~').contains(&c) {
                    break;
                }
            }
            continue;
        }
        out.push(ch);
    }
    out
}

fn rendered_text_width(s: &str) -> usize {
    const TAB_STOP: usize = 8;
    let mut col = 0usize;
    for ch in s.chars() {
        match ch {
            '\t' => {
                let advance = TAB_STOP - (col % TAB_STOP);
                col += advance;
            }
            '\r' | '\n' => {}
            c if c.is_control() => {}
            c => col += UnicodeWidthChar::width(c).unwrap_or(0),
        }
    }
    col
}

fn fit_single_line_tail(s: &str, max_width: usize) -> String {
    if max_width == 0 {
        return String::new();
    }

    let plain = strip_ansi(s).replace('\t', " ");
    if rendered_text_width(plain.as_str()) <= max_width {
        return plain;
    }

    const ELLIPSIS: char = '…';
    let ellipsis_width = UnicodeWidthChar::width(ELLIPSIS).unwrap_or(1);
    if max_width <= ellipsis_width {
        return ELLIPSIS.to_string();
    }
    let budget = max_width - ellipsis_width;

    let mut kept_rev: Vec<char> = Vec::new();
    let mut used = 0usize;
    for ch in plain.chars().rev() {
        if ch == '\r' || ch == '\n' || ch.is_control() {
            continue;
        }
        let w = UnicodeWidthChar::width(ch).unwrap_or(0);
        if w == 0 {
            continue;
        }
        if used + w > budget {
            break;
        }
        kept_rev.push(ch);
        used += w;
    }

    kept_rev.reverse();
    let mut out = String::new();
    out.push(ELLIPSIS);
    out.extend(kept_rev);
    out
}

fn split_tail_lines_by_width(s: &str, max_width: usize, max_lines: usize) -> Vec<String> {
    if max_width == 0 || max_lines == 0 {
        return Vec::new();
    }

    let plain = strip_ansi(s).replace('\t', " ");
    let mut wrapped = Vec::new();
    for raw_line in plain.lines() {
        let mut cur = String::new();
        let mut cur_w = 0usize;
        for ch in raw_line.chars() {
            if ch == '\r' || ch.is_control() {
                continue;
            }
            let w = UnicodeWidthChar::width(ch).unwrap_or(0);
            if w == 0 {
                continue;
            }
            if cur_w + w > max_width && !cur.is_empty() {
                wrapped.push(std::mem::take(&mut cur));
                cur_w = 0;
            }
            cur.push(ch);
            cur_w += w;
        }
        if !cur.is_empty() {
            wrapped.push(cur);
        }
    }

    if wrapped.is_empty() {
        return Vec::new();
    }
    if wrapped.len() <= max_lines {
        return wrapped;
    }

    let mut tail = wrapped[wrapped.len() - max_lines..].to_vec();
    if let Some(first) = tail.first_mut() {
        *first = fit_single_line_tail(&format!("…{}", first), max_width);
    }
    tail
}

// ── App ───────────────────────────────────────────────────────────────────────
struct App {
    messages: Vec<Message>,
    task: String,
    steps_taken: usize,
    max_steps: usize,
    llm_calling: bool,
    llm_stream_preview: String, // rolling raw stream buffer
    llm_preview_shown: String,  // last preview rendered in status
    needs_agent_step: bool,
    running: bool,
    quit: bool,
    pending_confirm: Option<String>,
    pending_confirm_note: bool, // typing note for risky command
    task_events: Vec<Event>,    // intermediate events saved for expand
    final_summary: Option<String>,
    task_collapsed: bool,
}

impl App {
    fn new() -> Self {
        let store = MemoryStore::new();
        let system_prompt = store.build_system_prompt(SYSTEM_PROMPT);
        Self {
            messages: vec![Message::system(system_prompt)],
            task: String::new(),
            steps_taken: 0,
            max_steps: 30,
            llm_calling: false,
            llm_stream_preview: String::new(),
            llm_preview_shown: String::new(),
            needs_agent_step: false,
            running: false,
            quit: false,
            pending_confirm: None,
            pending_confirm_note: false,
            task_events: Vec::new(),
            final_summary: None,
            task_collapsed: false,
        }
    }
}

enum LlmWorkerEvent {
    Delta(String),
    Done(anyhow::Result<String>),
}

// ── Event formatting ──────────────────────────────────────────────────────────

fn format_event(event: &Event) -> Vec<String> {
    match event {
        Event::UserTask { text } => lines_with(text, |i, line| {
            if i == 0 {
                format!("❯ {}", line).bold().to_string()
            } else {
                format!("  {}", line)
            }
        }),
        Event::Thinking { text } => lines_with(text, |_, line| {
            format!("  {}", line).dark_grey().to_string()
        }),
        Event::ToolCall { label, command } => {
            let mut lines = vec![format!("  ⏺ {}", label).cyan().to_string()];
            lines.extend(lines_with(command, |_, line| {
                format!("    {}", line).dark_grey().to_string()
            }));
            lines
        }
        Event::ToolResult { output, exit_code } => {
            let ok = *exit_code == 0;
            lines_with(output, |i, line| {
                let pfx = if i == 0 { "  ⎿ " } else { "    " };
                let s = format!("{}{}", pfx, line);
                if ok {
                    s.dark_grey().to_string()
                } else {
                    s.red().to_string()
                }
            })
        }
        Event::NeedsConfirmation { command, .. } => {
            let first = command.lines().next().unwrap_or(command.as_str());
            let cmd_display = if command.lines().count() > 1 {
                format!("{} …", first)
            } else {
                first.to_string()
            };
            let mut lines = vec![
                "  ⚠ 需要确认".dark_yellow().to_string(),
                format!("    {}", cmd_display).bold().cyan().to_string(),
            ];
            if command_contains_heredoc(command) {
                lines.push(
                    "    (EOF 是 Here-doc 的结束标记，表示接下来是多行写入内容)"
                        .dark_grey()
                        .to_string(),
                );
            }
            lines
        }
        Event::Final { summary } => format_final_lines(summary),
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
                return "    ─────────────────────────────".dark_grey().to_string();
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
            if let Some(rest) = trimmed.strip_prefix("> ") {
                return format!("    {}", render_inline_markdown(rest))
                    .dark_grey()
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

fn is_markdown_rule(line: &str) -> bool {
    let t = line.trim();
    t.len() >= 3 && t.chars().all(|c| matches!(c, '-' | '*' | '_'))
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
    if let Some((key, sep, value)) = split_key_value_parts(item) {
        let key = render_inline_markdown(key);
        let value = render_inline_markdown(value);
        return format!(
            "    {} {}{} {}",
            "•".dark_grey(),
            key.bold().yellow(),
            sep,
            value
        );
    }
    format!("    {} {}", "•".dark_grey(), render_inline_markdown(item))
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

        out.push(chars[i]);
        i += 1;
    }

    out
}

fn sanitize_final_summary_for_tui(text: &str) -> String {
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
            raw.trim().to_string()
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

fn format_event_live(event: &Event) -> Vec<String> {
    match event {
        Event::UserTask { .. } | Event::Final { .. } => format_event(event),
        Event::Thinking { text } => {
            let line = first_non_empty_line(text).unwrap_or("");
            vec![
                format!("  {}", shorten_text(line, 110))
                    .dark_grey()
                    .to_string(),
            ]
        }
        Event::ToolCall { label, command } => {
            let first = command.lines().next().unwrap_or(command.as_str());
            vec![
                format!("  ⏺ {}", label).cyan().to_string(),
                format!("    {}", shorten_text(first, 120))
                    .dark_grey()
                    .to_string(),
            ]
        }
        Event::ToolResult { output, exit_code } => compact_tool_result_lines(*exit_code, output),
        Event::NeedsConfirmation { .. } => format_event(event),
    }
}

fn emit_live_event(screen: &mut Screen, event: &Event) {
    screen.emit(&format_event_live(event));
}

#[derive(Default)]
struct FsChangeSummary {
    created: Vec<String>,
    updated: Vec<String>,
    deleted: Vec<String>,
}

fn format_event_compact(event: &Event) -> Vec<String> {
    match event {
        Event::Thinking { .. } => Vec::new(),
        Event::ToolCall { label, .. } => vec![format!("  • {}", label).cyan().to_string()],
        Event::ToolResult { output, exit_code } => compact_tool_result_lines(*exit_code, output),
        Event::NeedsConfirmation { command, .. } => {
            let first = command.lines().next().unwrap_or(command.as_str());
            let mut lines = vec![
                "    ⚠ 需要确认".dark_yellow().to_string(),
                format!("      {}", shorten_text(first, 72))
                    .bold()
                    .cyan()
                    .to_string(),
            ];
            if command_contains_heredoc(command) {
                lines.push("      (EOF = Here-doc 结束标记)".dark_grey().to_string());
            }
            lines
        }
        Event::Final { .. } | Event::UserTask { .. } => Vec::new(),
    }
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
                line.dark_grey().to_string()
            } else {
                line.red().to_string()
            }
        })
        .collect()
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

fn collapsed_task_event_lines(events: &[Event]) -> Vec<String> {
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
                                .dark_grey()
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

fn first_non_empty_line(output: &str) -> Option<&str> {
    output.lines().map(str::trim).find(|line| !line.is_empty())
}

fn command_contains_heredoc(command: &str) -> bool {
    command.contains("<<")
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

fn shorten_text(s: &str, max_chars: usize) -> String {
    let trimmed = s.trim();
    if trimmed.chars().count() <= max_chars {
        return trimmed.to_string();
    }
    let mut out: String = trimmed.chars().take(max_chars.saturating_sub(1)).collect();
    out.push('…');
    out
}

// ── Collapse / expand ─────────────────────────────────────────────────────────

fn collapsed_lines(app: &App) -> Vec<String> {
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

fn expanded_lines(app: &App) -> Vec<String> {
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

fn toggle_collapse(app: &mut App, screen: &mut Screen) {
    if app.final_summary.is_none() {
        return;
    }
    if app.task_collapsed {
        screen.collapse_to(&expanded_lines(app));
        app.task_collapsed = false;
        screen.status = "[Ctrl+d] compact view".dark_grey().to_string();
    } else {
        screen.collapse_to(&collapsed_lines(app));
        app.task_collapsed = true;
        screen.status = "[Ctrl+d] full details".dark_grey().to_string();
    }
    screen.refresh();
}

fn maybe_flush_and_compact_before_call(app: &mut App, screen: &mut Screen) {
    if app.messages.len() <= MAX_MESSAGES_BEFORE_COMPACTION {
        return;
    }
    if app.messages.len() <= KEEP_RECENT_MESSAGES_AFTER_COMPACTION + 1 {
        return;
    }

    let split_at = app
        .messages
        .len()
        .saturating_sub(KEEP_RECENT_MESSAGES_AFTER_COMPACTION);
    if split_at <= 1 {
        return;
    }

    let older: Vec<Message> = app.messages[1..split_at].to_vec();
    let store = MemoryStore::new();
    let mut flushed = 0usize;
    let mut last_user_task: Option<String> = None;

    for msg in &older {
        match msg.role {
            Role::User => {
                if msg.content.starts_with("Tool result")
                    || msg
                        .content
                        .starts_with("Your last response could not be parsed")
                    || msg.content.starts_with("[Context compacted]")
                {
                    continue;
                }
                last_user_task = Some(msg.content.clone());
            }
            Role::Assistant => {
                if let Some(final_text) = extract_last_tag_text(&msg.content, "final") {
                    if let Some(task) = last_user_task.as_deref() {
                        for note in store.derive_long_term_notes(task, &final_text) {
                            if let Ok(true) = store.append_long_term_if_new(&note) {
                                flushed += 1;
                            }
                        }
                    }
                }
            }
            Role::System => {}
        }
    }

    let summary = summarize_for_compaction(&older);
    let mut compacted = Vec::new();
    compacted.push(app.messages[0].clone());
    if !summary.is_empty() {
        compacted.push(Message::user(format!("[Context compacted]\n{summary}")));
    }
    compacted.extend_from_slice(&app.messages[split_at..]);
    app.messages = compacted;

    screen.status = if flushed > 0 {
        format!("🧠 pre-compaction flush: {flushed} long-term notes")
            .dark_grey()
            .to_string()
    } else {
        "🧠 context compacted".dark_grey().to_string()
    };
    screen.refresh();
}

fn summarize_for_compaction(messages: &[Message]) -> String {
    let mut items = Vec::new();
    for msg in messages.iter().rev() {
        match msg.role {
            Role::User => {
                if msg.content.starts_with("Tool result")
                    || msg
                        .content
                        .starts_with("Your last response could not be parsed")
                    || msg.content.starts_with("[Context compacted]")
                {
                    continue;
                }
                let one_line = msg.content.split_whitespace().collect::<Vec<_>>().join(" ");
                items.push(format!("- user: {}", shorten_text(&one_line, 120)));
            }
            Role::Assistant => {
                if let Some(final_text) = extract_last_tag_text(&msg.content, "final") {
                    let one_line = final_text.split_whitespace().collect::<Vec<_>>().join(" ");
                    items.push(format!("- final: {}", shorten_text(&one_line, 120)));
                }
            }
            Role::System => {}
        }
        if items.len() >= MAX_COMPACTION_SUMMARY_ITEMS {
            break;
        }
    }
    items.reverse();
    items.join("\n")
}

fn extract_last_tag_text(text: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let end = text.rfind(&close)?;
    let head = &text[..end];
    let start = head.rfind(&open)? + open.len();
    Some(head[start..].trim().to_string())
}

// ── Entry point ───────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let _ = dotenvy::dotenv();
    let http_client = build_http_client()?;
    let mut app = App::new();

    enable_raw_mode()?;
    let mut screen = Screen::new()?;

    let run_result = run_loop(&mut app, &mut screen, http_client).await;

    let _ = disable_raw_mode();
    let _ = execute!(io::stdout(), Print("\r\n"));
    run_result
}

async fn run_loop(
    app: &mut App,
    screen: &mut Screen,
    http_client: reqwest::Client,
) -> anyhow::Result<()> {
    if let Ok(task) = std::env::var("GOLDBOT_TASK") {
        start_task(app, screen, task);
    }

    let (tx, mut rx) = mpsc::channel::<LlmWorkerEvent>(64);

    loop {
        while let Ok(msg) = rx.try_recv() {
            match msg {
                LlmWorkerEvent::Delta(delta) => handle_llm_stream_delta(app, screen, &delta),
                LlmWorkerEvent::Done(result) => {
                    app.llm_calling = false;
                    app.llm_stream_preview.clear();
                    app.llm_preview_shown.clear();
                    screen.status.clear();
                    process_llm_result(app, screen, result);
                }
            }
        }

        if app.running && app.pending_confirm.is_none() && app.needs_agent_step && !app.llm_calling
        {
            maybe_flush_and_compact_before_call(app, screen);
            app.needs_agent_step = false;
            app.llm_calling = true;
            app.llm_stream_preview.clear();
            app.llm_preview_shown.clear();
            screen.status = "⏳ Thinking...".to_string();
            screen.refresh();

            let tx_done = tx.clone();
            let tx_delta = tx.clone();
            let client = http_client.clone();
            let messages = app.messages.clone();
            tokio::spawn(async move {
                let result = chat_stream_with(&client, &messages, |piece| {
                    let _ = tx_delta.try_send(LlmWorkerEvent::Delta(piece.to_string()));
                })
                .await;
                let _ = tx_done.send(LlmWorkerEvent::Done(result)).await;
            });
        }

        if event::poll(Duration::from_millis(50))? {
            match event::read()? {
                CEvent::Key(key) if key.kind == KeyEventKind::Press => {
                    if handle_key(app, screen, key.code, key.modifiers) {
                        break;
                    }
                }
                _ => {}
            }
        }

        if app.quit {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    Ok(())
}

fn handle_llm_stream_delta(app: &mut App, screen: &mut Screen, delta: &str) {
    if !app.llm_calling || delta.is_empty() {
        return;
    }

    app.llm_stream_preview.push_str(delta);
    trim_left_to_max_bytes(&mut app.llm_stream_preview, 16_384);

    let preview = extract_live_preview(&app.llm_stream_preview);
    if preview.is_empty() {
        return;
    }

    let punctuation_flush = preview.ends_with(['。', '！', '？', '.', '!', '?', ';', '；']);
    let grew_by = preview
        .chars()
        .count()
        .saturating_sub(app.llm_preview_shown.chars().count());
    let should_refresh = app.llm_preview_shown.is_empty()
        || preview.chars().count() < app.llm_preview_shown.chars().count()
        || !preview.starts_with(&app.llm_preview_shown)
        || grew_by >= 8
        || punctuation_flush;
    if !should_refresh {
        return;
    }

    app.llm_preview_shown = preview.clone();
    screen.status = format!("⏳ {}", preview);
    screen.refresh();
}

fn extract_live_preview(raw: &str) -> String {
    let mut s = if let Some(start) = raw.rfind("<thought>") {
        &raw[start + "<thought>".len()..]
    } else {
        raw
    };
    if let Some(end) = s.rfind("</thought>") {
        s = &s[..end];
    }

    let no_tags = strip_xml_tags(s);
    let collapsed = no_tags.split_whitespace().collect::<Vec<_>>().join(" ");
    tail_chars(&collapsed, 240)
}

fn strip_xml_tags(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for ch in s.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(ch),
            _ => {}
        }
    }
    out
}

fn tail_chars(s: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }
    let count = s.chars().count();
    if count <= max_chars {
        return s.to_string();
    }
    s.chars().skip(count - max_chars).collect()
}

fn trim_left_to_max_bytes(s: &mut String, max_bytes: usize) {
    if s.len() <= max_bytes {
        return;
    }

    let mut cut = s.len().saturating_sub(max_bytes);
    while cut < s.len() && !s.is_char_boundary(cut) {
        cut += 1;
    }
    s.drain(..cut);
}

// ── Key handling ──────────────────────────────────────────────────────────────

fn handle_key(app: &mut App, screen: &mut Screen, key: KeyCode, modifiers: KeyModifiers) -> bool {
    if key == KeyCode::Char('c') && modifiers.contains(KeyModifiers::CONTROL) {
        return true;
    }
    if key == KeyCode::Char('d')
        && modifiers.contains(KeyModifiers::CONTROL)
        && !app.running
        && app.final_summary.is_some()
        && screen.confirm_selected.is_none()
        && !app.pending_confirm_note
    {
        toggle_collapse(app, screen);
        return false;
    }

    if screen.confirm_selected.is_some() {
        // ── Confirmation mode: ↑/↓ navigate, Enter confirm, or type note ─────
        let sel = screen.confirm_selected.unwrap();
        match key {
            KeyCode::Up => {
                screen.confirm_selected = Some(sel.saturating_sub(1));
                screen.refresh();
            }
            KeyCode::Down => {
                screen.confirm_selected = Some((sel + 1).min(3));
                screen.refresh();
            }
            KeyCode::Enter => {
                match sel {
                    0 => {
                        // Execute
                        screen.confirm_selected = None;
                        screen.input_focused = true;
                        app.pending_confirm_note = false;
                        let Some(cmd) = app.pending_confirm.take() else {
                            screen.refresh();
                            return false;
                        };
                        execute_command(app, screen, &cmd);
                        app.needs_agent_step = true;
                    }
                    1 => {
                        // Skip
                        screen.confirm_selected = None;
                        screen.input_focused = true;
                        app.pending_confirm_note = false;
                        let Some(cmd) = app.pending_confirm.take() else {
                            screen.refresh();
                            return false;
                        };
                        let msg = format!("User chose to skip this command: {cmd}");
                        app.messages
                            .push(Message::user(format!("Tool result:\n{msg}")));
                        let ev = Event::ToolResult {
                            exit_code: 0,
                            output: msg,
                        };
                        emit_live_event(screen, &ev);
                        app.task_events.push(ev);
                        app.needs_agent_step = true;
                    }
                    2 => {
                        // Abort
                        screen.confirm_selected = None;
                        app.pending_confirm_note = false;
                        app.pending_confirm = None;
                        finish(app, screen, "Task aborted by user".to_string());
                    }
                    _ => begin_confirm_note_mode(app, screen, None),
                }
            }
            KeyCode::Char(c) if modifiers.is_empty() || modifiers == KeyModifiers::SHIFT => {
                begin_confirm_note_mode(app, screen, Some(c))
            }
            _ => {}
        }
    } else if app.pending_confirm_note {
        // ── Note mode: user adds extra instruction before executing risky cmd ──
        match key {
            KeyCode::Enter => {
                let note = screen.input.trim().to_string();
                if note.is_empty() {
                    exit_confirm_note_mode(app, screen, true);
                    return false;
                }

                let pending_cmd = app.pending_confirm.clone().unwrap_or_default();
                app.messages.push(Message::user(format!(
                    "User rejected the pending risky command and added instruction:\n{note}\nPending command was:\n{pending_cmd}"
                )));
                let ev = Event::Thinking {
                    text: format!("User note: {note}"),
                };
                emit_live_event(screen, &ev);
                app.task_events.push(ev);

                app.pending_confirm = None;
                app.pending_confirm_note = false;
                app.needs_agent_step = true;
                screen.status.clear();
                screen.input.clear();
                screen.input_focused = true;
                screen.refresh();
            }
            KeyCode::Esc if modifiers.is_empty() => exit_confirm_note_mode(app, screen, true),
            KeyCode::Backspace => {
                screen.input.pop();
                screen.refresh();
            }
            KeyCode::Char(c) if modifiers.is_empty() || modifiers == KeyModifiers::SHIFT => {
                screen.input.push(c);
                screen.refresh();
            }
            _ => {}
        }
    } else if !app.running {
        // ── Idle / input mode ─────────────────────────────────────────────────
        if screen.input_focused {
            match key {
                KeyCode::Enter => {
                    let task = screen.input.trim().to_string();
                    if !task.is_empty() {
                        screen.input.clear();
                        start_task(app, screen, task);
                    }
                }
                KeyCode::Esc if modifiers.is_empty() => {
                    screen.input_focused = false;
                    screen.refresh();
                }
                KeyCode::Char(c) if modifiers.is_empty() || modifiers == KeyModifiers::SHIFT => {
                    screen.input.push(c);
                    screen.refresh();
                }
                KeyCode::Backspace => {
                    screen.input.pop();
                    screen.refresh();
                }
                _ => {}
            }
        } else {
            match key {
                KeyCode::Char('i') if modifiers.is_empty() => {
                    screen.input_focused = true;
                    screen.refresh();
                }
                KeyCode::Esc if modifiers.is_empty() => return true,
                KeyCode::Char(c) if modifiers.is_empty() || modifiers == KeyModifiers::SHIFT => {
                    screen.input_focused = true;
                    screen.input.push(c);
                    screen.refresh();
                }
                KeyCode::Backspace => {
                    screen.input_focused = true;
                    screen.refresh();
                }
                _ => {}
            }
        }
    } else {
        // ── Running (LLM in flight) ───────────────────────────────────────────
        // Let user pre-type the next input while current loop is running.
        match key {
            KeyCode::Char(c) if modifiers.is_empty() || modifiers == KeyModifiers::SHIFT => {
                screen.input_focused = true;
                screen.input.push(c);
                screen.refresh();
            }
            KeyCode::Backspace => {
                screen.input_focused = true;
                screen.input.pop();
                screen.refresh();
            }
            _ => {}
        }
    }

    false
}

fn begin_confirm_note_mode(app: &mut App, screen: &mut Screen, first_char: Option<char>) {
    if app.pending_confirm.is_none() {
        screen.refresh();
        return;
    }

    app.pending_confirm_note = true;
    screen.confirm_selected = None;
    screen.input_focused = true;
    screen.status = "✍ 输入补充说明后按 Enter；Esc 返回确认菜单"
        .dark_yellow()
        .to_string();
    screen.input.clear();
    if let Some(c) = first_char {
        screen.input.push(c);
    }
    screen.refresh();
}

fn exit_confirm_note_mode(app: &mut App, screen: &mut Screen, back_to_menu: bool) {
    app.pending_confirm_note = false;
    screen.input.clear();
    screen.status.clear();
    if back_to_menu && app.pending_confirm.is_some() {
        screen.confirm_selected = Some(0);
        screen.input_focused = false;
    } else {
        screen.confirm_selected = None;
        screen.input_focused = true;
    }
    screen.refresh();
}

// ── Task lifecycle ────────────────────────────────────────────────────────────

fn start_task(app: &mut App, screen: &mut Screen, task: String) {
    if app.messages.len() > 1 {
        screen.emit(&[String::new()]); // blank separator (before reset, not counted)
    }
    screen.reset_task_lines();

    app.task = task.clone();
    app.steps_taken = 0;
    app.running = true;
    app.llm_stream_preview.clear();
    app.llm_preview_shown.clear();
    app.needs_agent_step = true;
    app.pending_confirm = None;
    app.pending_confirm_note = false;
    screen.confirm_selected = None;
    screen.input_focused = true;
    app.task_events.clear();
    app.final_summary = None;
    app.task_collapsed = false;
    app.messages.push(Message::user(task.clone()));

    emit_live_event(screen, &Event::UserTask { text: task });
}

fn process_llm_result(app: &mut App, screen: &mut Screen, result: anyhow::Result<String>) {
    if app.steps_taken >= app.max_steps {
        finish(
            app,
            screen,
            format!("Reached max steps ({}).", app.max_steps),
        );
        return;
    }
    app.steps_taken += 1;

    let response = match result {
        Ok(r) => r,
        Err(e) => {
            let ev = Event::Thinking {
                text: format!("[LLM error] {e}"),
            };
            emit_live_event(screen, &ev);
            app.task_events.push(ev);
            app.running = false;
            return;
        }
    };

    let (thought, action) = match parse_llm_response(&response) {
        Ok(parsed) => parsed,
        Err(e) => {
            app.messages.push(Message::assistant(response));
            app.messages.push(Message::user(
                "Your last response could not be parsed. \
                 Please reply with exactly:\n\
                 <thought>...</thought><tool>shell</tool><command>...</command>\n\
                 or:\n\
                 <thought>...</thought><final>...</final>"
                    .to_string(),
            ));
            screen.status = format!("↻ Retrying invalid response format: {e}")
                .dark_grey()
                .to_string();
            screen.refresh();
            app.needs_agent_step = true;
            return;
        }
    };

    if !thought.is_empty() {
        let ev = Event::Thinking { text: thought };
        emit_live_event(screen, &ev);
        app.task_events.push(ev);
    }
    app.messages.push(Message::assistant(response));

    match action {
        LlmAction::Shell { command } => {
            let (risk, reason) = assess_command(&command);
            match risk {
                RiskLevel::Safe => {
                    execute_command(app, screen, &command);
                    app.needs_agent_step = true;
                }
                RiskLevel::Confirm => {
                    let ev = Event::NeedsConfirmation {
                        command: command.clone(),
                        reason,
                    };
                    emit_live_event(screen, &ev);
                    app.task_events.push(ev);
                    app.pending_confirm = Some(command);
                    app.pending_confirm_note = false;
                    screen.confirm_selected = Some(0);
                    screen.input_focused = false;
                    screen.refresh();
                }
                RiskLevel::Block => {
                    let msg = "Command blocked by safety policy";
                    app.messages
                        .push(Message::user(format!("Tool result:\n{msg}")));
                    let ev = Event::ToolResult {
                        exit_code: -1,
                        output: msg.to_string(),
                    };
                    emit_live_event(screen, &ev);
                    app.task_events.push(ev);
                    app.needs_agent_step = true;
                }
            }
        }
        LlmAction::Final { summary } => {
            finish(app, screen, summary);
        }
    }
}

fn execute_command(app: &mut App, screen: &mut Screen, cmd: &str) {
    let intent = tools::shell::classify_command(cmd);
    let call_ev = Event::ToolCall {
        label: intent.label(),
        command: cmd.to_string(),
    };
    emit_live_event(screen, &call_ev);
    app.task_events.push(call_ev);

    match tools::shell::run_command(cmd) {
        Ok(out) => {
            app.messages.push(Message::user(format!(
                "Tool result (exit={}):\n{}",
                out.exit_code, out.output
            )));
            let ev = Event::ToolResult {
                exit_code: out.exit_code,
                output: out.output,
            };
            emit_live_event(screen, &ev);
            app.task_events.push(ev);
        }
        Err(e) => {
            let err = format!("execution failed: {e}");
            app.messages
                .push(Message::user(format!("Tool result (exit=-1):\n{err}")));
            let ev = Event::ToolResult {
                exit_code: -1,
                output: err,
            };
            emit_live_event(screen, &ev);
            app.task_events.push(ev);
        }
    }
}

fn finish(app: &mut App, screen: &mut Screen, summary: String) {
    let summary = sanitize_final_summary_for_tui(&summary);
    app.final_summary = Some(summary.clone());
    app.task_collapsed = true;

    screen.collapse_to(&collapsed_lines(app));

    let store = MemoryStore::new();
    let _ = store.append_short_term(&app.task, &summary);
    for note in store.derive_long_term_notes(&app.task, &summary) {
        let _ = store.append_long_term_if_new(&note);
    }
    let _ = store.promote_repeated_short_term_to_long_term();

    app.running = false;
    app.llm_stream_preview.clear();
    app.llm_preview_shown.clear();
    app.pending_confirm = None;
    app.pending_confirm_note = false;
    screen.confirm_selected = None;
    screen.input_focused = true;
    screen.status = "[Ctrl+d] full details".dark_grey().to_string();
    screen.refresh();
}

#[cfg(test)]
mod tests {
    use super::{collapsed_task_event_lines, format_final_lines, sanitize_final_summary_for_tui};
    use crate::types::Event;

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
