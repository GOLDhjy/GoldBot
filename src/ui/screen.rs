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
    /// When true, all TUI rendering is suppressed (used with -p headless mode).
    pub headless: bool,
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
    /// 输入光标字节偏移
    pub input_cursor: usize,
    /// 光标在 hint 行上方几行（0=hint 行，1=单行旧行为，N=多行）
    cursor_rows_above_hint: usize,
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
    /// /model 选择器：待显示的标签列表
    pub model_picker_labels: Vec<String>,
    /// /model 选择器：当前选中的索引
    pub model_picker_sel: usize,
}

impl Screen {
    pub(crate) fn new_headless() -> io::Result<Self> {
        Ok(Self {
            stdout: io::stdout(),
            headless: true,
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
            input_cursor: 0,
            cursor_rows_above_hint: 0,
            last_status_rows: 1,
            at_file_labels: Vec::new(),
            at_file_sel: 0,
            command_labels: Vec::new(),
            command_sel: 0,
            model_picker_labels: Vec::new(),
            model_picker_sel: 0,
        })
    }

    pub(crate) fn new() -> io::Result<Self> {
        let mut s = Self {
            stdout: io::stdout(),
            headless: false,
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
            input_cursor: 0,
            cursor_rows_above_hint: 0,
            last_status_rows: 1,
            at_file_labels: Vec::new(),
            at_file_sel: 0,
            command_labels: Vec::new(),
            command_sel: 0,
            model_picker_labels: Vec::new(),
            model_picker_sel: 0,
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
        if self.headless {
            return;
        }
        // 如果光标在 hint 行上方，先移回 hint 行（managed 区底部）
        if self.cursor_rows_above_hint > 0 {
            let down = self.cursor_rows_above_hint.min(u16::MAX as usize) as u16;
            let _ = execute!(self.stdout, cursor::MoveDown(down));
            self.cursor_rows_above_hint = 0;
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
        if self.headless {
            return;
        }
        let cols = crossterm::terminal::size()
            .map(|(c, _)| c.max(1) as usize)
            .unwrap_or(80);

        // ── Todo panel (topmost in managed area) ──
        let todo_rows = self.draw_todo_panel(cols);

        // ── @ file picker panel ──
        let at_file_rows = self.draw_at_file_panel(cols);

        // ── / command picker panel ──
        let command_rows = self.draw_command_panel(cols);

        // ── /model picker panel ──
        let model_picker_rows = self.draw_model_picker_panel(cols);

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
            self.managed_lines = todo_rows
                + at_file_rows
                + command_rows
                + model_picker_rows
                + display_labels.len()
                + 1;
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

            // ── Multi-line input rendering ──
            let prompt_str = format!("{} ", sym.prompt);
            let prompt_width = rendered_text_width(&prompt_str);
            let input_budget = cols.saturating_sub(prompt_width);
            let indent = " ".repeat(prompt_width);

            let input_lines: Vec<&str> = self.input.split('\n').collect();
            let input_row_count = input_lines.len();

            for (i, line) in input_lines.iter().enumerate() {
                let shown = fit_single_line_tail(line, input_budget);
                let rendered = if i == 0 {
                    if self.input_focused {
                        format!("{}{}", prompt_str, shown)
                    } else {
                        format!("{}{}", prompt_str, shown).grey().to_string()
                    }
                } else if self.input_focused {
                    format!("{}{}", indent, shown)
                } else {
                    format!("{}{}", indent, shown).grey().to_string()
                };
                let _ = execute!(self.stdout, Print(format!("{}\r\n", rendered)));
            }

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
            self.managed_lines = todo_rows
                + at_file_rows
                + command_rows
                + model_picker_rows
                + status_rows
                + input_row_count
                + 1;

            // 光标归位
            if self.input_focused {
                let (cursor_row, cursor_col) = self.cursor_row_col();
                let lines_below_cursor =
                    (input_row_count - 1 - cursor_row) + 1; // +1 for hint line
                let col_offset = if cursor_row == 0 {
                    prompt_width + cursor_col
                } else {
                    prompt_width + cursor_col
                };
                let up = lines_below_cursor.min(u16::MAX as usize) as u16;
                let col = col_offset.min(u16::MAX as usize) as u16;
                let _ = execute!(
                    self.stdout,
                    cursor::MoveUp(up),
                    cursor::MoveToColumn(col),
                    cursor::Show
                );
                self.cursor_rows_above_hint = lines_below_cursor;
            } else {
                self.cursor_rows_above_hint = 0;
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

    /// Render the /model picker panel. Returns the number of terminal rows consumed.
    fn draw_model_picker_panel(&mut self, cols: usize) -> usize {
        if self.model_picker_labels.is_empty() {
            return 0;
        }
        let count = self.model_picker_labels.len();
        let max_visible = 8.min(count);
        let half = max_visible / 2;
        let start = if self.model_picker_sel > half {
            (self.model_picker_sel - half).min(count.saturating_sub(max_visible))
        } else {
            0
        };
        let end = (start + max_visible).min(count);
        let sym = Symbols::current();
        let budget = cols.saturating_sub(6);
        let mut rows = 0;
        for i in start..end {
            let label = &self.model_picker_labels[i];
            let trimmed = fit_single_line_tail(label, budget);
            let line = if i == self.model_picker_sel {
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
        if self.headless {
            return;
        }
        self.task_lines += lines.iter().map(|l| self.rendered_rows(l)).sum::<usize>();
        self.task_rendered.extend(lines.iter().cloned());
        self.clear_managed();
        for line in lines {
            let _ = execute!(self.stdout, Print(format!("{}\r\n", line)));
        }
        self.draw_managed();
    }

    pub(crate) fn refresh(&mut self) {
        if self.headless {
            return;
        }
        self.clear_managed();
        self.draw_managed();
    }

    /// 清空整个终端屏幕，重置任务记录，重新绘制底部管理区。
    pub(crate) fn clear_screen(&mut self) {
        let reserve_rows = self.managed_lines.max(3);
        self.task_lines = 0;
        self.task_rendered.clear();
        self.managed_lines = 3;
        self.input_cursor = 0;
        self.cursor_rows_above_hint = 0;
        let target_row = crossterm::terminal::size()
            .map(|(_, rows)| rows.saturating_sub(reserve_rows.min(u16::MAX as usize) as u16))
            .unwrap_or(0);
        let _ = execute!(
            self.stdout,
            crossterm::terminal::Clear(crossterm::terminal::ClearType::All),
            cursor::MoveTo(0, target_row)
        );
        self.draw_managed();
    }

    /// 只原地刷新状态行，不动输入栏和 hint 行，避免光标在两行之间跳动。
    /// 仅适用于 spinner 跳帧和思考预览更新场景。
    /// 若行数发生变化或处于确认/todo 界面，则回退到完整 refresh()。
    pub(crate) fn refresh_status_only(&mut self) {
        if self.headless {
            return;
        }
        if self.confirm_selected.is_some()
            || !self.todo_items.is_empty()
            || !self.at_file_labels.is_empty()
            || !self.command_labels.is_empty()
            || !self.model_picker_labels.is_empty()
            || self.input.contains('\n')
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

        // 从当前光标位置向上定位到第一条状态行
        let up = if self.cursor_rows_above_hint > 0 {
            (new_rows + self.cursor_rows_above_hint - 1) as u16
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
        if self.cursor_rows_above_hint > 0 {
            let prompt_str = format!("{} ", sym.prompt);
            let prompt_width = rendered_text_width(&prompt_str);
            let (_, cursor_col) = self.cursor_row_col();
            let input_col = (prompt_width + cursor_col).min(u16::MAX as usize) as u16;
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
        if self.headless {
            return;
        }
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

    // ── Cursor helper methods ────────────────────────────────────────────────

    /// Move cursor left by one char. Returns true if moved.
    pub fn cursor_left(&mut self) -> bool {
        if self.input_cursor == 0 {
            return false;
        }
        // Find the previous char boundary
        let mut pos = self.input_cursor - 1;
        while pos > 0 && !self.input.is_char_boundary(pos) {
            pos -= 1;
        }
        self.input_cursor = pos;
        true
    }

    /// Move cursor right by one char. Returns true if moved.
    pub fn cursor_right(&mut self) -> bool {
        if self.input_cursor >= self.input.len() {
            return false;
        }
        let ch = self.input[self.input_cursor..].chars().next().unwrap();
        self.input_cursor += ch.len_utf8();
        true
    }

    /// Move cursor left by one word (Ctrl+Left).
    pub fn cursor_word_left(&mut self) {
        if self.input_cursor == 0 {
            return;
        }
        let bytes = self.input.as_bytes();
        let mut pos = self.input_cursor;
        // Skip whitespace
        while pos > 0 && bytes[pos - 1].is_ascii_whitespace() {
            pos -= 1;
        }
        // Skip word chars
        while pos > 0 && !bytes[pos - 1].is_ascii_whitespace() {
            pos -= 1;
        }
        self.input_cursor = pos;
    }

    /// Move cursor right by one word (Ctrl+Right).
    pub fn cursor_word_right(&mut self) {
        let len = self.input.len();
        if self.input_cursor >= len {
            return;
        }
        let bytes = self.input.as_bytes();
        let mut pos = self.input_cursor;
        // Skip word chars
        while pos < len && !bytes[pos].is_ascii_whitespace() {
            pos += 1;
        }
        // Skip whitespace
        while pos < len && bytes[pos].is_ascii_whitespace() {
            pos += 1;
        }
        self.input_cursor = pos;
    }

    /// Move cursor up one line. Returns true if moved.
    pub fn cursor_up(&mut self) -> bool {
        let (row, col) = self.cursor_row_col();
        if row == 0 {
            return false;
        }
        self.move_cursor_to_line_col(row - 1, col);
        true
    }

    /// Move cursor down one line. Returns true if moved.
    pub fn cursor_down(&mut self) -> bool {
        let (row, col) = self.cursor_row_col();
        let line_count = self.input_line_count();
        if row + 1 >= line_count {
            return false;
        }
        self.move_cursor_to_line_col(row + 1, col);
        true
    }

    /// Move cursor to start of current line.
    pub fn cursor_home(&mut self) {
        // Find the start of the current line
        let before = &self.input[..self.input_cursor];
        if let Some(nl_pos) = before.rfind('\n') {
            self.input_cursor = nl_pos + 1;
        } else {
            self.input_cursor = 0;
        }
    }

    /// Move cursor to end of current line.
    pub fn cursor_end(&mut self) {
        let after = &self.input[self.input_cursor..];
        if let Some(nl_pos) = after.find('\n') {
            self.input_cursor += nl_pos;
        } else {
            self.input_cursor = self.input.len();
        }
    }

    /// Insert a char at cursor position.
    pub fn insert_char_at_cursor(&mut self, c: char) {
        self.input.insert(self.input_cursor, c);
        self.input_cursor += c.len_utf8();
    }

    /// Insert a string at cursor position.
    pub fn insert_at_cursor(&mut self, s: &str) {
        self.input.insert_str(self.input_cursor, s);
        self.input_cursor += s.len();
    }

    /// Delete the char before cursor. Returns the deleted char if any.
    pub fn delete_char_before_cursor(&mut self) -> Option<char> {
        if self.input_cursor == 0 {
            return None;
        }
        // Find previous char boundary
        let mut pos = self.input_cursor - 1;
        while pos > 0 && !self.input.is_char_boundary(pos) {
            pos -= 1;
        }
        let ch = self.input[pos..].chars().next().unwrap();
        self.input.drain(pos..self.input_cursor);
        self.input_cursor = pos;
        Some(ch)
    }

    /// Calculate cursor row and column (display width).
    pub fn cursor_row_col(&self) -> (usize, usize) {
        let before = &self.input[..self.input_cursor];
        let row = before.matches('\n').count();
        let last_nl = before.rfind('\n').map(|p| p + 1).unwrap_or(0);
        let col = rendered_text_width(&self.input[last_nl..self.input_cursor]);
        (row, col)
    }

    /// Count total lines in input.
    pub fn input_line_count(&self) -> usize {
        if self.input.is_empty() {
            1
        } else {
            self.input.matches('\n').count() + 1
        }
    }

    /// Move cursor to specified line and column (by display width).
    fn move_cursor_to_line_col(&mut self, target_line: usize, target_col: usize) {
        let mut line = 0;
        let mut line_start = 0;
        for (i, ch) in self.input.char_indices() {
            if line == target_line {
                line_start = i;
                break;
            }
            if ch == '\n' {
                line += 1;
                line_start = i + 1;
            }
        }
        if line < target_line {
            // target_line is beyond input
            self.input_cursor = self.input.len();
            return;
        }
        // Now walk from line_start to find the right column
        let mut col = 0;
        let mut pos = line_start;
        for (i, ch) in self.input[line_start..].char_indices() {
            if ch == '\n' {
                pos = line_start + i;
                break;
            }
            let w = UnicodeWidthChar::width(ch).unwrap_or(0);
            if col + w > target_col {
                pos = line_start + i;
                self.input_cursor = pos;
                return;
            }
            col += w;
            pos = line_start + i + ch.len_utf8();
        }
        self.input_cursor = pos;
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
        parts.push(name.as_str().red().to_string());
    }
    Some(format!("  {}{}", "MCP  ".grey(), parts.join(&sep)))
}
