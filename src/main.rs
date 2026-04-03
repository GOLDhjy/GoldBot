mod agent;
mod cli;
mod consensus;
mod memory;
mod tools;
mod types;
mod ui;

use std::{
    io,
    sync::{
        Arc,
        atomic::AtomicBool,
    },
    time::Duration,
};

use agent::{
    dag::DagResult,
    executor::{
        LlmWorkerEvent, ShellExecResult,
        handle_llm_stream_delta, handle_llm_thinking_delta,
        interrupt_active_llm_loop,
        maybe_spawn_llm_worker, perform_manual_compact,
        poll_dag_result, poll_shell_exec_result,
        process_llm_result, refresh_llm_status,
        should_run_pending_manual_compact, shutdown_background_work,
        start_task, sync_context_budget,
    },
    provider::{LlmBackend, Message, build_http_client},
    react::{build_system_prompt, build_workspace_context},
};
use crossterm::{
    event::{DisableBracketedPaste, EnableBracketedPaste},
    execute,
    style::Print,
    terminal::{disable_raw_mode, enable_raw_mode},
};
use memory::Session;
use memory::project::init_workspace;
use tokio::sync::mpsc;
use tools::command::{Command as UserCommand, discover_commands};
use tools::skills::{Skill, discover_skills, skills_system_prompt};
use types::{AssistMode, Event, InputQueue, Mode};
use ui::ge::drain_ge_events;
use ui::input::handle_terminal_events;
use ui::screen::{Screen, format_mcp_status_line, format_skills_status_line};

pub(crate) const KEEP_RECENT_MESSAGES_AFTER_COMPACTION: usize = 18;
pub(crate) const MAX_COMPACTION_SUMMARY_ITEMS: usize = 8;

// ── App ───────────────────────────────────────────────────────────────────────
pub(crate) struct App {
    /// 发给 LLM 的协议级对话轨迹。
    /// 这里保存用户/assistant 原始消息，以及 synthetic 的 tool result 回灌消息。
    pub messages: Vec<Message>,
    pub task: String,
    pub steps_taken: usize,
    pub llm_calling: bool,
    /// Start time of the current in-flight LLM request (for status elapsed display).
    pub llm_call_started_at: Option<std::time::Instant>,
    /// Start time of the current task (persists across multiple LLM/tool rounds until final).
    pub task_started_at: Option<std::time::Instant>,
    /// Total elapsed time of the last finished task, for post-final UI display.
    pub last_task_elapsed: Option<std::time::Duration>,
    pub llm_stream_preview: String,
    pub llm_preview_shown: String,
    pub needs_agent_executor: bool,
    pub shell_task_running: bool,
    pub shell_exec_rx: Option<tokio::sync::mpsc::UnboundedReceiver<ShellExecResult>>,
    pub dag_task_running: bool,
    pub dag_result_rx: Option<tokio::sync::oneshot::Receiver<anyhow::Result<DagResult>>>,
    pub dag_progress_rx: Option<tokio::sync::mpsc::UnboundedReceiver<agent::dag::NodeProgress>>,
    pub dag_cancel_flag: Arc<AtomicBool>,
    /// `task_events` 中 DAG 树 ToolCall 事件的索引，用于原地刷新展示。
    pub dag_tree_event_idx: Option<usize>,
    /// node_id -> (success, elapsed_secs)
    pub dag_node_done: std::collections::HashMap<String, (bool, f64)>,
    /// Snapshot of graph nodes for tree rebuilding on progress updates
    pub dag_graph_nodes: Vec<crate::agent::sub_agent::TaskNode>,
    /// Snapshot of output_nodes for tree rebuilding
    pub dag_output_nodes: Vec<String>,
    /// User pressed Esc to stop the active LLM loop; run_loop should abort the in-flight worker.
    pub interrupt_llm_loop_requested: bool,
    /// Next normal user input should be sent as an in-conversation interjection, not a new task.
    pub interjection_mode: bool,
    pub running: bool,
    pub quit: bool,
    pub pending_confirm: Option<String>,
    pub pending_confirm_note: bool,
    /// 当前阶段摘要。
    /// 可由 LLM 的 `phase` 工具显式设置，也可由运行时自动生成用于中间进度展示。
    pub current_phase_summary: Option<String>,
    /// 当前任务的展示事件日志，仅供 TUI 渲染。
    /// 这里允许折叠、美化、压缩，绝不能作为 LLM 上下文来源。
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
    /// When Some, input is treated as an API key value for this env var name.
    pub pending_api_key_name: Option<String>,
    pub message_queue: InputQueue,
    pub mcp_registry: crate::tools::mcp::McpRegistry,
    pub mcp_discovery_rx:
        Option<std::sync::mpsc::Receiver<(crate::tools::mcp::McpRegistry, Vec<String>)>>,
    pub skills: Vec<Skill>,
    /// Base system prompt = SYSTEM_PROMPT + skills section.
    /// Used as the foundation when rebuilding the full prompt after MCP discovery.
    pub base_prompt: String,
    pub mode: Mode,
    pub assist_mode: AssistMode,
    pub workspace: std::path::PathBuf,
    pub backend: LlmBackend,
    pub ge_agent: Option<crate::consensus::subagent::GeSubagent>,
    pub todo_items: Vec<crate::types::TodoItem>,
    /// True when launched with -p: auto-quit after task finishes, print final_summary to stdout.
    pub headless: bool,

