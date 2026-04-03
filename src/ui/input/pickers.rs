use crossterm::style::Stylize;

use crate::agent::executor::sync_context_budget;
use crate::agent::provider::BACKEND_PRESETS;
use crate::memory::Session;
use crate::tools::command::{BuiltinCommand, CommandAction, all_commands, filter_commands};
use crate::ui::screen::Screen;
use crate::{App, AtFileChunk};

use super::submit::clear_input_buffer;

pub(super) fn submit_api_key_input(app: &mut App, screen: &mut Screen, raw: String) {
    let Some(key_name) = app.pending_api_key_name.clone() else {
        return;
    };

    let parsed_value = parse_api_key_input(&raw, &key_name);
    let Some(key_value) = normalize_api_key_value(&key_name, &parsed_value) else {
        screen.status = format!("Please input a valid {} value.", key_name)
            .dark_yellow()
            .to_string();
        screen.refresh();
        return;
    };

    persist_api_key_to_env(&key_name, &key_value);
    clear_input_buffer(app, screen);

    app.pending_api_key_name = None;
    app.running = true;
    app.needs_agent_executor = true;
    screen.status = "API key saved. Retrying...".grey().to_string();
    screen.emit(&[format!("  {} updated. Retrying current task...", key_name)]);
    screen.refresh();
}

fn parse_api_key_input(raw: &str, key_name: &str) -> String {
    let trimmed = raw.trim();
    if let Some((lhs, rhs)) = trimmed.split_once('=')
        && lhs.trim().eq_ignore_ascii_case(key_name)
    {
        return rhs.trim().trim_matches('"').trim_matches('\'').to_string();
    }
    trimmed.trim_matches('"').trim_matches('\'').to_string()
}

pub(super) fn resolve_valid_api_key(key_name: &str) -> Option<String> {
    if let Ok(value) = std::env::var(key_name)
        && let Some(valid) = normalize_api_key_value(key_name, &value)
    {
        return Some(valid);
    }

    read_key_from_dot_env(key_name).and_then(|v| normalize_api_key_value(key_name, &v))
}

fn read_key_from_dot_env(key_name: &str) -> Option<String> {
    let env_path = crate::tools::mcp::goldbot_home_dir().join(".env");
    let raw = std::fs::read_to_string(env_path).ok()?;
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let Some((lhs, rhs)) = trimmed.split_once('=') else {
            continue;
        };
        if lhs.trim() == key_name {
            return Some(rhs.trim().to_string());
        }
    }
    None
}

fn normalize_api_key_value(key_name: &str, raw: &str) -> Option<String> {
    let trimmed = raw.trim().trim_matches('"').trim_matches('\'').trim();
    if trimmed.is_empty() || is_placeholder_api_key(key_name, trimmed) {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn is_placeholder_api_key(key_name: &str, value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    let known_placeholder = match key_name {
        "BIGMODEL_API_KEY" => "your_bigmodel_api_key_here",
        "KIMI_API_KEY" => "your_kimi_api_key_here",
        "MIMO_API_KEY" => "your_mimo_api_key_here",
        "MINIMAX_API_KEY" => "your_minimax_api_key_here",
        _ => "",
    };
    if !known_placeholder.is_empty() && lower == known_placeholder {
        return true;
    }
    matches!(
        lower.as_str(),
        "changeme" | "replace_me" | "your_api_key_here"
    ) || (lower.starts_with("your_") && lower.ends_with("_here") && lower.contains("api_key"))
}

pub(super) fn enter_at_file_mode(app: &mut App, screen: &mut Screen) {
    app.at_file.at_pos = screen.input_cursor;
    app.at_file.query = Some(String::new());
    app.at_file.sel = 0;
    update_at_file_candidates(app, screen, "");
}

pub(super) fn cancel_at_file_mode(app: &mut App, screen: &mut Screen) {
    app.at_file.query = None;
    app.at_file.candidates.clear();
    app.at_file.sel = 0;
    screen.at_file_labels.clear();
    screen.at_file_sel = 0;
}

pub(super) fn update_at_file_candidates(app: &mut App, screen: &mut Screen, _query: &str) {
    if app.at_file_index.is_empty() && app.at_file_index_rx.is_none() {
        let workspace = app.workspace.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut results = Vec::new();
            collect_all_files(&workspace, &workspace, &mut results, 0);
            let _ = tx.send(results);
        });
        app.at_file_index_rx = Some(rx);
    }
    apply_at_file_filter(app, screen);
}

