mod agent;
mod consensus;
mod memory;
mod tools;
mod types;
mod ui;

use std::{io, time::Duration};

use agent::{
    provider::{Message, build_http_client, chat_stream_with},
    react::build_system_prompt,
    step::{
        execute_command, finish, handle_llm_stream_delta, handle_llm_thinking_delta,
        maybe_flush_and_compact_before_call, process_llm_result, start_task,
    },
};
use crossterm::{
    event::{
        self, DisableBracketedPaste, EnableBracketedPaste, Event as CEvent, KeyCode, KeyEventKind,
        KeyModifiers,
    },
    execute,
    style::{Print, Stylize},
    terminal::{disable_raw_mode, enable_raw_mode},
};
use memory::store::MemoryStore;
use tokio::sync::mpsc;
use tools::skills::{Skill, discover_skills, skills_system_prompt};
use types::{Event, Mode};
use ui::format::{emit_live_event, toggle_collapse};
use ui::screen::{Screen, format_mcp_status_line, format_skills_status_line, strip_ansi};

pub(crate) const MAX_MESSAGES_BEFORE_COMPACTION: usize = 48;
pub(crate) const KEEP_RECENT_MESSAGES_AFTER_COMPACTION: usize = 18;
pub(crate) const MAX_COMPACTION_SUMMARY_ITEMS: usize = 8;

// ── App ───────────────────────────────────────────────────────────────────────
pub(crate) struct App {
    pub messages: Vec<Message>,
    pub task: String,
    pub steps_taken: usize,
    pub max_steps: usize,
    pub llm_calling: bool,
    pub llm_stream_preview: String,
    pub llm_preview_shown: String,
    pub needs_agent_step: bool,
    pub running: bool,
    pub quit: bool,
    pub pending_confirm: Option<String>,
    pub pending_confirm_note: bool,
    pub task_events: Vec<Event>,
    pub final_summary: Option<String>,
    pub task_collapsed: bool,
    pub show_thinking: bool,
    pub paste_counter: usize,
    pub paste_chunks: Vec<PasteChunk>,
    pub mcp_registry: crate::tools::mcp::McpRegistry,
    pub mcp_discovery_rx:
        Option<std::sync::mpsc::Receiver<(crate::tools::mcp::McpRegistry, Vec<String>)>>,
    pub skills: Vec<Skill>,
    /// Base system prompt = SYSTEM_PROMPT + skills section.
    /// Used as the foundation when rebuilding the full prompt after MCP discovery.
    pub base_prompt: String,
    pub mode: Mode,
    pub ge_agent: Option<crate::consensus::subagent::GeSubagent>,
}

#[derive(Clone, Debug)]
pub(crate) struct PasteChunk {
    pub placeholder: String,
    pub content: String,
}

impl App {
    fn new() -> Self {
        let store = MemoryStore::new();
        let (mcp_registry, mcp_warnings) = crate::tools::mcp::McpRegistry::from_env();
        for warning in mcp_warnings {
            eprintln!("[mcp] {warning}");
        }
        let skills = discover_skills();
        // base_prompt = SYSTEM_PROMPT + skills section.
        // MCP tools are appended later after background discovery.
        let skills_section = skills_system_prompt(&skills);
        let base_prompt = format!("{}{skills_section}", build_system_prompt());
        // System prompt built without MCP tools; augmented after background discovery.
        let system_prompt = store.build_system_prompt(&base_prompt);
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
            show_thinking: true,
            paste_counter: 0,
            paste_chunks: Vec::new(),
            mcp_registry,
            mcp_discovery_rx: None,
            skills,
            base_prompt,
            mode: Mode::Normal,
            ge_agent: None,
        }
    }
}

pub(crate) enum LlmWorkerEvent {
    Delta(String),
    ThinkingDelta(String),
    Done(anyhow::Result<String>),
}

