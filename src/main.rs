mod agent;
mod consensus;
mod memory;
mod tools;
mod types;
mod ui;

use std::{io, time::Duration};

use agent::{
    provider::{Message, build_http_client, chat_stream_with},
    react::{build_assistant_context, build_system_prompt},
    executor::{
        handle_llm_stream_delta, handle_llm_thinking_delta,
        maybe_flush_and_compact_before_call, process_llm_result, start_task,
    },
};
use crossterm::{
    event::{
        self, DisableBracketedPaste, EnableBracketedPaste, Event as CEvent, KeyEventKind,
    },
    execute,
    style::Print,
    terminal::{disable_raw_mode, enable_raw_mode},
};
use memory::store::MemoryStore;
use tokio::sync::mpsc;
use tools::skills::{Skill, discover_skills, skills_system_prompt};
use types::{Event, Mode};
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
    pub mode: Mode,
    pub ge_agent: Option<crate::consensus::subagent::GeSubagent>,
    pub todo_items: Vec<crate::types::TodoItem>,
}

#[derive(Clone, Debug)]
pub(crate) struct PasteChunk {
    pub placeholder: String,
    pub content: String,
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
        // messages[1] = 固定 assistant 提示词，永久保留。
        // 第一次请求时把当日记忆拼在后面一起发；收到回复后截断，只留固定部分。
        let base_ctx = build_assistant_context();
        let memory = store.build_memory_message();
        let has_memory_message = memory.is_some();
        let first_ctx = match memory {
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
            mode: Mode::Normal,
            ge_agent: None,
            todo_items: Vec::new(),
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
    // Create ~/.goldbot/.env from template if it doesn't exist yet.
    ensure_dot_env();
    let _ = dotenvy::from_path(crate::tools::mcp::goldbot_home_dir().join(".env"));
    let http_client = build_http_client()?;
    let mut app = App::new();

    enable_raw_mode()?;
    execute!(io::stdout(), EnableBracketedPaste)?;
    let mut screen = Screen::new()?;

    // Warn if required API key is missing and show .env path.
    if std::env::var("BIGMODEL_API_KEY").is_err() {
        let env_path = crate::tools::mcp::goldbot_home_dir().join(".env");
        screen.emit(&[
            format!(
                "  {} BIGMODEL_API_KEY 未配置，请编辑: {}",
                crossterm::style::Stylize::yellow("⚠"),
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

        //接收llm worker的消息，更新界面状态或处理结果
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

        if app.running && app.pending_confirm.is_none() && app.needs_agent_executor && !app.llm_calling
        {
            //在compact之前写入长期记忆把
            maybe_flush_and_compact_before_call(app, screen);
            app.needs_agent_executor = false;
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
