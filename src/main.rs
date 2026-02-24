mod agent;
mod consensus;
mod memory;
mod tools;
mod types;
mod ui;

use std::{io, time::Duration};

use agent::{
    executor::{
        handle_llm_stream_delta, handle_llm_thinking_delta, maybe_flush_and_compact_before_call,
        process_llm_result, start_task,
    },
    provider::{LlmBackend, Message, build_http_client},
    react::{build_assistant_context, build_system_prompt},
};
use crossterm::{
    event::{self, DisableBracketedPaste, EnableBracketedPaste, Event as CEvent, KeyEventKind},
    execute,
    style::Print,
    terminal::{disable_raw_mode, enable_raw_mode},
};
use memory::store::MemoryStore;
use tokio::sync::mpsc;
use tools::skills::{Skill, discover_skills, skills_system_prompt};
use types::{AssistMode, Event, Mode};
use ui::ge::drain_ge_events;
use ui::input::{handle_key, handle_paste};
use ui::screen::{Screen, format_mcp_status_line, format_skills_status_line};

pub(crate) const MAX_MESSAGES_BEFORE_COMPACTION: usize = 48;
pub(crate) const KEEP_RECENT_MESSAGES_AFTER_COMPACTION: usize = 18;
pub(crate) const MAX_COMPACTION_SUMMARY_ITEMS: usize = 8;

// ── App ───────────────────────────────────────────────────────────────────────
pub(crate) struct App {
    pub messages: Vec<Message>,
    pub task: String,
    pub steps_taken: usize,
    pub llm_calling: bool,
    pub llm_stream_preview: String,
    pub llm_preview_shown: String,
    pub needs_agent_executor: bool,
    /// User pressed Esc to stop the active LLM loop; run_loop should abort the in-flight worker.
    pub interrupt_llm_loop_requested: bool,
    /// Next normal user input should be sent as an in-conversation interjection, not a new task.
    pub interjection_mode: bool,
    pub running: bool,
    pub quit: bool,
    pub pending_confirm: Option<String>,
    /// File hint accompanying `pending_confirm`, forwarded to `execute_command` for diff capture.
    pub pending_confirm_file: Option<String>,
    pub pending_confirm_note: bool,
    pub task_events: Vec<Event>,
    pub final_summary: Option<String>,
    pub task_collapsed: bool,
    pub show_thinking: bool,
    pub paste_counter: usize,
    pub paste_chunks: Vec<PasteChunk>,
    /// Pending question from LLM: (question text, raw options vec).
    pub pending_question: Option<(String, Vec<String>)>,
    /// True when user is typing a free-text answer to a question/plan supplement.
    pub answering_question: bool,
    pub mcp_registry: crate::tools::mcp::McpRegistry,
    pub mcp_discovery_rx:
        Option<std::sync::mpsc::Receiver<(crate::tools::mcp::McpRegistry, Vec<String>)>>,
    pub skills: Vec<Skill>,
    /// Base system prompt = SYSTEM_PROMPT + skills section.
    /// Used as the foundation when rebuilding the full prompt after MCP discovery.
    pub base_prompt: String,
    /// True when the memory assistant message is still sitting at messages[1].
    /// Cleared (and the message removed) after the first LLM response is received.
    pub has_memory_message: bool,
    /// Startup memory text appended to messages[1] until the first LLM response arrives.
    pub assistant_memory_suffix: Option<String>,
    pub mode: Mode,
    pub assist_mode: AssistMode,
    pub workspace: std::path::PathBuf,
    pub backend: LlmBackend,
    pub ge_agent: Option<crate::consensus::subagent::GeSubagent>,
    pub todo_items: Vec<crate::types::TodoItem>,
    /// True when launched with -p: auto-quit after task finishes, print final_summary to stdout.
    pub headless: bool,

    // ── @ file picker ──────────────────────────────────────────────────────────
    pub at_file: AtFilePickerState,
}

#[derive(Clone, Debug)]
pub(crate) struct PasteChunk {
    pub placeholder: String,
    pub content: String,
}