// ── Entry point ───────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let _ = dotenvy::dotenv();
    let http_client = build_http_client()?;
    let mut app = App::new();

    enable_raw_mode()?;
    execute!(io::stdout(), EnableBracketedPaste)?;
    let mut screen = Screen::new()?;

    // Display discovered skills below the banner.
    let skill_names: Vec<String> = app.skills.iter().map(|s| s.name.clone()).collect();
    if let Some(line) = format_skills_status_line(&skill_names) {
        screen.emit(&[line]);
    }

    // Start MCP discovery in background; results arrive via channel in run_loop.
    if app.mcp_registry.has_servers() {
        let registry = app.mcp_registry.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(registry.run_discovery());
        });
        app.mcp_discovery_rx = Some(rx);
    }

    let run_result = run_loop(&mut app, &mut screen, http_client).await;

    let _ = execute!(io::stdout(), DisableBracketedPaste);
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
        // Check if background MCP discovery has completed.
        let mcp_result = app
            .mcp_discovery_rx
            .as_ref()
            .and_then(|rx| rx.try_recv().ok());
        if let Some((registry, warnings)) = mcp_result {
            for w in &warnings {
                screen.emit(&[format!(
                    "  {}",
                    crossterm::style::Stylize::dark_yellow(w.as_str())
                )]);
            }
            app.mcp_registry = registry;
            // Rebuild system prompt now that tools are known.
            let base = app.mcp_registry.augment_system_prompt(&app.base_prompt);
            let new_prompt = MemoryStore::new().build_system_prompt(&base);
            if let Some(sys) = app.messages.first_mut() {
                sys.content = new_prompt;
            }
            // Display result below the banner.
            let status = app.mcp_registry.startup_status();
            if let Some(line) = format_mcp_status_line(&status.ok, &status.failed) {
                screen.emit(&[line]);
            }
            app.mcp_discovery_rx = None;
        }

        while let Ok(msg) = rx.try_recv() {
            match msg {
                LlmWorkerEvent::Delta(delta) => handle_llm_stream_delta(app, screen, &delta),
                LlmWorkerEvent::ThinkingDelta(chunk) => {
                    handle_llm_thinking_delta(app, screen, &chunk)
                }
                LlmWorkerEvent::Done(result) => {
                    app.llm_calling = false;
                    app.llm_stream_preview.clear();
                    app.llm_preview_shown.clear();
                    screen.status.clear();
                    process_llm_result(app, screen, result);
                }
            }
        }

        drain_ge_events(app, screen);

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
            let show_thinking = app.show_thinking;
            tokio::spawn(async move {
                let result = chat_stream_with(
                    &client,
                    &messages,
                    show_thinking,
                    |piece| {
                        let _ = tx_delta.try_send(LlmWorkerEvent::Delta(piece.to_string()));
                    },
                    |chunk| {
                        let _ = tx_delta.try_send(LlmWorkerEvent::ThinkingDelta(chunk.to_string()));
                    },
                )
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
                CEvent::Paste(text) => handle_paste(app, screen, &text),
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

// ── Key handling ──────────────────────────────────────────────────────────────

fn handle_key(app: &mut App, screen: &mut Screen, key: KeyCode, modifiers: KeyModifiers) -> bool {
    if key == KeyCode::Char('c') && modifiers.contains(KeyModifiers::CONTROL) {
        return true;
    }
    if is_ge_mode(app.mode)
        && modifiers.is_empty()
        && screen.input.trim().is_empty()
        && matches!(key, KeyCode::Char('q') | KeyCode::Char('Q'))
    {
        if let Some(agent) = app.ge_agent.as_ref() {
            if agent.hard_exit() {
                screen.emit(&[
                    "  GE hard exit requested (q). Stopping current executor...".to_string()
                ]);
            } else {
                app.ge_agent = None;
                app.mode = Mode::Normal;
                screen.emit(&["  GE channel disconnected.".to_string()]);
            }
        } else {
            app.mode = Mode::Normal;
            screen.emit(&["  GE is already disabled.".to_string()]);
        }
        return false;
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
    if key == KeyCode::Tab
        && modifiers.is_empty()
        && screen.confirm_selected.is_none()
        && !app.pending_confirm_note
    {
        app.show_thinking = !app.show_thinking;
        let label = if app.show_thinking {
            format!("{} {}", "Thinking:".dark_grey(), "ON".green().bold())
        } else {
            format!("{} {}", "Thinking:".dark_grey(), "OFF".yellow().bold())
        };
        if !app.llm_calling {
            screen.status = label;
            screen.refresh();
        }
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
                let note = expand_input_text(app, &screen.input).trim().to_string();
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
                clear_input_buffer(app, screen);
                screen.input_focused = true;
                screen.refresh();
            }
            KeyCode::Esc if modifiers.is_empty() => exit_confirm_note_mode(app, screen, true),
            KeyCode::Backspace => {
                pop_input_tail(app, screen);
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
                    let task = expand_input_text(app, &screen.input).trim().to_string();
                    if !task.is_empty() {
                        clear_input_buffer(app, screen);
                        submit_user_input(app, screen, task);
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
                    pop_input_tail(app, screen);
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
        match key {
            KeyCode::Char(c) if modifiers.is_empty() || modifiers == KeyModifiers::SHIFT => {
                screen.input_focused = true;
                screen.input.push(c);
                screen.refresh();
            }
            KeyCode::Backspace => {
                screen.input_focused = true;
                pop_input_tail(app, screen);
                screen.refresh();
            }
            _ => {}
        }
    }

    false
}

fn submit_user_input(app: &mut App, screen: &mut Screen, task: String) {
    match try_submit_user_input(app, screen, &task) {
        Ok(()) => {}
        Err(e) => {
            screen.emit(&[format!(
                "  {}",
                crossterm::style::Stylize::red(format!("GE error: {e}"))
            )]);
        }
    }
}

fn try_submit_user_input(app: &mut App, screen: &mut Screen, task: &str) -> anyhow::Result<()> {
    if let Some(rest) = parse_ge_command(task) {
        let rest = rest.trim();
        if rest == "退出" || rest.eq_ignore_ascii_case("exit") {
            if let Some(agent) = app.ge_agent.as_ref() {
                if agent.hard_exit() {
                    screen.emit(&[
                        "  GE hard exit requested. Stopping current executor...".to_string()
                    ]);
                } else {
                    app.ge_agent = None;
                    app.mode = Mode::Normal;
                    screen.emit(&["  GE channel disconnected.".to_string()]);
                }
            } else {
                app.mode = Mode::Normal;
                screen.emit(&["  GE is already disabled.".to_string()]);
            }
            return Ok(());
        }
        if rest == "细化todo" || rest.eq_ignore_ascii_case("replan") {
            if let Some(agent) = app.ge_agent.as_ref() {
                if !agent.send(crate::consensus::subagent::GeAgentCommand::ReplanTodos) {
                    app.ge_agent = None;
                    app.mode = Mode::Normal;
                    screen.emit(&["  GE channel disconnected.".to_string()]);
                }
            } else {
                screen.emit(&["  GE is not active. Start with `GE <goal>` first.".to_string()]);
            }
            return Ok(());
        }

        if app.running {
            finish(
                app,
                screen,
                "Stopped current task to enter GE mode.".to_string(),
            );
        }
        let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
        if app.ge_agent.is_none() {
            let agent = crate::consensus::subagent::GeSubagent::start(cwd, rest)?;
            app.ge_agent = Some(agent);
            drain_ge_events(app, screen);
            screen.emit(&["  GE controls: press `q` for hard exit.".to_string()]);
        } else {
            screen.emit(&["  GE already active. Use `GE 退出` to leave this mode.".to_string()]);
        }
        return Ok(());
    }

    if app.mode == Mode::GeInterview {
        if let Some(agent) = app.ge_agent.as_ref() {
            screen.emit(&["  GE: input received.".to_string()]);
            if !agent.send(crate::consensus::subagent::GeAgentCommand::InterviewReply(
                task.to_string(),
            )) {
                app.ge_agent = None;
                app.mode = Mode::Normal;
                screen.emit(&["  GE channel disconnected.".to_string()]);
            } else {
                screen.status = "GE: processing interview input...".to_string();
                screen.refresh();
                drain_ge_events(app, screen);
            }
        } else {
            app.mode = Mode::Normal;
            screen.emit(&["  GE session not found. Start again with `GE`.".to_string()]);
        }
        return Ok(());
    }

    if app.mode == Mode::GeRun || app.mode == Mode::GeIdle {
        screen.emit(&["  GE mode is active. Use `GE 退出` to return to normal mode.".to_string()]);
        return Ok(());
    }

    start_task(app, screen, task.to_string());
    Ok(())
}

fn parse_ge_command(task: &str) -> Option<&str> {
    let trimmed = task.trim();
    if trimmed.len() < 2 {
        return None;
    }
    let mut chars = trimmed.chars();
    let (Some(c1), Some(c2)) = (chars.next(), chars.next()) else {
        return None;
    };
    if !c1.eq_ignore_ascii_case(&'g') || !c2.eq_ignore_ascii_case(&'e') {
        return None;
    }
    let rest = chars.as_str();
    if let Some(first) = rest.chars().next()
        && first.is_ascii_alphabetic()
    {
        return None;
    }
    Some(rest.trim_start_matches(|c: char| c.is_whitespace() || c == '\u{3000}'))
}

fn drain_ge_events(app: &mut App, screen: &mut Screen) {
    let mut drop_agent = false;
    let mut disable_screen_status = false;
    let mut disconnected = false;

    loop {
        let Some(agent) = app.ge_agent.as_ref() else {
            break;
        };
        match agent.try_recv() {
            Ok(crate::consensus::subagent::GeAgentEvent::OutputLines(lines)) => {
                if !lines.is_empty() {
                    if strip_ansi(&screen.status).starts_with("GE: processing interview input") {
                        screen.status.clear();
                    }
                    let styled = stylize_ge_lines(&lines);
                    screen.emit(&styled);
                }
            }
            Ok(crate::consensus::subagent::GeAgentEvent::ModeChanged(mode)) => {
                app.mode = mode;
                if app.mode != Mode::GeInterview
                    && strip_ansi(&screen.status).starts_with("GE: processing interview input")
                {
                    screen.status.clear();
                }
            }
            Ok(crate::consensus::subagent::GeAgentEvent::Exited) => {
                drop_agent = true;
                app.mode = Mode::Normal;
                disable_screen_status = true;
                break;
            }
            Ok(crate::consensus::subagent::GeAgentEvent::Error(err)) => {
                screen.emit(&[format!(
                    "  {}",
                    crossterm::style::Stylize::red(format!("GE error: {err}"))
                )]);
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => break,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                drop_agent = true;
                app.mode = Mode::Normal;
                disconnected = true;
                break;
            }
        }
    }

    if drop_agent {
        app.ge_agent = None;
        if disconnected {
            screen.emit(&["  GE subagent disconnected.".to_string()]);
        }
    }
    if disable_screen_status {
        app.running = false;
        app.needs_agent_step = false;
        app.pending_confirm = None;
        app.pending_confirm_note = false;
        screen.status.clear();
        screen.refresh();
    }
}

fn is_ge_mode(mode: Mode) -> bool {
    matches!(mode, Mode::GeInterview | Mode::GeRun | Mode::GeIdle)
}

fn stylize_ge_lines(lines: &[String]) -> Vec<String> {
    lines.iter().map(|line| stylize_ge_line(line)).collect()
}

fn stylize_ge_line(line: &str) -> String {
    let trimmed = line.trim_start();
    if let Some(text) = line.strip_prefix("    1) ") {
        return format!("    {}) {}", "1".cyan().bold(), text);
    }
    if let Some(text) = line.strip_prefix("    2) ") {
        return format!("    {}) {}", "2".cyan().bold(), text);
    }
    if let Some(text) = line.strip_prefix("    3) ") {
        return format!("    {}) {}", "3".cyan().bold(), text);
    }
    if trimmed.starts_with("GE Q") || trimmed.starts_with("GE Clarify") {
        return line.cyan().bold().to_string();
    }
    if trimmed.starts_with("Reply with 1, 2, or 3.") || trimmed.starts_with("Reply with 1/2/3") {
        return line.dark_grey().to_string();
    }
    if trimmed.starts_with("GE controls:") || trimmed.starts_with("Audit log:") {
        return line.dark_grey().to_string();
    }
    if trimmed == "GE: input received." {
        return line.dark_grey().to_string();
    }
    if trimmed.starts_with("GE: Planning") {
        return line.dark_yellow().bold().to_string();
    }
    if trimmed.starts_with("GE: Working on current todo")
        || trimmed.starts_with("GE: still running current step")
    {
        return line.dark_yellow().to_string();
    }
    if trimmed.starts_with("GE ->") && trimmed.contains("prompt:") {
        return line.cyan().to_string();
    }
    if trimmed.starts_with("GE <-") && trimmed.contains("result:") {
        if trimmed.contains("status=success") {
            return line.green().to_string();
        }
        return line.dark_yellow().to_string();
    }
    if trimmed.starts_with('T') && trimmed.contains("->") && trimmed.contains("prompt:") {
        return line.cyan().to_string();
    }
    if trimmed.starts_with('T') && trimmed.contains("<-") && trimmed.contains("result:") {
        if trimmed.contains("status=success") {
            return line.green().to_string();
        }
        return line.dark_yellow().to_string();
    }
    if trimmed.starts_with("...(truncated in console view;") {
        return line.dark_grey().to_string();
    }
    if trimmed.starts_with("GE: clarification complete.")
        || trimmed.starts_with("GE interview complete;")
        || trimmed.ends_with(" checked.")
    {
        return line.green().bold().to_string();
    }
    if trimmed.starts_with("GE running ") {
        return line.cyan().bold().to_string();
    }
    if trimmed.starts_with('T') && trimmed.contains(" started.") {
        return line.cyan().to_string();
    }
    if trimmed.starts_with('T') && trimmed.contains(" deferred:") {
        return line.dark_yellow().to_string();
    }
    if trimmed.starts_with('T') && trimmed.contains(" cancelled.") {
        return line.dark_yellow().bold().to_string();
    }
    if trimmed.starts_with("GE mode disabled.")
        || trimmed.starts_with("GE disabled.")
        || trimmed.starts_with("GE subagent disconnected.")
    {
        return line.dark_grey().to_string();
    }
    if trimmed.starts_with("GE:") {
        return line.dark_yellow().to_string();
    }
    line.to_string()
}

fn handle_paste(app: &mut App, screen: &mut Screen, pasted: &str) {
    if pasted.is_empty() {
        return;
    }

    if screen.confirm_selected.is_some() {
        begin_confirm_note_mode(app, screen, None);
        if !app.pending_confirm_note {
            return;
        }
    }

    if !screen.input_focused {
        screen.input_focused = true;
    }

    append_paste_input(app, screen, pasted);
    screen.refresh();
}

fn append_paste_input(app: &mut App, screen: &mut Screen, pasted: &str) {
    let multiline = pasted.contains('\n') || pasted.contains('\r');
    let long_text = pasted.chars().count() > 120;

    if multiline || long_text {
        app.paste_counter += 1;
        let placeholder = build_paste_placeholder(app.paste_counter, pasted);
        if !screen.input.is_empty() && !screen.input.ends_with(char::is_whitespace) {
            screen.input.push(' ');
        }
        screen.input.push_str(&placeholder);
        app.paste_chunks.push(PasteChunk {
            placeholder,
            content: pasted.to_string(),
        });
        return;
    }

    for ch in pasted.chars() {
        if ch != '\r' && ch != '\n' {
            screen.input.push(ch);
        }
    }
}

fn build_paste_placeholder(index: usize, pasted: &str) -> String {
    let normalized = pasted.replace("\r\n", "\n").replace('\r', "\n");
    let line_count = if normalized.is_empty() {
        0
    } else {
        normalized.split('\n').count()
    };
    if line_count > 1 {
        format!(
            "[Pasted text #{} +{} lines]",
            index,
            line_count.saturating_sub(1)
        )
    } else {
        format!(
            "[Pasted text #{} +{} chars]",
            index,
            normalized.chars().count()
        )
    }
}

fn expand_input_text(app: &App, input: &str) -> String {
    let mut expanded = input.to_string();
    for chunk in &app.paste_chunks {
        expanded = expanded.replace(&chunk.placeholder, &chunk.content);
    }
    expanded
}

fn clear_input_buffer(app: &mut App, screen: &mut Screen) {
    screen.input.clear();
    app.paste_chunks.clear();
}

fn pop_input_tail(app: &mut App, screen: &mut Screen) {
    if screen.input.is_empty() {
        return;
    }

    let mut matched_idx = None;
    for (idx, chunk) in app.paste_chunks.iter().enumerate().rev() {
        if screen.input.ends_with(&chunk.placeholder) {
            matched_idx = Some(idx);
            break;
        }
    }

    if let Some(idx) = matched_idx {
        let placeholder_len = app.paste_chunks[idx].placeholder.len();
        let new_len = screen.input.len().saturating_sub(placeholder_len);
        screen.input.truncate(new_len);
        app.paste_chunks.remove(idx);
        if screen.input.ends_with(' ') {
            screen.input.pop();
        }
        return;
    }

    screen.input.pop();
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
    clear_input_buffer(app, screen);
    if let Some(c) = first_char {
        screen.input.push(c);
    }
    screen.refresh();
}

fn exit_confirm_note_mode(app: &mut App, screen: &mut Screen, back_to_menu: bool) {
    app.pending_confirm_note = false;
    clear_input_buffer(app, screen);
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

#[cfg(test)]
mod tests {
    use super::parse_ge_command;

    #[test]
    fn parse_ge_command_supports_exit_variants() {
        assert_eq!(parse_ge_command("GE 退出"), Some("退出"));
        assert_eq!(parse_ge_command("ge exit"), Some("exit"));
        assert_eq!(parse_ge_command("GE退出"), Some("退出"));
    }

    #[test]
    fn parse_ge_command_rejects_regular_words() {
        assert_eq!(parse_ge_command("get status"), None);
        assert_eq!(parse_ge_command("general"), None);
    }
}
