use std::io::{self, Write};

use crossterm::{
    cursor, execute,
    style::{Print, Stylize},
    terminal::{Clear, ClearType},
};
use unicode_width::UnicodeWidthChar;

use crate::types::{TodoItem, TodoStatus};

pub(crate) const TITLE_BANNER: [&str; 5] = [
    "   ____       _     _ ____        _   ",
    "  / ___| ___ | | __| | __ )  ___ | |_ ",
    " | |  _ / _ \\| |/ _` |  _ \\ / _ \\| __|",
    " | |_| | (_) | | (_| | |_) | (_) | |_ ",
    "  \\____|\\___/|_|\\__,_|____/ \\___/ \\__|",
];

pub(crate) struct Screen {
    stdout: io::Stdout,
    pub status: String,
    pub input: String,
    pub task_lines: usize,
    pub task_rendered: Vec<String>,
    pub managed_lines: usize,
    pub confirm_selected: Option<usize>,
    pub input_focused: bool,
    /// When non-empty, the confirm menu renders these labels instead of the hardcoded Execute/Skip/Abort/Add Note.
    pub question_labels: Vec<String>,
    /// Active todo progress panel items.
    pub todo_items: Vec<TodoItem>,
}

impl Screen {
    pub(crate) fn new() -> io::Result<Self> {
        let mut s = Self {
            stdout: io::stdout(),
            status: String::new(),
            input: String::new(),
            task_lines: 0,
            task_rendered: Vec::new(),
            managed_lines: 2,
            confirm_selected: None,
            input_focused: true,
            question_labels: Vec::new(),
            todo_items: Vec::new(),
        };
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
                1 => line.grey().to_string(),
                _ => line.grey().to_string(),
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

    pub(crate) fn clear_managed(&mut self) {
        let up = self.managed_lines.saturating_sub(1).min(u16::MAX as usize) as u16;
        let _ = execute!(
            self.stdout,
            cursor::MoveToColumn(0),
            cursor::MoveUp(up),
            Clear(ClearType::FromCursorDown),
        );
    }

    pub(crate) fn draw_managed(&mut self) {
        let cols = crossterm::terminal::size()
            .map(|(c, _)| c.max(1) as usize)
            .unwrap_or(80);

        // ── Todo panel (topmost in managed area) ──
        let todo_rows = self.draw_todo_panel(cols);

        if let Some(selected) = self.confirm_selected {
            let (labels, hint): (&[&str], &str) = if self.question_labels.is_empty() {
                (
                    &["Execute", "Skip", "Abort", "Add Note"],
                    "❯ 直接输入补充说明，或 ↑/↓ 选择后 Enter",
                )
            } else {
                (&[], "❯ ↑/↓ 选择，Enter 确认，或直接输入自定义内容")
            };

            let display_labels: Vec<String> = if self.question_labels.is_empty() {
                labels.iter().map(|s| s.to_string()).collect()
            } else {
                self.question_labels.clone()
            };

            for (i, label) in display_labels.iter().enumerate() {
                let num = format!("{}.", i + 1);
                let line = if i == selected {
                    format!(
                        "  {} {} {}\r\n",
                        "❯".cyan().bold(),
                        num.cyan().bold(),
                        label.clone().bold().white()
                    )
                } else {
                    format!("    {} {}\r\n", num, label).grey().to_string()
                };
                let _ = execute!(self.stdout, Print(line));
            }
            let hint = fit_single_line_tail(hint, cols);
            let _ = execute!(self.stdout, Print(hint.dark_yellow().to_string()));
            self.managed_lines = todo_rows + display_labels.len() + 1;
        } else {
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
                format!("❯ {}", shown_input).grey().to_string()
            };
            let _ = execute!(self.stdout, Print(prompt));
            self.managed_lines = todo_rows + status_rows + 1;
        }
        let _ = self.stdout.flush();
    }

    /// Render the todo progress panel above the main status area.
    /// Returns the number of terminal rows consumed.
    fn draw_todo_panel(&mut self, cols: usize) -> usize {
        if self.todo_items.is_empty() {
            return 0;
        }

        // Count stats
        let total = self.todo_items.len();
        let done = self.todo_items.iter().filter(|t| t.status == TodoStatus::Done).count();
        let header = format!("  {} Todos ({}/{})", "▼".grey(), done, total);
        let _ = execute!(self.stdout, Print(format!("{}\r\n", header.grey())));

        let budget = cols.saturating_sub(rendered_text_width("      "));
        let mut rows = 1; // header
        for item in &self.todo_items {
            let (icon, styled_label) = match item.status {
                TodoStatus::Done => (
                    "\u{2705}".to_string(), // ✅
                    item.label.as_str().green().to_string(),
                ),
                TodoStatus::Running => (
                    "\u{25CE}".to_string(), // ◎
                    item.label.as_str().bold().white().to_string(),
                ),
                TodoStatus::Pending => (
                    "\u{25CB}".to_string(), // ○
                    item.label.as_str().grey().to_string(),
                ),
            };
            let label = fit_single_line_tail(&strip_ansi(&styled_label), budget);
            let styled = match item.status {
                TodoStatus::Done => label.green().to_string(),
                TodoStatus::Running => label.bold().white().to_string(),
                TodoStatus::Pending => label.grey().to_string(),
            };
            let _ = execute!(self.stdout, Print(format!("    {} {}\r\n", icon, styled)));
            rows += 1;
        }
        rows
    }

