mod agent;
mod memory;
mod tools;
mod types;

use std::{
    io::{self, Write},
    time::Duration,
};

use agent::{
    provider::{Message, build_http_client, chat_stream_with},
    react::{SYSTEM_PROMPT, parse_llm_response},
};
use crossterm::{
    cursor,
    event::{
        self, Event as CEvent, KeyCode, KeyEventKind, KeyModifiers,
    },
    execute,
    style::{Print, Stylize},
    terminal::{Clear, ClearType, disable_raw_mode, enable_raw_mode},
};
use memory::store::MemoryStore;
use tokio::sync::mpsc;
use tools::safety::{RiskLevel, assess_command};
use types::{Event, LlmAction};
use unicode_width::UnicodeWidthChar;

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
    managed_lines: usize, // lines currently on screen in managed area
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
        for line in TITLE_BANNER {
            execute!(s.stdout, Print(format!("{}\r\n", line.cyan().bold())))?;
        }
        execute!(
            s.stdout,
            Print(
                "  Local terminal automation agent\r\n"
                    .dark_grey()
                    .to_string()
            ),
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
        if let Some(selected) = self.confirm_selected {
            // Vertical confirmation menu (4 options + hint line = 5 lines).
            let labels = ["Execute", "Skip", "Abort", "Add Note"];
            for (i, label) in labels.iter().enumerate() {
                let line = if i == selected {
                    format!("  ❯ {}\r\n", label)
                        .bold()
                        .black()
                        .on_cyan()
                        .to_string()
                } else {
                    format!("    {}\r\n", label).white().to_string()
                };
                let _ = execute!(self.stdout, Print(line));
            }
            let hint = "❯ 直接输入补充说明，或 ↑/↓ 选择后 Enter";
            let _ = execute!(self.stdout, Print(hint.dark_yellow().to_string()));
            self.managed_lines = 5;
        } else {
            // Normal: status + input.
            let st = if self.status.is_empty() {
                "\r\n".to_string()
            } else {
                format!("  {}\r\n", self.status)
            };
            let prompt = if self.input_focused {
                format!("❯ {}", self.input)
            } else {
                format!("❯ {}", self.input).dark_grey().to_string()
            };
            let _ = execute!(self.stdout, Print(st), Print(prompt));
            self.managed_lines = 2;
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

// ── App ───────────────────────────────────────────────────────────────────────
struct App {
    messages: Vec<Message>,
    task: String,
    steps_taken: usize,
    max_steps: usize,
    llm_calling: bool,
    llm_stream_preview: String, // rolling raw stream buffer
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
        Self {
            messages: vec![Message::system(SYSTEM_PROMPT.to_string())],
            task: String::new(),
            steps_taken: 0,
            max_steps: 30,
            llm_calling: false,
            llm_stream_preview: String::new(),
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
        Event::NeedsConfirmation { command, reason } => {
            let first = command.lines().next().unwrap_or(command.as_str());
            let cmd_display = if command.lines().count() > 1 {
                format!("{} …", first)
            } else {
                first.to_string()
            };
            vec![
                format!("  ⚠ {} — {}", cmd_display, reason)
                    .yellow()
                    .to_string(),
            ]
        }
        Event::Final { summary } => lines_with(summary, |i, line| {
            if i == 0 {
                format!("  ✓ {}", line).green().bold().to_string()
            } else {
                format!("    {}", line).green().to_string()
            }
        }),
    }
}

fn lines_with(text: &str, f: impl Fn(usize, &str) -> String) -> Vec<String> {
    let v: Vec<&str> = text.lines().collect();
    if v.is_empty() {
        return vec![f(0, "")];
    }
    v.iter().enumerate().map(|(i, l)| f(i, l)).collect()
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
        Event::NeedsConfirmation { command, reason } => {
            let first = command.lines().next().unwrap_or(command.as_str());
            vec![
                format!("    ⚠ confirm needed: {} — {}", shorten_text(first, 72), reason)
                    .yellow()
                    .to_string(),
            ]
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
        .map(String::as_str)
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
    for ev in &app.task_events {
        lines.extend(format_event_compact(ev));
    }
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
                    screen.status.clear();
                    process_llm_result(app, screen, result);
                }
            }
        }

        if app.running && app.pending_confirm.is_none() && app.needs_agent_step && !app.llm_calling
        {
            app.needs_agent_step = false;
            app.llm_calling = true;
            app.llm_stream_preview.clear();
            screen.status = "⏳ Thinking...".dim().to_string();
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
    if app.llm_stream_preview.len() > 4096 {
        let drop_len = app.llm_stream_preview.len().saturating_sub(4096);
        app.llm_stream_preview.drain(..drop_len);
    }

    let preview = extract_live_preview(&app.llm_stream_preview);
    if preview.is_empty() {
        return;
    }
    screen.status = format!("⏳ {}", preview).dim().to_string();
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
    tail_chars(&collapsed, 56)
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

fn tail_chars(s: &str, max: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max {
        return s.trim().to_string();
    }
    let tail: String = chars[chars.len() - max..].iter().collect();
    format!("…{}", tail.trim())
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
                        app.messages.push(Message::user(format!("Tool result:\n{msg}")));
                        let ev = Event::ToolResult {
                            exit_code: 0,
                            output: msg,
                        };
                        screen.emit(&format_event(&ev));
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
                screen.emit(&format_event(&ev));
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
    app.needs_agent_step = true;
    app.pending_confirm = None;
    app.pending_confirm_note = false;
    screen.confirm_selected = None;
    screen.input_focused = true;
    app.task_events.clear();
    app.final_summary = None;
    app.task_collapsed = false;
    app.messages.push(Message::user(task.clone()));

    screen.emit(&format_event(&Event::UserTask { text: task }));
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
            screen.emit(&format_event(&ev));
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
        screen.emit(&format_event(&ev));
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
                    screen.emit(&format_event(&ev));
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
                    screen.emit(&format_event(&ev));
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
    screen.emit(&format_event(&call_ev));
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
            screen.emit(&format_event(&ev));
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
            screen.emit(&format_event(&ev));
            app.task_events.push(ev);
        }
    }
}

fn finish(app: &mut App, screen: &mut Screen, summary: String) {
    app.final_summary = Some(summary.clone());
    app.task_collapsed = true;

    screen.collapse_to(&collapsed_lines(app));

    let store = MemoryStore::new();
    let _ = store.append_short_term(&app.task, &summary);
    let _ = store.append_long_term(&format!("task={} | result={}", app.task, summary));

    app.running = false;
    app.llm_stream_preview.clear();
    app.pending_confirm = None;
    app.pending_confirm_note = false;
    screen.confirm_selected = None;
    screen.input_focused = true;
    screen.status = "[Ctrl+d] full details".dark_grey().to_string();
    screen.refresh();
}