pub(crate) fn apply_at_file_filter(app: &mut App, screen: &mut Screen) {
    let query_lower = app.at_file.query.as_deref().unwrap_or("").to_lowercase();
    let mut matched: Vec<_> = app
        .at_file_index
        .iter()
        .filter(|p| {
            query_lower.is_empty() || p.to_string_lossy().to_lowercase().contains(&query_lower)
        })
        .cloned()
        .collect();
    matched.sort_by(|a, b| {
        a.components()
            .count()
            .cmp(&b.components().count())
            .then_with(|| a.cmp(b))
    });
    matched.truncate(8);
    app.at_file.candidates = matched;
    app.at_file.sel = 0;
    screen.at_file_sel = 0;
    screen.at_file_labels = app
        .at_file
        .candidates
        .iter()
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .collect();
    screen.refresh();
}

pub(super) fn select_at_file(app: &mut App, screen: &mut Screen) {
    let sel = app.at_file.sel;
    let Some(rel_path) = app.at_file.candidates.get(sel).cloned() else {
        cancel_at_file_mode(app, screen);
        return;
    };

    let rel_str = rel_path.to_string_lossy().replace('\\', "/");
    let placeholder = format!("@{}", rel_str);
    let at_pos = app.at_file.at_pos;
    let replace_start = at_pos.saturating_sub(1);
    screen.input.truncate(replace_start);
    screen.input.push_str(&placeholder);
    screen.input_cursor = screen.input.len();

    let abs_path = app.workspace.join(&rel_path);
    app.at_file.chunks.push(AtFileChunk {
        placeholder,
        path: abs_path,
    });
    cancel_at_file_mode(app, screen);
}

pub(super) fn attach_files_to_task(chunks: &[AtFileChunk], task: &str) -> String {
    if chunks.is_empty() {
        return task.to_string();
    }
    let mut result = task.to_string();
    let mut refs = Vec::with_capacity(chunks.len());
    for chunk in chunks {
        let at_ref = chunk
            .placeholder
            .strip_prefix('[')
            .and_then(|s| s.strip_suffix(']'))
            .unwrap_or(&chunk.placeholder)
            .to_string();
        result = result.replace(&chunk.placeholder, &at_ref);
        refs.push(at_ref);
    }
    result.push_str("\n\nAttached file paths:");
    for (chunk, at_ref) in chunks.iter().zip(refs.iter()) {
        let abs_path = chunk.path.to_string_lossy().replace('\\', "/");
        result.push_str(&format!("\n- {at_ref} ({abs_path})"));
    }
    result
}

fn collect_all_files(
    base: &std::path::Path,
    dir: &std::path::Path,
    results: &mut Vec<std::path::PathBuf>,
    depth: usize,
) {
    if depth > 6 || results.len() >= 20_000 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if name.starts_with('.')
            || matches!(
                name,
                "target"
                    | "node_modules"
                    | "dist"
                    | "build"
                    | "out"
                    | "obj"
                    | "vendor"
                    | "__pycache__"
                    | "Binaries"
                    | "Saved"
                    | "Intermediate"
                    | "DerivedDataCache"
            )
        {
            continue;
        }
        if path.is_dir() {
            collect_all_files(base, &path, results, depth + 1);
        } else if path.is_file() {
            if let Ok(rel) = path.strip_prefix(base) {
                results.push(rel.to_path_buf());
            }
        }
    }
}

pub(super) fn enter_command_mode(app: &mut App, screen: &mut Screen) {
    app.cmd_picker.query = Some(String::new());
    app.cmd_picker.sel = 0;
    update_command_candidates(app, screen, "");
}

