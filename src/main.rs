mod agent;
mod memory;
mod tools;
mod types;

use std::{
    io::{self, Write},
    time::Duration,
};

use agent::{
    react::{SYSTEM_PROMPT, parse_llm_response},
    provider::{Message, build_http_client, chat},
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

// ── Screen ────────────────────────────────────────────────────────────────────
// The bottom of the terminal is a "managed" area that is redrawn in place.
//
// Normal mode  (managed_lines = 2):
//   ─ status line  : "  ⏳ Thinking…"  or blank
//   ─ input line   : "❯ <text>"
//
// Confirmation mode  (managed_lines = 4):
//   ─ "  ❯ Execute"   (selected, cyan+bold)
//   ─ "    Skip"
//   ─ "    Abort"
//   ─ "❯ "            (greyed out — typing disabled)
//
// task_lines counts lines currently shown for the active task so that
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
    managed_lines: usize,               // lines currently on screen in managed area
    pub confirm_selected: Option<usize>, // Some(n) → in confirmation mode, n selected
}

impl Screen {
    fn new() -> io::Result<Self> {
        let mut s = Self {
            stdout: io::stdout(),
            status: String::new(),
            input: String::new(),
            task_lines: 0,
            managed_lines: 2,
            confirm_selected: None,
        };
        for line in TITLE_BANNER {
            execute!(s.stdout, Print(format!("{}\r\n", line.cyan().bold())))?;
        }
        execute!(
            s.stdout,
            Print("  Local terminal automation agent\r\n".dark_grey().to_string()),
            Print("\r\n"),
            Print("❯ ")
        )?;
        s.stdout.flush()?;
        Ok(s)
    }

    /// Erase the managed area; cursor ends at col 0 of the first managed line.
    fn clear_managed(&mut self) {
        let _ = execute!(
            self.stdout,
            cursor::MoveToColumn(0),
            cursor::MoveUp((self.managed_lines - 1) as u16),
            Clear(ClearType::FromCursorDown),
        );
    }

    /// Draw the managed area and update managed_lines to match what was drawn.
    fn draw_managed(&mut self) {
        if let Some(selected) = self.confirm_selected {
            // Vertical confirmation menu (3 options + disabled input = 4 lines).
            let labels = ["Execute", "Skip", "Abort"];
            for (i, label) in labels.iter().enumerate() {
                let line = if i == selected {
                    format!("  ❯ {}\r\n", label).bold().cyan().to_string()
                } else {
                    format!("    {}\r\n", label).dark_grey().to_string()
                };
                let _ = execute!(self.stdout, Print(line));
            }
            let _ = execute!(self.stdout, Print("❯ ".dark_grey().to_string()));
            self.managed_lines = 4;
        } else {
            // Normal: status + input.
            let st = if self.status.is_empty() {
                "\r\n".to_string()
            } else {
                format!("  {}\r\n", self.status)
            };
            let _ = execute!(self.stdout, Print(st), Print(format!("❯ {}", self.input)));
            self.managed_lines = 2;
        }
        let _ = self.stdout.flush();
    }

    /// Emit event lines into the scrolling area above, then redraw managed area.
    fn emit(&mut self, lines: &[String]) {
        self.task_lines += lines.len();
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
    }

    /// Erase all task lines + managed area and replace with `kept`.
    fn collapse_to(&mut self, kept: &[String]) {
        let up = (self.task_lines + self.managed_lines - 1) as u16;
        let _ = execute!(
            self.stdout,
            cursor::MoveToColumn(0),
            cursor::MoveUp(up),
            Clear(ClearType::FromCursorDown),
        );
        self.task_lines = 0;
        for line in kept {
            let _ = execute!(self.stdout, Print(format!("{}\r\n", line)));
            self.task_lines += 1;
        }
        self.draw_managed();
    }
}

// ── App ───────────────────────────────────────────────────────────────────────
struct App {
    messages: Vec<Message>,
    task: String,
    steps_taken: usize,
    max_steps: usize,
    llm_calling: bool,
    needs_agent_step: bool,
    running: bool,
    quit: bool,
    pending_confirm: Option<String>,
    task_events: Vec<Event>,        // intermediate events saved for expand
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
            needs_agent_step: false,
            running: false,
            quit: false,
            pending_confirm: None,
            task_events: Vec::new(),
            final_summary: None,
            task_collapsed: false,
        }
    }
}

