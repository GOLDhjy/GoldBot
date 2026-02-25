use std::io::{self, Write};

use crossterm::{
    cursor, execute,
    style::{Color, Print, Stylize},
    terminal::{Clear, ClearType},
};
use unicode_width::UnicodeWidthChar;

use crate::types::{AssistMode, TodoItem, TodoStatus};
use crate::ui::symbols::Symbols;

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
    /// Current Shift+Tab assist mode.
    pub assist_mode: AssistMode,
    /// Current workspace path (shown in UI hint bar).
    pub workspace: String,
    /// Whether the agent is currently running (shows animated spinner).
    pub is_running: bool,
    /// Spinner animation frame counter, incremented by the main loop.
    pub spinner_tick: u64,
    /// 光标是否已从 hint 行上移到 prompt 行（影响 clear_managed 的起点计算）
    cursor_at_prompt: bool,
    /// 上次 draw_managed 渲染的状态行数，用于 refresh_status_only 定位
    last_status_rows: usize,
    /// @ 文件选择器：待显示的候选文件路径（相对路径字符串）
    pub at_file_labels: Vec<String>,
    /// @ 文件选择器：当前选中的索引
    pub at_file_sel: usize,
    /// / 命令选择器：待显示的命令名+描述列表（"name  description" 格式）
    pub command_labels: Vec<String>,
    /// / 命令选择器：当前选中的索引
    pub command_sel: usize,
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
            assist_mode: AssistMode::Off,
            workspace: String::new(),
            is_running: false,
            spinner_tick: 0,
            cursor_at_prompt: false,
            last_status_rows: 1,
            at_file_labels: Vec::new(),
            at_file_sel: 0,
            command_labels: Vec::new(),
            command_sel: 0,
        };
        execute!(s.stdout, cursor::MoveToColumn(0), Print("\r\n"))?;
        for line in TITLE_BANNER {
            execute!(
                s.stdout,
                cursor::MoveToColumn(0),
                Clear(ClearType::CurrentLine),
                Print(format!(
                    "{}\r\n",
                    line.with(Color::Rgb {
                        r: 255,
                        g: 215,
                        b: 0
                    })
                    .bold()
                ))
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
            Print(format!("{} ", Symbols::current().prompt))
        )?;
        let _ = execute!(s.stdout, cursor::Hide);
        s.stdout.flush()?;
        Ok(s)
    }

    pub(crate) fn clear_managed(&mut self) {
        // 如果光标已上移到 prompt 行，先移回 hint 行（managed 区底部）
        if self.cursor_at_prompt {
            let _ = execute!(self.stdout, cursor::MoveDown(1));
            self.cursor_at_prompt = false;
        }
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

        // ── @ file picker panel ──
        let at_file_rows = self.draw_at_file_panel(cols);

        // ── / command picker panel ──
        let command_rows = self.draw_command_panel(cols);

        // 进入渲染前先隐藏光标，避免渲染过程中闪烁
        let _ = execute!(self.stdout, cursor::Hide);

        if let Some(selected) = self.confirm_selected {
            let sym = Symbols::current();
            let (labels, hint): (&[&str], String) = if self.question_labels.is_empty() {
                (
                    &["Execute", "Skip", "Abort", "Add Note"],
                    format!("{} 直接输入补充说明，或 ↑/↓ 选择后 Enter", sym.prompt),
                )
            } else {
                (
                    &[],
                    format!("{} ↑/↓ 选择，Enter 确认，或直接输入自定义内容", sym.prompt),
                )
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
                        sym.prompt.cyan().bold(),
                        num.cyan().bold(),
                        label.clone().bold().white()
                    )
                } else {
                    format!("    {} {}\r\n", num, label).grey().to_string()
                };
                let _ = execute!(self.stdout, Print(line));
            }
            let hint = fit_single_line_tail(&hint, cols);
            let _ = execute!(self.stdout, Print(hint.dark_yellow().to_string()));
            self.managed_lines = todo_rows + at_file_rows + command_rows + display_labels.len() + 1;
        } else {
            let sym = Symbols::current();
            let spinner_frame =
                sym.spinner_frames[self.spinner_tick as usize % sym.spinner_frames.len()];
            let status_display = if self.is_running {
                let label = if self.status.is_empty() {
                    "Working...".to_string()
                } else {
                    self.status.clone()
                };
                format!("{} {}", spinner_frame.cyan().bold(), label)
            } else {
                self.status.clone()
            };
            let status_budget = cols.saturating_sub(rendered_text_width("  "));
            let max_status_lines = if self.is_running { 3 } else { 1 };
            let status_lines =
                split_tail_lines_by_width(&status_display, status_budget, max_status_lines);
            let status_rows = if status_lines.is_empty() {
                1
            } else {
                status_lines.len()
            };
            self.last_status_rows = status_rows;
            if status_lines.is_empty() {
                let _ = execute!(self.stdout, Print("\r\n"));
            } else {
                for line in status_lines {
                    let _ = execute!(self.stdout, Print(format!("  {}\r\n", line)));
                }
            }
            let prompt_str = format!("{} ", sym.prompt);
            let input_budget = cols.saturating_sub(rendered_text_width(&prompt_str));
            let shown_input = fit_single_line_tail(&self.input, input_budget);
            let prompt = if self.input_focused {
                format!("{}{}", prompt_str, shown_input)
            } else {
                format!("{}{}", prompt_str, shown_input).grey().to_string()
            };
            let _ = execute!(self.stdout, Print(format!("{}\r\n", prompt)));
            let accept_hint = match self.assist_mode {
                AssistMode::Off => format!(
                    "  {} {}{}",
                    sym.arrow_right.dark_grey(),
                    "mode: agent".grey(),
                    " (shift+tab to cycle)".grey(),
                ),
                AssistMode::AcceptEdits => format!(
                    "  {} {} {}{}",
                    format!("{}{}", sym.arrow_right, sym.arrow_right)
                        .green()
                        .bold(),
                    "mode:".grey(),
                    "accept edits".green().bold(),
                    " (shift+tab to cycle)".grey(),
                ),
                AssistMode::Plan => format!(
                    "  {} {} {}{}",
                    format!("{}{}", sym.arrow_right, sym.arrow_right)
                        .cyan()
                        .bold(),
                    "mode:".grey(),
                    "plan".cyan().bold(),
                    " (shift+tab to cycle)".grey(),
                ),
            };
            let _ = execute!(self.stdout, Print(accept_hint));
            self.managed_lines = todo_rows + at_file_rows + command_rows + status_rows + 2;

            // 光标归位：hint 行无 \r\n，cursor 就在 hint 行末；
            // 上移 1 行即到输入行，再定位到输入末尾并显示
            if self.input_focused {
                let input_col = (rendered_text_width(&prompt_str)
                    + rendered_text_width(&shown_input))
                .min(u16::MAX as usize) as u16;
                let _ = execute!(
                    self.stdout,
                    cursor::MoveUp(1),
                    cursor::MoveToColumn(input_col),
                    cursor::Show
                );
                self.cursor_at_prompt = true;
            }
        }
        let _ = self.stdout.flush();
    }

    /// Render the todo progress panel above the main status area.
    /// Returns the number of terminal rows consumed.
    fn draw_todo_panel(&mut self, cols: usize) -> usize {
        if self.todo_items.is_empty() {
            return 0;
        }

        let sym = Symbols::current();
        // Count stats
        let total = self.todo_items.len();
        let done = self
            .todo_items
            .iter()
            .filter(|t| t.status == TodoStatus::Done)
            .count();
        let header = format!("  {} Todos ({}/{})", sym.arrow_down.grey(), done, total);
        let _ = execute!(self.stdout, Print(format!("{}\r\n", header.grey())));

        let budget = cols.saturating_sub(rendered_text_width("      "));
        let mut rows = 1; // header
        for item in &self.todo_items {
            let (icon, styled_label) = match item.status {
                TodoStatus::Done => (
                    sym.check.to_string(),
                    item.label.as_str().green().to_string(),
                ),
                TodoStatus::Running => (
                    sym.running.to_string(),
                    item.label.as_str().bold().white().to_string(),
                ),
                TodoStatus::Pending => (
                    sym.pending.to_string(),
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

    /// Render the @ file picker panel below the todo panel.
    /// Returns the number of terminal rows consumed.
    fn draw_at_file_panel(&mut self, cols: usize) -> usize {
        if self.at_file_labels.is_empty() {
            return 0;
        }

        let sym = Symbols::current();
        let count = self.at_file_labels.len();
        let header = format!("  {} 文件 ({count})", sym.arrow_down);
        let _ = execute!(self.stdout, Print(format!("{}\r\n", header.grey())));

        let max_visible = 8.min(count);
        // Sliding window: keep selected item visible
        let half = max_visible / 2;
        let start = if self.at_file_sel > half {
            (self.at_file_sel - half).min(count.saturating_sub(max_visible))
        } else {
            0
        };
        let end = (start + max_visible).min(count);

        let budget = cols.saturating_sub(6);
        let mut rows = 1; // header row
        for i in start..end {
            let label = &self.at_file_labels[i];
            let trimmed = fit_single_line_tail(label, budget);
            let line = if i == self.at_file_sel {
                format!(
                    "  {} {}\r\n",
                    sym.prompt.cyan().bold(),
                    trimmed.bold().white()
                )
            } else {
                format!("    {}\r\n", trimmed).grey().to_string()
            };
            let _ = execute!(self.stdout, Print(line));
            rows += 1;
        }
        rows
    }

    /// Render the / command picker panel. Returns the number of terminal rows consumed.
    fn draw_command_panel(&mut self, cols: usize) -> usize {
        if self.command_labels.is_empty() {
            return 0;
        }

        let count = self.command_labels.len();
        let max_visible = 8.min(count);
        let half = max_visible / 2;
        let start = if self.command_sel > half {
            (self.command_sel - half).min(count.saturating_sub(max_visible))
        } else {
            0
        };
        let end = (start + max_visible).min(count);

        let sym = Symbols::current();
        let budget = cols.saturating_sub(6);
        let mut rows = 0;
        for i in start..end {
            let label = &self.command_labels[i];
            let trimmed = fit_single_line_tail(label, budget);
            let line = if i == self.command_sel {
                format!(
                    "  {} {}\r\n",
                    sym.prompt.cyan().bold(),
                    trimmed.bold().white()
                )
            } else {
                format!("    {}\r\n", trimmed).grey().to_string()
            };
            let _ = execute!(self.stdout, Print(line));
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

    /// 只原地刷新状态行，不动输入栏和 hint 行，避免光标在两行之间跳动。
    /// 仅适用于 spinner 跳帧和思考预览更新场景。
    /// 若行数发生变化或处于确认/todo 界面，则回退到完整 refresh()。
    pub(crate) fn refresh_status_only(&mut self) {
        if self.confirm_selected.is_some()
            || !self.todo_items.is_empty()
            || !self.at_file_labels.is_empty()
            || !self.command_labels.is_empty()
        {
            self.refresh();
            return;
        }

        let cols = crossterm::terminal::size()
            .map(|(c, _)| c.max(1) as usize)
            .unwrap_or(80);
        let sym = Symbols::current();
        let spinner_frame =
            sym.spinner_frames[self.spinner_tick as usize % sym.spinner_frames.len()];

        let status_display = if self.is_running {
            let label = if self.status.is_empty() {
                "Working...".to_string()
            } else {
                self.status.clone()
            };
            format!("{} {}", spinner_frame.cyan().bold(), label)
        } else {
            self.status.clone()
        };

        let max_status_lines = if self.is_running { 3 } else { 1 };
        let status_budget = cols.saturating_sub(rendered_text_width("  "));
        let new_lines = split_tail_lines_by_width(&status_display, status_budget, max_status_lines);
        let new_rows = new_lines.len().max(1);

        // 行数变了需要全刷（否则会错位）
        if new_rows != self.last_status_rows {
            self.refresh();
            return;
        }

        // 从当前光标位置（prompt 行 or hint 行）向上定位到第一条状态行
        let up = if self.cursor_at_prompt {
            new_rows as u16
        } else {
            (new_rows + 1) as u16
        };

        let _ = execute!(
            self.stdout,
            cursor::Hide,
            cursor::MoveToColumn(0),
            cursor::MoveUp(up)
        );

        if new_lines.is_empty() {
            let _ = execute!(self.stdout, Clear(ClearType::CurrentLine), Print("\r\n"));
        } else {
            for line in &new_lines {
                let _ = execute!(
                    self.stdout,
                    Clear(ClearType::CurrentLine),
                    Print(format!("  {}\r\n", line))
                );
            }
        }

        // 打印完状态行后，光标位于 prompt 行开头，恢复到原来位置
        if self.cursor_at_prompt {
            let prompt_str = format!("{} ", sym.prompt);
            let input_budget = cols.saturating_sub(rendered_text_width(&prompt_str));
            let shown_input = fit_single_line_tail(&self.input, input_budget);
            let input_col = (rendered_text_width(&prompt_str) + rendered_text_width(&shown_input))
                .min(u16::MAX as usize) as u16;
            let _ = execute!(self.stdout, cursor::MoveToColumn(input_col), cursor::Show);
        } else {
            // 移回 hint 行（仅向下一行，不显示光标）
            let _ = execute!(self.stdout, cursor::MoveDown(1));
        }

        let _ = self.stdout.flush();
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
    let (model, base_url) = crate::agent::provider::LlmBackend::from_env().display_info();
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

    let ellipsis = Symbols::current().ellipsis;
    let ellipsis_width = rendered_text_width(ellipsis);
    if max_width <= ellipsis_width {
        return ellipsis.to_string();
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
    out.push_str(ellipsis);
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
        let ellipsis = Symbols::current().ellipsis;
        *first = fit_single_line_tail(&format!("{}{}", ellipsis, plain), max_width);
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
    let sep = format!(" {} ", Symbols::current().dot);
    let mut budget = cols.saturating_sub(rendered_text_width(prefix));

    let mut shown: Vec<String> = Vec::new();
    for (i, name) in names.iter().enumerate() {
        let needed = if i == 0 { 0 } else { rendered_text_width(&sep) };
        let name_w = rendered_text_width(name);
        let remaining = names.len() - i;
        // Reserve space for "+N more" if we can't fit all remaining.
        let more_w = if remaining > 1 {
            rendered_text_width(&sep) + rendered_text_width(&format!("+{}", remaining - 1))
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
    Some(format!("  {}{}", prefix.grey(), shown.join(&sep_styled)))
}

/// Format the MCP discovery result as a single styled line for `Screen::emit()`.
/// Returns `None` if there are no servers at all.
pub(crate) fn format_mcp_status_line(ok: &[(String, usize)], failed: &[String]) -> Option<String> {
    if ok.is_empty() && failed.is_empty() {
        return None;
    }
    let sep = format!(" {} ", Symbols::current().dot).grey().to_string();
    let mut parts: Vec<String> = ok
        .iter()
        .map(|(name, _)| name.as_str().dark_cyan().to_string())
        .collect();
    for name in failed {
        parts.push(format!("{} {}", name.as_str().red(), "(failed)".grey()));
    }
    Some(format!("  {}{}", "MCP  ".grey(), parts.join(&sep)))
}