#[derive(Clone, Debug)]
pub(crate) struct AtFileChunk {
    /// The placeholder token inserted into the input, e.g. `[@src/main.rs]`.
    pub placeholder: String,
    /// Resolved path to the file (relative to workspace).
    pub path: std::path::PathBuf,
}

#[derive(Debug, Default)]
pub(crate) struct AtFilePickerState {
    /// When `Some`, the @ file picker is active; value is the search query after `@`.
    pub query: Option<String>,
    /// Byte position in `screen.input` immediately after the `@` that opened the picker.
    pub at_pos: usize,
    /// Current file candidates matching `query`.
    pub candidates: Vec<std::path::PathBuf>,
    /// Index of the selected candidate in `candidates`.
    pub sel: usize,
    /// Files attached to the current input via @ selection, waiting to be sent with the message.
    pub chunks: Vec<AtFileChunk>,
}

impl App {
    fn new() -> Self {
        let store = MemoryStore::new();
        // 启动时清理超过 15 天的短期记忆文件
        store.cleanup_old_short_term();
        let (mcp_registry, mcp_warnings) = crate::tools::mcp::McpRegistry::from_env();
        for warning in mcp_warnings {
            eprintln!("[mcp] {warning}");
        }
        let skills = discover_skills();
        // base_prompt = SYSTEM_PROMPT + skills section.
        // MCP tools are appended later after background discovery.
        let skills_section = skills_system_prompt(&skills);
        let base_prompt = format!("{}{skills_section}", build_system_prompt());

        // Determine workspace: GOLDBOT_WORKSPACE env var, or current directory.
        let workspace = std::env::var("GOLDBOT_WORKSPACE")
            .ok()
            .and_then(|p| {
                std::fs::canonicalize(&p)
                    .ok()
                    .or_else(|| Some(std::path::PathBuf::from(p)))
            })
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_else(|| std::path::PathBuf::from("."));
        // chdir into workspace so all shell commands run relative to it.
        let _ = std::env::set_current_dir(&workspace);

        // messages[1] = 固定 assistant 提示词，永久保留。
        // 第一次请求时把当日记忆拼在后面一起发；收到回复后截断，只留固定部分。
        let assist_mode = AssistMode::Off;
        let base_ctx = build_assistant_context(&workspace, assist_mode);
        let assistant_memory_suffix = store.build_memory_message();
        let has_memory_message = assistant_memory_suffix.is_some();
        let first_ctx = match &assistant_memory_suffix {
            Some(mem) => format!("{base_ctx}\n\n{mem}"),
            None => base_ctx,
        };
        let messages = vec![
            Message::system(base_prompt.clone()),
            Message::assistant(first_ctx),
        ];
        Self {
            messages,

            task: String::new(),
            steps_taken: 0,
            llm_calling: false,
            llm_stream_preview: String::new(),
            llm_preview_shown: String::new(),
            needs_agent_executor: false,
            interrupt_llm_loop_requested: false,
            interjection_mode: false,
            running: false,
            quit: false,
            pending_confirm: None,
            pending_confirm_file: None,
            pending_confirm_note: false,
            task_events: Vec::new(),
            final_summary: None,
            task_collapsed: false,
            show_thinking: true,
            paste_counter: 0,
            paste_chunks: Vec::new(),
            pending_question: None,
            answering_question: false,
            mcp_registry,
            mcp_discovery_rx: None,
            skills,
            base_prompt,
            has_memory_message,
            assistant_memory_suffix,
            mode: Mode::Normal,
            assist_mode,
            workspace,
            ge_agent: None,
            todo_items: Vec::new(),
            backend: LlmBackend::from_env(),
            headless: false,
            at_file: AtFilePickerState::default(),
        }
    }