pub(super) fn cancel_command_mode(app: &mut App, screen: &mut Screen) {
    app.cmd_picker.query = None;
    app.cmd_picker.candidates.clear();
    app.cmd_picker.sel = 0;
    screen.command_labels.clear();
    screen.command_sel = 0;
}

pub(super) fn update_command_candidates(app: &mut App, screen: &mut Screen, query: &str) {
    let all = all_commands(&app.user_commands);
    let filtered = filter_commands(&all, query);
    app.cmd_picker.candidates = filtered.iter().map(|c| c.name.clone()).collect();
    screen.command_labels = filtered
        .iter()
        .map(|c| format!("/{:<16}  {}", c.name, c.description))
        .collect();
    app.cmd_picker.sel = 0;
    screen.command_sel = 0;
}

pub(super) fn select_command(app: &mut App, screen: &mut Screen) {
    let sel = app.cmd_picker.sel;
    let Some(name) = app.cmd_picker.candidates.get(sel).cloned() else {
        cancel_command_mode(app, screen);
        return;
    };

    let all = all_commands(&app.user_commands);
    let Some(cmd) = all.into_iter().find(|c| c.name == name) else {
        cancel_command_mode(app, screen);
        return;
    };

    cancel_command_mode(app, screen);
    clear_input_buffer(app, screen);

    match cmd.action {
        CommandAction::Builtin(builtin) => {
            dispatch_builtin_command(app, screen, builtin);
        }
        CommandAction::Template(content) => {
            let placeholder = format!("/{}", cmd.name);
            screen.input = placeholder.clone();
            app.cmd_picker.pending_template = Some((placeholder, content));
            screen.refresh();
        }
    }
}