    /// 覆盖 UserTask 事件的 TUI 显示文本（命令展开时显示占位符而非完整内容）。
    pub task_display_override: Option<String>,

    // ── @ file picker ──────────────────────────────────────────────────────────
    pub at_file: AtFilePickerState,
    /// 全量文件路径索引（后台扫描一次，之后内存过滤）
    pub at_file_index: Vec<std::path::PathBuf>,
    /// 后台扫描线程的结果接收端
    pub at_file_index_rx: Option<std::sync::mpsc::Receiver<Vec<std::path::PathBuf>>>,

    // ── / command picker ───────────────────────────────────────────────────────
    /// 用户通过 COMMAND.md 自定义的命令列表（启动时加载一次）。
    pub user_commands: Vec<UserCommand>,
    pub cmd_picker: CmdPickerState,

    // ── /model picker ──────────────────────────────────────────────────────────
    pub model_picker: ModelPickerState,

    // ── /Session picker ────────────────────────────────────────────────────────
    /// Session IDs shown in the /Session picker (None = picker not active).
    pub pending_session_list: Option<Vec<String>>,

    pub total_usage: crate::agent::provider::Usage,
    pub prompt_token_scale: f32,
    pub recent_completion_tokens_ema: u32,
    /// HTTP client shared with SubAgent DAG executor
    pub http_client: Option<reqwest::Client>,
    /// 用户通过 /compact 命令请求手动压缩
    pub pending_manual_compact: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct PasteChunk {
    pub placeholder: String,
    pub content: String,
}

#[derive(Clone, Debug)]
pub(crate) struct AtFileChunk {
    /// The placeholder token inserted into the input, e.g. `@src/main.rs`.
    pub placeholder: String,
    /// Resolved path to the file (relative to workspace).
    pub path: std::path::PathBuf,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) enum ModelPickerStage {
    /// 第一级：选择后端（GLM / Kimi / Mimo / MiniMax）
    #[default]
    Backend,
    /// 第二级：选择具体模型（已知选定的后端 label）
    Model,
}

#[derive(Debug, Default)]
pub(crate) struct ModelPickerState {
    pub stage: ModelPickerStage,
    /// 当前页显示的标签列表（第一级=后端名，第二级=模型名）
    pub labels: Vec<String>,
    /// 与 labels 一一对应的原始值（用于逻辑判断）
    pub values: Vec<String>,
    /// 当前高亮索引
    pub sel: usize,
    /// 第一级选定的后端 label（进入第二级后使用）
    pub pending_backend: Option<String>,
}

#[derive(Debug, Default)]
pub(crate) struct CmdPickerState {
    /// 当 `Some` 时，/ 命令选择器激活；值为 `/` 之后已输入的过滤字符串。
    pub query: Option<String>,
    /// 当前过滤后匹配的命令名列表（用于渲染面板）。
    pub candidates: Vec<String>,
    /// 当前选中的索引。
    pub sel: usize,
    /// 用户选中模板命令后暂存的 (占位符, 模板内容)，提交时把占位符替换成内容。
    pub pending_template: Option<(String, String)>,
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
        let backend = LlmBackend::from_env();
        let (mut mcp_registry, mcp_warnings) = crate::tools::mcp::McpRegistry::from_env();
        mcp_registry.inject_builtin_for_backend(backend.backend_label());
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

        // Initialise process-level workspace state before any stores are used.
        init_workspace(workspace.clone());
        Session::current().cleanup_old_sessions();