    pub(crate) fn emit(&mut self, lines: &[String]) {
        self.task_lines += lines.iter().map(|l| self.rendered_rows(l)).sum::<usize>();
        self.task_rendered.extend(lines.iter().cloned());
        self.clear_managed();
        for line in lines {
            let _ = execute!(self.stdout, Print(format!("{}\r\n", line)));
        }
        self.draw_managed();
    }

    pub(crate) fn refresh(&mut self) {
        self.clear_managed();
        self.draw_managed();
    }

    pub(crate) fn reset_task_lines(&mut self) {
        self.task_lines = 0;
        self.task_rendered.clear();
    }

    pub(crate) fn rendered_rows(&self, line: &str) -> usize {
        let cols = crossterm::terminal::size()
            .map(|(c, _)| c.max(1) as usize)
            .unwrap_or(80);
        let plain = strip_ansi(line);
        let width = rendered_text_width(plain.as_str());
        width.saturating_sub(1) / cols + 1
    }

    pub(crate) fn collapse_to(&mut self, kept: &[String]) {
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
    let model = std::env::var("BIGMODEL_MODEL").unwrap_or_else(|_| "GLM-4.7".to_string());
    let base_url = std::env::var("BIGMODEL_BASE_URL")
        .unwrap_or_else(|_| "https://open.bigmodel.cn/api/coding/paas/v4".to_string());
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

pub(crate) fn strip_ansi(s: &str) -> String {
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

pub(crate) fn rendered_text_width(s: &str) -> usize {
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

pub(crate) fn fit_single_line_tail(s: &str, max_width: usize) -> String {
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

pub(crate) fn split_tail_lines_by_width(
    s: &str,
    max_width: usize,
    max_lines: usize,
) -> Vec<String> {
    if max_width == 0 || max_lines == 0 {
        return Vec::new();
    }

    let mut wrapped = Vec::new();
    for raw_line in s.lines() {
        let raw_line = raw_line.replace('\t', " ");
        let mut cur = String::new();
        let mut cur_w = 0usize;
        let mut chars = raw_line.chars().peekable();
        while let Some(ch) = chars.next() {
            if ch == '\r' {
                continue;
            }
            if ch == '\u{1b}' && matches!(chars.peek(), Some('[')) {
                // Keep ANSI style sequence in output, but treat it as zero-width.
                cur.push(ch);
                if let Some(next) = chars.next() {
                    cur.push(next);
                }
                for c in chars.by_ref() {
                    cur.push(c);
                    if ('@'..='~').contains(&c) {
                        break;
                    }
                }
                continue;
            }
            if ch.is_control() {
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
        let plain = strip_ansi(first);
        *first = fit_single_line_tail(&format!("…{}", plain), max_width);
    }
    tail
}

/// Format discovered skills as a single styled line for `Screen::emit()`.
/// Returns `None` if no skills were found.
pub(crate) fn format_skills_status_line(names: &[String]) -> Option<String> {
    if names.is_empty() {
        return None;
    }
    let cols = crossterm::terminal::size()
        .map(|(c, _)| c.max(1) as usize)
        .unwrap_or(80);
    let prefix = "  Skills  ";
    let sep = " · ";
    let mut budget = cols.saturating_sub(rendered_text_width(prefix));

    let mut shown: Vec<String> = Vec::new();
    for (i, name) in names.iter().enumerate() {
        let needed = if i == 0 { 0 } else { rendered_text_width(sep) };
        let name_w = rendered_text_width(name);
        let remaining = names.len() - i;
        // Reserve space for "+N more" if we can't fit all remaining.
        let more_w = if remaining > 1 {
            rendered_text_width(sep) + rendered_text_width(&format!("+{}", remaining - 1))
        } else {
            0
        };
        if needed + name_w + more_w <= budget {
            budget = budget.saturating_sub(needed + name_w);
            shown.push(name.as_str().green().to_string());
        } else {
            let more = names.len() - shown.len();
            shown.push(format!("+{more}").grey().to_string());
            break;
        }
    }

    let sep_styled = sep.grey().to_string();
    Some(format!(
        "  {}{}",
        prefix.grey(),
        shown.join(&sep_styled)
    ))
}

/// Format the MCP discovery result as a single styled line for `Screen::emit()`.
/// Returns `None` if there are no servers at all.
pub(crate) fn format_mcp_status_line(ok: &[(String, usize)], failed: &[String]) -> Option<String> {
    if ok.is_empty() && failed.is_empty() {
        return None;
    }
    let sep = " · ".grey().to_string();
    let mut parts: Vec<String> = ok
        .iter()
        .map(|(name, _)| name.as_str().dark_cyan().to_string())
        .collect();
    for name in failed {
        parts.push(format!(
            "{} {}",
            name.as_str().red(),
            "(failed)".grey()
        ));
    }
    Some(format!("  {}{}", "MCP  ".grey(), parts.join(&sep)))
}