pub(super) fn dispatch_builtin_command(app: &mut App, screen: &mut Screen, cmd: BuiltinCommand) {
    match cmd {
        BuiltinCommand::Help => {
            screen.emit(&[
                "  键位绑定：".to_string(),
                "    Ctrl+C         退出".to_string(),
                "    Ctrl+D         展开/折叠任务详情".to_string(),
                "    Tab            切换原生 Thinking ON/OFF".to_string(),
                "    Shift+Tab      循环切换协助模式 (agent / accept edits / plan)".to_string(),
                "    ↑ / ↓          导航菜单选项".to_string(),
                "    Enter          确认选择 / 提交输入".to_string(),
                "    Esc            中断 LLM / 取消输入焦点".to_string(),
                "    @              搜索并附加文件".to_string(),
                "    /              打开命令选择器".to_string(),
                "    GE <目标>       进入 Golden Experience 督导模式".to_string(),
                String::new(),
                "  内置命令：/help  /clear  /compact  /memory  /thinking  /skills  /mcp  /status"
                    .to_string(),
            ]);
        }
        BuiltinCommand::Clear => {
            let clear_session_error = Session::current().clear_current_session().err();
            app.messages.truncate(1);
            app.task_events.clear();
            app.task.clear();
            app.final_summary = None;
            app.running = false;
            app.needs_agent_executor = false;
            app.current_phase_summary = None;
            app.task_started_at = None;
            app.last_task_elapsed = None;
            app.pending_confirm = None;
            app.pending_confirm_note = false;
            app.pending_question = None;
            app.answering_question = false;
            app.pending_manual_compact = false;
            app.pending_session_list = None;
            app.clear_message_queue(screen);
            app.llm_stream_preview.clear();
            app.llm_preview_shown.clear();
            sync_context_budget(app, screen);
            screen.status.clear();
            screen.clear_screen();
            if let Some(err) = clear_session_error {
                screen.emit(&[format!("  /clear: 清理当前 session 失败：{err}")]);
            }
        }
        BuiltinCommand::Compact => {
            if app.pending_manual_compact {
                screen.emit(&["  /compact: 已在排队，等待当前步骤结束后执行。".to_string()]);
            } else {
                app.pending_manual_compact = true;
                let busy = app.llm_calling || app.shell_task_running || app.dag_task_running;
                if busy {
                    screen.emit(&[
                        "  /compact: 已加入队列，将在当前步骤结束后走 LLM 压缩摘要。".to_string(),
                    ]);
                } else {
                    screen.emit(&["  /compact: 开始走 LLM 压缩摘要流程。".to_string()]);
                }
            }
        }
        BuiltinCommand::Memory => {
            let store = crate::memory::project::ProjectStore::current();
            match store.build_memory_message(None) {
                Some(mem) => {
                    let lines: Vec<String> = mem.lines().map(|l| format!("  {l}")).collect();
                    screen.emit(&lines);
                }
                None => {
                    screen.emit(&["  （暂无项目记忆内容）".to_string()]);
                }
            }
        }
        BuiltinCommand::Session => {
            let store = Session::current();
            let sessions = store.list_sessions();
            if sessions.is_empty() {
                screen.emit(&["  （暂无历史会话）".to_string()]);
            } else {
                let labels: Vec<String> = sessions
                    .iter()
                    .map(|id| {
                        let ts = Session::format_session_timestamp(id);
                        let active_session_id = Session::active_id();
                        let marker = if id == &active_session_id {
                            "  ← 当前"
                        } else {
                            ""
                        };
                        format!("{ts}{marker}")
                    })
                    .collect();
                screen.emit(&{
                    let mut v = vec!["  历史会话（↑↓ 选择，Enter 恢复）:".to_string()];
                    for (i, l) in labels.iter().enumerate() {
                        v.push(format!("  {}. {l}", i + 1));
                    }
                    v
                });
                screen.question_labels = labels;
                screen.confirm_selected = Some(0);
                screen.input_focused = false;
                app.pending_session_list = Some(sessions);
                screen.refresh();
            }
        }
        BuiltinCommand::Thinking => {
            app.show_thinking = !app.show_thinking;
            let state = if app.show_thinking { "ON" } else { "OFF" };
            let label = format!("  Thinking: {}", state);
            screen.emit(&[label]);
        }
        BuiltinCommand::Skills => {
            if app.skills.is_empty() {
                screen.emit(&["  未发现任何 Skill。".to_string()]);
            } else {
                let names: Vec<String> = app.skills.iter().map(|s| s.name.clone()).collect();
                screen.emit(&[format!("  Skills ({}): {}", names.len(), names.join(", "))]);
            }
        }
        BuiltinCommand::Mcp => {
            let status = app.mcp_registry.startup_status();
            if status.ok.is_empty() && status.failed.is_empty() {
                screen.emit(&["  未配置任何 MCP 服务器。".to_string()]);
            } else {
                let mut lines = vec!["  MCP 服务器：".to_string()];
                for (server, tool_count) in &status.ok {
                    lines.push(format!("    ✓ {}  ({} 个工具)", server, tool_count));
                }
                for server in &status.failed {
                    lines.push(format!("    ✗ {}  (连接失败)", server));
                }
                screen.emit(&lines);
            }
        }
        BuiltinCommand::Status => {
            let ws = app.workspace.to_string_lossy().replace('\\', "/");
            let mode_str = format!("{:?}", app.assist_mode);
            let thinking = if app.show_thinking { "ON" } else { "OFF" };
            screen.emit(&[
                format!("  Workspace:  {}", ws),
                format!("  Backend:    {}", app.backend.backend_label()),
                format!("  Model:      {}", app.backend.model_name()),
                format!("  Mode:       {}", mode_str),
                format!("  Thinking:   {}", thinking),
                format!("  Skills:     {}", app.skills.len()),
                format!("  Commands:   {} 用户 + 9 内置", app.user_commands.len()),
                format!("  Messages:   {}", app.messages.len()),
            ]);
        }
        BuiltinCommand::Model => {
            enter_model_picker_backend_stage(app, screen);
        }
    }
}