        // messages[0] = system prompt + workspace context（含 AGENTS.md、assist mode 等）。
        // 记忆在每次 start_task 时按任务过滤后拼入 user 消息。
        let assist_mode = AssistMode::Off;
        let context = build_workspace_context(&workspace, assist_mode);
        let messages = vec![Message::system(format!("{base_prompt}\n\n{context}"))];
        Self {
            messages,

            task: String::new(),
            steps_taken: 0,
            llm_calling: false,
            llm_call_started_at: None,
            task_started_at: None,
            last_task_elapsed: None,
            llm_stream_preview: String::new(),
            llm_preview_shown: String::new(),
            needs_agent_executor: false,
            shell_task_running: false,
            shell_exec_rx: None,
            dag_task_running: false,
            dag_result_rx: None,
            dag_progress_rx: None,
            dag_cancel_flag: Arc::new(AtomicBool::new(false)),
            dag_tree_event_idx: None,
            dag_node_done: std::collections::HashMap::new(),
            dag_graph_nodes: Vec::new(),
            dag_output_nodes: Vec::new(),
            interrupt_llm_loop_requested: false,
            interjection_mode: false,
            running: false,
            quit: false,
            pending_confirm: None,

            pending_confirm_note: false,
            current_phase_summary: None,
            task_events: Vec::new(),
            final_summary: None,
            task_collapsed: false,
            show_thinking: true,
            paste_counter: 0,
            paste_chunks: Vec::new(),
            pending_question: None,
            answering_question: false,
            pending_api_key_name: None,
            message_queue: InputQueue::default(),
            mcp_registry,
            mcp_discovery_rx: None,
            skills,
            base_prompt,
            mode: Mode::Normal,
            assist_mode,
            workspace,
            ge_agent: None,
            todo_items: Vec::new(),
            backend,
            headless: false,
            task_display_override: None,
            at_file: AtFilePickerState::default(),
            at_file_index: Vec::new(),
            at_file_index_rx: None,
            user_commands: Vec::new(),
            cmd_picker: CmdPickerState::default(),
            model_picker: ModelPickerState::default(),
            pending_session_list: None,
            total_usage: Default::default(),
            prompt_token_scale: 1.0,
            recent_completion_tokens_ema: 0,
            http_client: None,
            pending_manual_compact: false,
        }
    }
    /// Rebuild messages[0] (system prompt) with the latest base_prompt + MCP tools + workspace context.
    pub(crate) fn rebuild_system_message(&mut self) {
        let system = self.mcp_registry.augment_system_prompt(&self.base_prompt);
        let context = build_workspace_context(&self.workspace, self.assist_mode);
        if let Some(msg) = self.messages.first_mut() {
            msg.content = format!("{system}\n\n{context}");
        }
    }

    pub(crate) fn sync_message_queue_labels(&self, screen: &mut Screen) {
        screen.message_queue_labels = self.message_queue.labels();
    }

    pub(crate) fn enqueue_message(&mut self, screen: &mut Screen, text: String) -> usize {
        let len = self.message_queue.push(text);
        self.sync_message_queue_labels(screen);
        len
    }

    pub(crate) fn dequeue_message(&mut self, screen: &mut Screen) -> Option<String> {
        let item = self.message_queue.pop().map(|queued| queued.text);
        self.sync_message_queue_labels(screen);
        item
    }

    pub(crate) fn clear_message_queue(&mut self, screen: &mut Screen) {
        self.message_queue.clear();
        self.sync_message_queue_labels(screen);
    }
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

    let (cli_prompt, cli_yes) = cli::parse_cli_args();
    let headless = cli_prompt.is_some();

    // Create ~/.goldbot/.env from template if it doesn't exist yet.
    cli::ensure_dot_env();
    let http_client = build_http_client()?;
    let mut app = App::new();
    app.http_client = Some(http_client.clone());
    let _ = dotenvy::from_path(crate::tools::mcp::goldbot_home_dir().join(".env"));

    if !headless {
        enable_raw_mode()?;
        execute!(io::stdout(), EnableBracketedPaste)?;
    }
    let mut screen = if headless {
        Screen::new_headless()?
    } else {
        Screen::new()?
    };
    screen.workspace = app.workspace.to_string_lossy().replace('\\', "/");
    screen.assist_mode = app.assist_mode;
    sync_context_budget(&app, &mut screen);

    // Display discovered skills below the banner.
    let skill_names: Vec<String> = app.skills.iter().map(|s| s.name.clone()).collect();
    if let Some(line) = format_skills_status_line(&skill_names) {
        screen.emit(&[line]);
    }

    // Discover user-defined slash commands.
    app.user_commands = discover_commands();

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