    pub(crate) fn rebuild_assistant_context_message(&mut self) {
        let mut base_ctx = build_assistant_context(&self.workspace, self.assist_mode);
        if self.has_memory_message
            && let Some(mem) = &self.assistant_memory_suffix
        {
            base_ctx.push_str("\n\n");
            base_ctx.push_str(mem);
        }
        if let Some(msg) = self.messages.get_mut(1) {
            msg.content = base_ctx;
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
    // 在 Windows 老终端（CMD/PowerShell conhost）里强制 UTF-8 代码页，
    // 否则 ❯ 等 Unicode 字符会显示乱码。
    #[cfg(windows)]
    {
        unsafe extern "system" {
            fn SetConsoleOutputCP(wCodePageID: u32) -> i32;
            fn SetConsoleCP(wCodePageID: u32) -> i32;
        }
        unsafe {
            SetConsoleOutputCP(65001);
            SetConsoleCP(65001);
        }
    }

    let (cli_prompt, cli_yes) = parse_cli_args();

    // Create ~/.goldbot/.env from template if it doesn't exist yet.
    ensure_dot_env();
    let _ = dotenvy::from_path(crate::tools::mcp::goldbot_home_dir().join(".env"));
    let http_client = build_http_client()?;
    let mut app = App::new();

    enable_raw_mode()?;
    execute!(io::stdout(), EnableBracketedPaste)?;
    let mut screen = Screen::new()?;
    screen.workspace = app.workspace.to_string_lossy().replace('\\', "/");
    screen.assist_mode = app.assist_mode;

    // Warn if required API key is missing and show .env path.
    if app.backend.api_key_missing() {
        let env_path = crate::tools::mcp::goldbot_home_dir().join(".env");
        screen.emit(&[
            format!(
                "  {} {} 未配置，请编辑: {}",
                crossterm::style::Stylize::yellow(crate::ui::symbols::Symbols::current().warning),
                app.backend.required_key_name(),
                env_path.display()
            ),
            String::new(),
        ]);
    }

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

    let run_result = run_loop(&mut app, &mut screen, http_client, cli_prompt, cli_yes).await;

    let _ = execute!(io::stdout(), DisableBracketedPaste);
    let _ = disable_raw_mode();
    let _ = execute!(io::stdout(), crossterm::cursor::Show, Print("\r\n"));
    // headless 模式：清除当前行残留的 TUI 内容，然后把最终答案打印到 stdout
    if app.headless {
        if let Some(summary) = &app.final_summary {
            let _ = execute!(
                io::stdout(),
                crossterm::terminal::Clear(crossterm::terminal::ClearType::CurrentLine),
                Print("\r")
            );
            println!("{summary}");
        }
    }
    run_result
}

async fn run_loop(
    app: &mut App,
    screen: &mut Screen,
    http_client: reqwest::Client,
    initial_task: Option<String>,
    auto_accept: bool,
) -> anyhow::Result<()> {
    let startup_task = initial_task.or_else(|| std::env::var("GOLDBOT_TASK").ok());
    // -y / --yes 开启自动接受非 Block 命令
    if auto_accept {
        app.assist_mode = AssistMode::AcceptEdits;
        app.rebuild_assistant_context_message();
        screen.assist_mode = app.assist_mode;
    }
    if let Some(task) = startup_task {
        app.headless = true;
        start_task(app, screen, task);
    }

    let (tx, mut rx) = mpsc::channel::<LlmWorkerEvent>(64);
    let mut last_spinner_refresh = std::time::Instant::now();
    let mut llm_task_handle: Option<tokio::task::JoinHandle<()>> = None;

    loop {
        // 动态检查能用的mcp加入系统提示，并显示当前可用的mcp工具状态
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
            // Rebuild system prompt now that tools are known (memory stays in assistant message).
            let new_prompt = app.mcp_registry.augment_system_prompt(&app.base_prompt);
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

        if app.interrupt_llm_loop_requested {
            interrupt_active_llm_loop(app, screen, &mut llm_task_handle);
        }

        //接收llm worker的消息，更新界面状态或处理结果
        while let Ok(msg) = rx.try_recv() {
            match msg {
                LlmWorkerEvent::Delta(delta) => handle_llm_stream_delta(app, screen, &delta),
                LlmWorkerEvent::ThinkingDelta(chunk) => {
                    handle_llm_thinking_delta(app, screen, &chunk)
                }
                LlmWorkerEvent::Done(result) => {
                    llm_task_handle = None;
                    app.llm_calling = false;
                    app.llm_stream_preview.clear();
                    app.llm_preview_shown.clear();
                    screen.status.clear();
                    process_llm_result(app, screen, result);
                }
            }
        }

        drain_ge_events(app, screen);

        if app.running
            && app.pending_confirm.is_none()
            && app.needs_agent_executor
            && !app.llm_calling
        {
            //在compact之前写入长期记忆把
            maybe_flush_and_compact_before_call(app, screen);
            app.needs_agent_executor = false;
            app.llm_calling = true;
            app.llm_stream_preview.clear();
            app.llm_preview_shown.clear();
            screen.status = "Thinking...".to_string();
            screen.refresh();

            let tx_done = tx.clone();
            let tx_delta = tx.clone();
            let client = http_client.clone();
            let messages = app.messages.clone();
            let show_thinking = app.show_thinking;
            let backend = app.backend;
            llm_task_handle = Some(tokio::spawn(async move {
                let result = backend
                    .chat_stream_with(
                        &client,
                        &messages,
                        show_thinking,
                        |piece| {
                            let _ = tx_delta.try_send(LlmWorkerEvent::Delta(piece.to_string()));
                        },
                        |chunk| {
                            let _ =
                                tx_delta.try_send(LlmWorkerEvent::ThinkingDelta(chunk.to_string()));
                        },
                    )
                    .await;
                let _ = tx_done.send(LlmWorkerEvent::Done(result)).await;
            }));
        }

        // 同步运行状态，每 400ms 推进一次 spinner 帧，避免频繁刷屏闪烁
        screen.is_running = app.running;
        if app.running && last_spinner_refresh.elapsed() >= Duration::from_millis(400) {
            screen.spinner_tick = screen.spinner_tick.wrapping_add(1);
            screen.refresh_status_only();
            last_spinner_refresh = std::time::Instant::now();
        }

        //处理键盘事件，包括普通按键和粘贴事件
        if event::poll(Duration::from_millis(50))? {
            // Drain all immediately-available events.
            let mut events = vec![event::read()?];
            while event::poll(Duration::ZERO)? {
                events.push(event::read()?);
            }

            for ev in events {
                match ev {
                    CEvent::Key(k) if k.kind == KeyEventKind::Press => {
                        if handle_key(app, screen, k.code, k.modifiers) {
                            app.quit = true;
                            break;
                        }
                    }
                    CEvent::Paste(text) => handle_paste(app, screen, &text),
                    _ => {}
                }
            }
        }

        if app.quit {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    Ok(())
}

// ── Utilities ─────────────────────────────────────────────────────────────────

fn interrupt_active_llm_loop(
    app: &mut App,
    screen: &mut Screen,
    llm_task_handle: &mut Option<tokio::task::JoinHandle<()>>,
) {
    app.interrupt_llm_loop_requested = false;
    app.running = false;
    app.needs_agent_executor = false;
    app.llm_calling = false;
    app.llm_stream_preview.clear();
    app.llm_preview_shown.clear();
    if let Some(handle) = llm_task_handle.take() {
        handle.abort();
    }
    screen.refresh();
}

/// Parse CLI arguments, returning (prompt, auto_accept).
/// Supported flags:
///   -p / --prompt <text>   Initial chat message to send on startup.
///   -y / --yes             Auto-accept all Confirm-level commands (non-Block).
fn parse_cli_args() -> (Option<String>, bool) {
    let args: Vec<String> = std::env::args().collect();
    let mut prompt = None;
    let mut yes = false;
    let mut i = 1;
    while i < args.len() {
        if (args[i] == "-p" || args[i] == "--prompt") && i + 1 < args.len() {
            prompt = Some(args[i + 1].clone());
            i += 2;
        } else if args[i] == "-y" || args[i] == "--yes" {
            yes = true;
            i += 1;
        } else {
            i += 1;
        }
    }
    (prompt, yes)
}

/// If `~/.goldbot/.env` doesn't exist, create it from the bundled template.
fn ensure_dot_env() {
    let home = crate::tools::mcp::goldbot_home_dir();
    let env_path = home.join(".env");
    if env_path.exists() {
        return;
    }
    let _ = std::fs::create_dir_all(&home);
    let _ = std::fs::write(&env_path, include_str!("../.env.example"));
}