fn persist_backend_to_env(backend_label: &str, model: &str) {
    let env_path = crate::tools::mcp::goldbot_home_dir().join(".env");
    let raw = std::fs::read_to_string(&env_path).unwrap_or_default();

    let provider_value = match backend_label {
        "Kimi" => "kimi",
        "Mimo" => "mimo",
        "MiniMax" => "minimax",
        _ => "glm",
    };
    let model_key = match backend_label {
        "Kimi" => "KIMI_MODEL",
        "Mimo" => "MIMO_MODEL",
        "MiniMax" => "MINIMAX_MODEL",
        _ => "BIGMODEL_MODEL",
    };

    let mut lines: Vec<String> = raw.lines().map(|l| l.to_string()).collect();
    let mut found_provider = false;
    let mut found_model = false;

    for line in &mut lines {
        let trimmed = line.trim_start();
        if trimmed.starts_with("LLM_PROVIDER=") || trimmed.starts_with("LLM_PROVIDER =") {
            *line = format!("LLM_PROVIDER={}", provider_value);
            found_provider = true;
        } else if trimmed.starts_with(&format!("{}=", model_key))
            || trimmed.starts_with(&format!("{} =", model_key))
        {
            *line = format!("{}={}", model_key, model);
            found_model = true;
        }
    }
    if !found_provider {
        lines.push(format!("LLM_PROVIDER={}", provider_value));
    }
    if !found_model {
        lines.push(format!("{}={}", model_key, model));
    }

    let content = lines.join("\n") + "\n";
    let _ = std::fs::write(&env_path, content);
}

fn persist_api_key_to_env(key_name: &str, key_value: &str) {
    let env_path = crate::tools::mcp::goldbot_home_dir().join(".env");
    let raw = std::fs::read_to_string(&env_path).unwrap_or_default();

    let mut lines: Vec<String> = raw.lines().map(|l| l.to_string()).collect();
    let mut found = false;
    for line in &mut lines {
        let trimmed = line.trim_start();
        if trimmed.starts_with(&format!("{key_name}="))
            || trimmed.starts_with(&format!("{key_name} ="))
        {
            *line = format!("{key_name}={key_value}");
            found = true;
        }
    }
    if !found {
        lines.push(format!("{key_name}={key_value}"));
    }

    let content = lines.join("\n") + "\n";
    let _ = std::fs::write(&env_path, content);
    unsafe {
        std::env::set_var(key_name, key_value);
    }
}

pub(super) fn enter_model_picker_backend_stage(app: &mut App, screen: &mut Screen) {
    app.model_picker.stage = crate::ModelPickerStage::Backend;
    app.model_picker.pending_backend = None;
    app.model_picker.sel = 0;
    app.model_picker.labels = BACKEND_PRESETS
        .iter()
        .map(|(label, models)| format!("{label}  ({} 个模型)", models.len()))
        .collect();
    app.model_picker.values = BACKEND_PRESETS
        .iter()
        .map(|(label, _)| label.to_string())
        .collect();
    screen.model_picker_labels = app.model_picker.labels.clone();
    screen.model_picker_sel = 0;
    clear_input_buffer(app, screen);
    screen.refresh();
}

pub(super) fn enter_model_picker_model_stage(app: &mut App, screen: &mut Screen, backend: &str) {
    let Some(preset) = BACKEND_PRESETS.iter().find(|(l, _)| *l == backend) else {
        cancel_model_picker(app, screen);
        return;
    };
    let current_model = if app.backend.backend_label() == backend {
        app.backend.model_name().to_string()
    } else {
        String::new()
    };
    app.model_picker.stage = crate::ModelPickerStage::Model;
    app.model_picker.pending_backend = Some(backend.to_string());
    app.model_picker.labels = preset
        .1
        .iter()
        .map(|m| {
            if *m == current_model {
                format!("{m}  ✓")
            } else {
                m.to_string()
            }
        })
        .collect();
    app.model_picker.values = preset.1.iter().map(|m| m.to_string()).collect();
    app.model_picker.sel = preset
        .1
        .iter()
        .position(|m| *m == current_model)
        .unwrap_or(0);
    screen.model_picker_labels = app.model_picker.labels.clone();
    screen.model_picker_sel = app.model_picker.sel;
    screen.refresh();
}