// ── Event formatting ──────────────────────────────────────────────────────────

fn format_event(event: &Event) -> Vec<String> {
    match event {
        Event::UserTask { text } => lines_with(text, |i, line| {
            if i == 0 { format!("❯ {}", line).bold().to_string() } else { format!("  {}", line) }
        }),
        Event::Thinking { text } => {
            lines_with(text, |_, line| format!("  {}", line).dark_grey().to_string())
        }
        Event::ToolCall { command } => lines_with(command, |i, line| {
            if i == 0 {
                format!("  ⏺ {}", line).cyan().to_string()
            } else {
                format!("    {}", line).dark_grey().to_string()
            }
        }),
        Event::ToolResult { output, exit_code, .. } => {
            let ok = *exit_code == 0;
            lines_with(output, |i, line| {
                let pfx = if i == 0 { "  ⎿ " } else { "    " };
                let s = format!("{}{}", pfx, line);
                if ok { s.dark_grey().to_string() } else { s.red().to_string() }
            })
        }
        Event::NeedsConfirmation { command, reason } => {
            let first = command.lines().next().unwrap_or(command.as_str());
            let cmd_display = if command.lines().count() > 1 {
                format!("{} …", first)
            } else {
                first.to_string()
            };
            vec![format!("  ⚠ {} — {}", cmd_display, reason).yellow().to_string()]
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
    if v.is_empty() { return vec![f(0, "")]; }
    v.iter().enumerate().map(|(i, l)| f(i, l)).collect()
}

// ── Collapse / expand ─────────────────────────────────────────────────────────

fn collapsed_lines(app: &App) -> Vec<String> {
    let summary = app.final_summary.as_deref().unwrap_or("");
    let mut lines = format_event(&Event::UserTask { text: app.task.clone() });
    lines.push(String::new());
    lines.extend(format_event(&Event::Final { summary: summary.to_string() }));
    lines.push(String::new());
    lines
}

fn expanded_lines(app: &App) -> Vec<String> {
    let summary = app.final_summary.as_deref().unwrap_or("");
    let mut lines = format_event(&Event::UserTask { text: app.task.clone() });
    lines.push(String::new());
    for ev in &app.task_events {
        lines.extend(format_event(ev));
    }
    lines.push(String::new());
    lines.extend(format_event(&Event::Final { summary: summary.to_string() }));
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
        screen.status = "[d] collapse".dark_grey().to_string();
    } else {
        screen.collapse_to(&collapsed_lines(app));
        app.task_collapsed = true;
        screen.status = "[d] expand".dark_grey().to_string();
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

    let (tx, mut rx) = mpsc::channel::<anyhow::Result<String>>(1);

    loop {
        if let Ok(result) = rx.try_recv() {
            app.llm_calling = false;
            screen.status.clear();
            process_llm_result(app, screen, result);
        }

        if app.running
            && app.pending_confirm.is_none()
            && app.needs_agent_step
            && !app.llm_calling
        {
            app.needs_agent_step = false;
            app.llm_calling = true;
            screen.status = "⏳ Thinking...".dim().to_string();
            screen.refresh();

            let tx2 = tx.clone();
            let client = http_client.clone();
            let messages = app.messages.clone();
            tokio::spawn(async move {
                let _ = tx2.send(chat(&client, &messages).await).await;
            });
        }

        if event::poll(Duration::from_millis(50))?
            && let CEvent::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
        {
            if handle_key(app, screen, key.code, key.modifiers) {
                break;
            }
        }

        if app.quit { break; }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    Ok(())
}

// ── Key handling ──────────────────────────────────────────────────────────────

fn handle_key(
    app: &mut App,
    screen: &mut Screen,
    key: KeyCode,
    modifiers: KeyModifiers,
) -> bool {
    if key == KeyCode::Char('c') && modifiers.contains(KeyModifiers::CONTROL) {
        return true;
    }

    if screen.confirm_selected.is_some() {
        // ── Confirmation mode: ↑/↓ navigate, Enter confirm ───────────────────
        let sel = screen.confirm_selected.unwrap();
        match key {
            KeyCode::Up => {
                screen.confirm_selected = Some(sel.saturating_sub(1));
                screen.refresh();
            }
            KeyCode::Down => {
                screen.confirm_selected = Some((sel + 1).min(2));
                screen.refresh();
            }
            KeyCode::Enter => {
                screen.confirm_selected = None;
                let Some(cmd) = app.pending_confirm.take() else {
                    screen.refresh();
                    return false;
                };
                match sel {
                    0 => {
                        // Execute
                        execute_command(app, screen, &cmd);
                        app.needs_agent_step = true;
                    }
                    1 => {
                        // Skip
                        let msg = "User chose to skip this command";
                        app.messages.push(Message::user(format!("Tool result:\n{msg}")));
                        let ev = Event::ToolResult {
                            _command: cmd,
                            exit_code: 0,
                            output: msg.to_string(),
                        };
                        screen.emit(&format_event(&ev));
                        app.task_events.push(ev);
                        app.needs_agent_step = true;
                    }
                    _ => {
                        // Abort
                        finish(app, screen, "Task aborted by user".to_string());
                    }
                }
            }
            _ => {}
        }
    } else if !app.running {
        // ── Idle / input mode ─────────────────────────────────────────────────
        match key {
            KeyCode::Enter => {
                let task = screen.input.trim().to_string();
                if !task.is_empty() {
                    screen.input.clear();
                    start_task(app, screen, task);
                }
            }
            KeyCode::Char('d') if modifiers.is_empty() => toggle_collapse(app, screen),
            KeyCode::Esc | KeyCode::Char('q') if modifiers.is_empty() => return true,
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
        // ── Running (LLM in flight) ───────────────────────────────────────────
        if key == KeyCode::Char('q') && modifiers.is_empty() {
            return true;
        }
    }

    false
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
    app.needs_agent_step = true;
    app.pending_confirm = None;
    screen.confirm_selected = None;
    app.task_events.clear();
    app.final_summary = None;
    app.task_collapsed = false;
    app.messages.push(Message::user(task.clone()));

    screen.emit(&format_event(&Event::UserTask { text: task }));
}

fn process_llm_result(app: &mut App, screen: &mut Screen, result: anyhow::Result<String>) {
    if app.steps_taken >= app.max_steps {
        finish(app, screen, format!("Reached max steps ({}).", app.max_steps));
        return;
    }
    app.steps_taken += 1;

    let response = match result {
        Ok(r) => r,
        Err(e) => {
            let ev = Event::Thinking { text: format!("[LLM error] {e}") };
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
            screen.status = format!("↻ Retrying invalid response format: {e}").dark_grey().to_string();
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
                    screen.confirm_selected = Some(0);
                    screen.refresh();
                }
                RiskLevel::Block => {
                    let msg = "Command blocked by safety policy";
                    app.messages.push(Message::user(format!("Tool result:\n{msg}")));
                    let ev = Event::ToolResult {
                        _command: command,
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
    let call_ev = Event::ToolCall { command: cmd.to_string() };
    screen.emit(&format_event(&call_ev));
    app.task_events.push(call_ev);

    match tools::shell::run_command(cmd) {
        Ok(out) => {
            app.messages.push(Message::user(format!(
                "Tool result (exit={}):\n{}",
                out.exit_code, out.output
            )));
            let ev = Event::ToolResult {
                _command: cmd.to_string(),
                exit_code: out.exit_code,
                output: out.output,
            };
            screen.emit(&format_event(&ev));
            app.task_events.push(ev);
        }
        Err(e) => {
            let err = format!("execution failed: {e}");
            app.messages.push(Message::user(format!("Tool result (exit=-1):\n{err}")));
            let ev = Event::ToolResult {
                _command: cmd.to_string(),
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
    app.pending_confirm = None;
    screen.confirm_selected = None;
    screen.status = "[d] expand".dark_grey().to_string();
    screen.refresh();
}