    if !headless {
        let _ = execute!(io::stdout(), DisableBracketedPaste);
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), crossterm::cursor::Show, Print("\r\n"));
    }
    // headless 模式：直接把最终答案打印到 stdout
    if app.headless {
        if let Some(summary) = &app.final_summary {
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
        app.rebuild_system_message();
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
            // Rebuild system prompt now that tools are known.
            app.rebuild_system_message();
            // Display result below the banner.
            let status = app.mcp_registry.startup_status();
            if let Some(line) = format_mcp_status_line(&status.ok, &status.failed) {
                screen.emit(&[line]);
            }
            app.mcp_discovery_rx = None;
        }

        // 轮询 @ 文件索引后台扫描结果
        if let Some(rx) = &app.at_file_index_rx {
            if let Ok(index) = rx.try_recv() {
                app.at_file_index = index;
                app.at_file_index_rx = None;
                // @ picker 仍然活跃时，用新索引刷新候选
                if app.at_file.query.is_some() {
                    ui::input::apply_at_file_filter(app, screen);
                }
            }
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
                    app.llm_call_started_at = None;
                    app.llm_stream_preview.clear();
                    app.llm_preview_shown.clear();
                    screen.status.clear();
                    process_llm_result(app, screen, result);
                }
            }
        }

        poll_shell_exec_result(app, screen);
        poll_dag_result(app, screen);

        drain_ge_events(app, screen);

        if should_run_pending_manual_compact(app) {
            perform_manual_compact(app, screen).await;
            app.pending_manual_compact = false;
        }

        // Consume queued user messages as interjections before the next LLM call
        if app.running
            && app.pending_confirm.is_none()
            && !app.llm_calling
            && !app.shell_task_running
            && !app.message_queue.is_empty()
            && !app.needs_agent_executor
        {
            if let Some(msg) = app.dequeue_message(screen) {
                let wrapped = agent::react::build_interjection_user_message(&msg);
                app.messages.push(agent::provider::Message::user(wrapped));
                let ev = Event::UserTask { text: msg };
                ui::format::emit_live_event(screen, &ev);
                app.task_events.push(ev);
            }
            agent::executor::sync_context_budget(app, screen);
            app.needs_agent_executor = true;
            screen.status = "Interjection sent. Continuing...".to_string();
            screen.refresh();
        }

        if let Some(handle) =
            maybe_spawn_llm_worker(app, screen, &tx, &http_client).await
        {
            llm_task_handle = Some(handle);
        }

        // 同步运行状态，每 400ms 推进一次 spinner 帧，避免频繁刷屏闪烁
        screen.is_running = app.running;
        if app.running && last_spinner_refresh.elapsed() >= Duration::from_millis(400) {
            screen.spinner_tick = screen.spinner_tick.wrapping_add(1);
            if app.llm_calling {
                refresh_llm_status(app, screen);
            } else {
                screen.refresh_status_only();
            }
            last_spinner_refresh = std::time::Instant::now();
        }

        handle_terminal_events(app, screen).await?;

        if app.quit {
            shutdown_background_work(app, screen, &mut llm_task_handle).await;
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    Ok(())
}

// ── Utilities ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use crate::agent::executor::{
        parse_retryable_http_status, retry_delay_for_attempt,
        should_retry_llm_error, should_run_pending_manual_compact,
    };
    use crate::App;
    use std::time::Duration;

    #[test]
    fn retries_server_errors_before_streaming() {
        assert!(should_retry_llm_error(
            "API error 500 Internal Server Error: boom",
            false,
        ));
        assert!(should_retry_llm_error(
            "API error 503 Service Unavailable: boom",
            false,
        ));
    }

    #[test]
    fn does_not_retry_client_errors() {
        assert!(!should_retry_llm_error(
            "API error 400 Bad Request: boom",
            false,
        ));
        assert!(!should_retry_llm_error(
            "API error 429 Too Many Requests: boom",
            false,
        ));
    }

    #[test]
    fn does_not_retry_after_streaming_started() {
        assert!(!should_retry_llm_error("failed reading stream chunk", true,));
    }

    #[test]
    fn parses_http_status_from_provider_errors() {
        assert_eq!(
            parse_retryable_http_status("API error 502 Bad Gateway: upstream down"),
            Some(502)
        );
        assert_eq!(
            parse_retryable_http_status("HTTP request failed: timeout"),
            None
        );
    }

    #[test]
    fn backs_off_between_retries() {
        assert_eq!(retry_delay_for_attempt(1), Duration::from_millis(500));
        assert_eq!(retry_delay_for_attempt(2), Duration::from_secs(1));
        assert_eq!(retry_delay_for_attempt(3), Duration::from_secs(2));
    }

    #[test]
    fn pending_manual_compact_waits_until_agent_is_idle() {
        let mut app = App::new();
        app.pending_manual_compact = true;
        assert!(should_run_pending_manual_compact(&app));

        app.llm_calling = true;
        assert!(!should_run_pending_manual_compact(&app));
        app.llm_calling = false;

        app.shell_task_running = true;
        assert!(!should_run_pending_manual_compact(&app));
        app.shell_task_running = false;

        app.dag_task_running = true;
        assert!(!should_run_pending_manual_compact(&app));
        app.dag_task_running = false;

        app.pending_confirm = Some("rm -rf target".to_string());
        assert!(!should_run_pending_manual_compact(&app));
    }
}