pub(super) fn cancel_model_picker(app: &mut App, screen: &mut Screen) {
    app.model_picker.stage = crate::ModelPickerStage::Backend;
    app.model_picker.labels.clear();
    app.model_picker.values.clear();
    app.model_picker.sel = 0;
    app.model_picker.pending_backend = None;
    screen.model_picker_labels.clear();
    screen.model_picker_sel = 0;
}

pub(super) fn select_model_item(app: &mut App, screen: &mut Screen) {
    let sel = app.model_picker.sel;
    let Some(value) = app.model_picker.values.get(sel).cloned() else {
        cancel_model_picker(app, screen);
        return;
    };
    match app.model_picker.stage {
        crate::ModelPickerStage::Backend => {
            enter_model_picker_model_stage(app, screen, &value);
        }
        crate::ModelPickerStage::Model => {
            let backend = app.model_picker.pending_backend.clone().unwrap_or_default();
            let model = value;
            app.backend = match backend.as_str() {
                "Kimi" => crate::agent::provider::LlmBackend::Kimi(model.clone()),
                "Mimo" => crate::agent::provider::LlmBackend::Mimo(model.clone()),
                "MiniMax" => crate::agent::provider::LlmBackend::MiniMax(model.clone()),
                _ => crate::agent::provider::LlmBackend::Glm(model.clone()),
            };
            app.prompt_token_scale = 1.0;
            app.recent_completion_tokens_ema = 0;
            persist_backend_to_env(app.backend.backend_label(), app.backend.model_name());
            sync_context_budget(app, screen);
            cancel_model_picker(app, screen);
            clear_input_buffer(app, screen);
            let mut lines = vec![format!(
                "  已切换至 {} / {}",
                app.backend.backend_label(),
                app.backend.model_name()
            )];
            app.pending_api_key_name = None;
            screen.status.clear();
            let key_name = app.backend.required_key_name().to_string();
            if let Some(key_value) = resolve_valid_api_key(&key_name) {
                unsafe {
                    std::env::set_var(&key_name, &key_value);
                }
            } else {
                let env_path = crate::tools::mcp::goldbot_home_dir().join(".env");
                app.pending_api_key_name = Some(key_name.clone());
                app.running = false;
                app.needs_agent_executor = false;
                screen.input_focused = true;
                lines.push(format!(
                    "  {} {} 未配置，请编辑: {}",
                    crossterm::style::Stylize::yellow(
                        crate::ui::symbols::Symbols::current().warning
                    ),
                    key_name,
                    env_path.display()
                ));
                lines.push(format!(
                    "  Paste {key_name} now and press Enter to continue this session."
                ));
                screen.status = format!("Waiting for {} input...", key_name)
                    .dark_yellow()
                    .to_string();
            }
            screen.emit(&lines);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::dispatch_builtin_command;
    use crate::App;
    use crate::agent::provider::Message;
    use crate::tools::command::BuiltinCommand;
    use crate::ui::screen::Screen;

    #[test]
    fn compact_command_queues_manual_compaction() {
        let mut app = App::new();
        let mut screen = Screen::new_headless().expect("headless screen");

        dispatch_builtin_command(&mut app, &mut screen, BuiltinCommand::Compact);

        assert!(app.pending_manual_compact);
    }

    #[test]
    fn compact_command_does_not_truncate_messages_immediately() {
        let mut app = App::new();
        let mut screen = Screen::new_headless().expect("headless screen");
        app.messages.push(Message::user("task"));
        app.messages.push(Message::assistant("step 1"));
        app.messages.push(Message::user("more context"));
        app.messages.push(Message::assistant("step 2"));
        let before: Vec<_> = app
            .messages
            .iter()
            .map(|msg| (msg.role.clone(), msg.content.clone()))
            .collect();

        dispatch_builtin_command(&mut app, &mut screen, BuiltinCommand::Compact);

        let after: Vec<_> = app
            .messages
            .iter()
            .map(|msg| (msg.role.clone(), msg.content.clone()))
            .collect();
        assert_eq!(after, before);
        assert!(app.pending_manual_compact);
    }
}
