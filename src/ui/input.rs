use crossterm::{event::KeyCode, event::KeyModifiers, style::Stylize};

use crate::agent::executor::{execute_command, finish};
use crate::agent::provider::Message;
use crate::types::{Event, Mode};
use crate::ui::format::{emit_live_event, toggle_collapse};
use crate::ui::ge::{drain_ge_events, is_ge_mode, parse_ge_command};
use crate::ui::screen::Screen;
use crate::{App, AtFileChunk, PasteChunk};

// ── Key handling ──────────────────────────────────────────────────────────────

pub(crate) fn handle_key(
    app: &mut App,
    screen: &mut Screen,
    key: KeyCode,
    modifiers: KeyModifiers,
) -> bool {
    if key == KeyCode::Char('c') && modifiers.contains(KeyModifiers::CONTROL) {
        return true;
    }
    if is_ge_mode(app.mode) && screen.input.trim().is_empty() {
        let expand_prompt_hotkey = matches!(key, KeyCode::Char('p') | KeyCode::Char('P'))
            && modifiers.contains(KeyModifiers::CONTROL);
        let expand_result_hotkey = matches!(key, KeyCode::Char('r') | KeyCode::Char('R'))
            && modifiers.contains(KeyModifiers::CONTROL);
        if expand_prompt_hotkey || expand_result_hotkey {
            let cmd = if expand_prompt_hotkey {
                crate::consensus::subagent::GeAgentCommand::ExpandLastPrompt
            } else {
                crate::consensus::subagent::GeAgentCommand::ExpandLastResult
            };
            if let Some(agent) = app.ge_agent.as_ref() {
                if !agent.send(cmd) {
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
            format!("{} {}", "Thinking:".grey(), "ON".green().bold())
        } else {
            format!("{} {}", "Thinking:".grey(), "OFF".yellow().bold())
        };
        if !app.llm_calling {
            screen.status = label;
            screen.refresh();
        }
        return false;
    }
    if key == KeyCode::BackTab
        && modifiers.contains(KeyModifiers::SHIFT)
        && screen.confirm_selected.is_none()
        && !app.pending_confirm_note
    {
        app.assist_mode = app.assist_mode.cycle();
        screen.assist_mode = app.assist_mode;
        app.rebuild_assistant_context_message();
        screen.refresh();
        return false;
    }

    if screen.confirm_selected.is_some() {
        handle_confirm_mode(app, screen, key, modifiers);
    } else if app.pending_confirm_note {
        handle_note_mode(app, screen, key, modifiers);
    } else if !app.running {
        handle_idle_mode(app, screen, key, modifiers);
    } else {
        handle_running_mode(app, screen, key, modifiers);
    }

    false
}

fn handle_confirm_mode(app: &mut App, screen: &mut Screen, key: KeyCode, modifiers: KeyModifiers) {
    let sel = screen.confirm_selected.unwrap();

    if app.pending_question.is_some() {
        // ── Question mode: ↑/↓ navigate options, Enter to confirm ────────
        let opt_count = screen.question_labels.len();
        match key {
            KeyCode::Up => {
                screen.confirm_selected = Some(sel.saturating_sub(1));
                screen.refresh();
            }
            KeyCode::Down => {
                screen.confirm_selected = Some((sel + 1).min(opt_count.saturating_sub(1)));
                screen.refresh();
            }
            KeyCode::Enter => {
                let (_, options) = app.pending_question.take().unwrap();
                let raw_opt = options.get(sel).cloned().unwrap_or_default();
                screen.confirm_selected = None;
                screen.question_labels.clear();
                app.running = false;
                screen.input_focused = true;
                if raw_opt == "<user_input>" {
                    // Switch to free-text input mode.
                    app.answering_question = true;
                    screen.status = "✍ 请输入你的答案后按 Enter".dark_yellow().to_string();
                    screen.refresh();
                } else {
                    // Feed the preset answer directly.
                    submit_question_answer(app, screen, raw_opt);
                }
            }
            KeyCode::Char(c) if modifiers.is_empty() || modifiers == KeyModifiers::SHIFT => {
                // Typing starts free-text input for <user_input> option.
                app.pending_question = None;
                screen.confirm_selected = None;
                screen.question_labels.clear();
                screen.input_focused = true;
                app.answering_question = true;
                screen.input.push(c);
                screen.status = "✍ 请输入你的答案后按 Enter".dark_yellow().to_string();
                screen.refresh();
            }
            _ => {}
        }
    } else {
        // ── Confirmation mode: ↑/↓ navigate, Enter confirm, or type note ─────
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
                            return;
                        };
                        let file_hint = app.pending_confirm_file.take();
                        execute_command(app, screen, &cmd, file_hint.as_deref());
                        app.needs_agent_executor = true;
                    }
                    1 => {
                        // Skip
                        screen.confirm_selected = None;
                        screen.input_focused = true;
                        app.pending_confirm_note = false;
                        let Some(cmd) = app.pending_confirm.take() else {
                            screen.refresh();
                            return;
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
                        app.needs_agent_executor = true;
                    }
                    2 => {
                        // Abort
                        screen.confirm_selected = None;
                        app.pending_confirm_note = false;
                        app.pending_confirm = None;
                        app.pending_confirm_file = None;
                        finish(app, screen, "Task aborted by user".to_string());
                    }
                    _ => begin_confirm_note_mode(app, screen, None),
                }
            }
            KeyCode::Char(c) if modifiers.is_empty() || modifiers == KeyModifiers::SHIFT => {
                begin_confirm_note_mode(app, screen, Some(c));
            }
            _ => {}
        }
    }
}

fn handle_note_mode(app: &mut App, screen: &mut Screen, key: KeyCode, modifiers: KeyModifiers) {
    match key {
        KeyCode::Enter => {
            let note = expand_input_text(app, &screen.input).trim().to_string();
            if note.is_empty() {
                exit_confirm_note_mode(app, screen, true);
                return;
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
            app.pending_confirm_file = None;
            app.pending_confirm_note = false;
            app.needs_agent_executor = true;
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
}

fn handle_idle_mode(app: &mut App, screen: &mut Screen, key: KeyCode, modifiers: KeyModifiers) {
    if screen.input_focused {
        // ── @ file picker intercepts navigation keys first ──
        if app.at_file_query.is_some() {
            match key {
                KeyCode::Up => {
                    app.at_file_sel = app.at_file_sel.saturating_sub(1);
                    screen.at_file_sel = app.at_file_sel;
                    screen.refresh();
                    return;
                }
                KeyCode::Down => {
                    let max = app.at_file_candidates.len().saturating_sub(1);
                    app.at_file_sel = (app.at_file_sel + 1).min(max);
                    screen.at_file_sel = app.at_file_sel;
                    screen.refresh();
                    return;
                }
                KeyCode::Enter | KeyCode::Tab => {
                    if app.at_file_candidates.is_empty() {
                        cancel_at_file_mode(app, screen);
                    } else {
                        select_at_file(app, screen);
                    }
                    screen.refresh();
                    return;
                }
                KeyCode::Esc if modifiers.is_empty() => {
                    cancel_at_file_mode(app, screen);
                    screen.refresh();
                    return;
                }
                KeyCode::Backspace => {
                    let query = app.at_file_query.as_mut().unwrap();
                    if query.is_empty() {
                        // Remove the @ from input and exit picker
                        screen.input.pop();
                        cancel_at_file_mode(app, screen);
                    } else {
                        query.pop();
                        screen.input.pop();
                        let q = query.clone();
                        update_at_file_candidates(app, screen, &q);
                    }
                    screen.refresh();
                    return;
                }
                KeyCode::Char(c) if modifiers.is_empty() || modifiers == KeyModifiers::SHIFT => {
                    let query = app.at_file_query.as_mut().unwrap();
                    query.push(c);
                    screen.input.push(c);
                    let q = query.clone();
                    update_at_file_candidates(app, screen, &q);
                    screen.refresh();
                    return;
                }
                _ => {}
            }
        }

        match key {
            KeyCode::Enter => {
                let task = expand_input_text(app, &screen.input).trim().to_string();
                if !task.is_empty() {
                    // Build final task with attached file contents before clearing state
                    let at_file_chunks = std::mem::take(&mut app.at_file_chunks);
                    cancel_at_file_mode(app, screen);
                    let final_task = attach_files_to_task(&at_file_chunks, &task);
                    clear_input_buffer(app, screen);
                    if app.answering_question {
                        app.answering_question = false;
                        screen.status.clear();
                        submit_question_answer(app, screen, final_task);
                    } else {
                        submit_user_input(app, screen, final_task);
                    }
                }
            }
            KeyCode::Esc if modifiers.is_empty() => {
                if app.at_file_query.is_some() {
                    cancel_at_file_mode(app, screen);
                } else {
                    screen.input_focused = false;
                }
                screen.refresh();
            }
            KeyCode::Char(c) if modifiers.is_empty() || modifiers == KeyModifiers::SHIFT => {
                screen.input.push(c);
                if c == '@' {
                    enter_at_file_mode(app, screen);
                }
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
            KeyCode::Esc if modifiers.is_empty() => {
                app.quit = true;
            }
            KeyCode::Char(c) if modifiers.is_empty() || modifiers == KeyModifiers::SHIFT => {
                screen.input_focused = true;
                screen.input.push(c);
                if c == '@' {
                    enter_at_file_mode(app, screen);
                }
                screen.refresh();
            }
            KeyCode::Backspace => {
                screen.input_focused = true;
                screen.refresh();
            }
            _ => {}
        }
    }
}

fn handle_running_mode(app: &mut App, screen: &mut Screen, key: KeyCode, modifiers: KeyModifiers) {
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

// ── Question / answer submission ──────────────────────────────────────────────

pub(crate) fn submit_question_answer(app: &mut App, screen: &mut Screen, answer: String) {
    app.messages
        .push(Message::user(format!("[回答]: {answer}")));
    let ev = Event::Thinking {
        text: format!("用户回答：{answer}"),
    };
    emit_live_event(screen, &ev);
    app.task_events.push(ev);
    app.running = true;
    app.needs_agent_executor = true;
    screen.status.clear();
    screen.refresh();
}

pub(crate) fn submit_user_input(app: &mut App, screen: &mut Screen, task: String) {
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
        if rest == "展开提示词"
            || rest == "展开prompt"
            || rest.eq_ignore_ascii_case("expand prompt")
            || rest.eq_ignore_ascii_case("expand")
        {
            if let Some(agent) = app.ge_agent.as_ref() {
                if !agent.send(crate::consensus::subagent::GeAgentCommand::ExpandLastPrompt) {
                    app.ge_agent = None;
                    app.mode = Mode::Normal;
                    screen.emit(&["  GE channel disconnected.".to_string()]);
                }
            } else {
                screen.emit(&["  GE is not active. Start with `GE <goal>` first.".to_string()]);
            }
            return Ok(());
        }
        if rest == "展开结果"
            || rest == "展开输出"
            || rest.eq_ignore_ascii_case("expand result")
            || rest.eq_ignore_ascii_case("expand output")
        {
            if let Some(agent) = app.ge_agent.as_ref() {
                if !agent.send(crate::consensus::subagent::GeAgentCommand::ExpandLastResult) {
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
            screen.emit(&[String::from(
                "  GE controls: `q` hard exit | Ctrl+P expand prompt | Ctrl+R expand result.",
            )]);
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

    crate::agent::executor::start_task(app, screen, task.to_string());
    Ok(())
}

// ── Paste handling ────────────────────────────────────────────────────────────

pub(crate) fn handle_paste(app: &mut App, screen: &mut Screen, pasted: &str) {
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

// ── Input buffer helpers ──────────────────────────────────────────────────────

pub(crate) fn expand_input_text(app: &App, input: &str) -> String {
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

// ── Confirm-note mode ─────────────────────────────────────────────────────────

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

// ── @ file picker ─────────────────────────────────────────────────────────────

/// Enter @ file picker mode: record the cursor position, run the initial (empty) search.
fn enter_at_file_mode(app: &mut App, screen: &mut Screen) {
    app.at_file_at_pos = screen.input.len(); // byte offset immediately after '@'
    app.at_file_query = Some(String::new());
    app.at_file_sel = 0;
    update_at_file_candidates(app, screen, "");
}

/// Exit @ file picker mode without selecting a file (clears picker state, leaves input intact).
fn cancel_at_file_mode(app: &mut App, screen: &mut Screen) {
    app.at_file_query = None;
    app.at_file_candidates.clear();
    app.at_file_sel = 0;
    screen.at_file_labels.clear();
    screen.at_file_sel = 0;
}

/// Re-run the file search for `query` and update App + Screen state.
fn update_at_file_candidates(app: &mut App, screen: &mut Screen, query: &str) {
    app.at_file_candidates = search_files(&app.workspace, query);
    app.at_file_sel = 0;
    screen.at_file_sel = 0;
    screen.at_file_labels = app
        .at_file_candidates
        .iter()
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .collect();
}

/// Confirm the currently highlighted candidate: replace `@{query}` in the input with a
/// placeholder token, record the file chunk, then exit picker mode.
fn select_at_file(app: &mut App, screen: &mut Screen) {
    let sel = app.at_file_sel;
    let Some(rel_path) = app.at_file_candidates.get(sel).cloned() else {
        cancel_at_file_mode(app, screen);
        return;
    };

    let rel_str = rel_path.to_string_lossy().replace('\\', "/");
    let placeholder = format!("[@{}]", rel_str);

    // Replace "@{query}" portion of screen.input (from the @ position onwards) with placeholder
    let at_pos = app.at_file_at_pos; // byte position just after '@'
    // at_pos - 1 is where '@' sits; everything from there to the end is "@query"
    let replace_start = at_pos.saturating_sub(1);
    screen.input.truncate(replace_start);
    screen.input.push_str(&placeholder);

    // Store absolute path so read works regardless of CWD changes
    let abs_path = app.workspace.join(&rel_path);
    app.at_file_chunks.push(AtFileChunk {
        placeholder,
        path: abs_path,
    });
    cancel_at_file_mode(app, screen);
}

/// Append the content of all attached files to the task string.
fn attach_files_to_task(chunks: &[AtFileChunk], task: &str) -> String {
    if chunks.is_empty() {
        return task.to_string();
    }
    let mut result = task.to_string();
    for chunk in chunks {
        let rel = chunk.path.to_string_lossy().replace('\\', "/");
        match std::fs::read_to_string(&chunk.path) {
            Ok(content) => {
                result.push_str(&format!(
                    "\n\n--- {} ({}) ---\n{content}\n--- end {} ---",
                    chunk.placeholder, rel, rel
                ));
            }
            Err(e) => {
                result.push_str(&format!("\n\n--- {} 读取失败: {e} ---", chunk.placeholder));
            }
        }
    }
    result
}

/// Search files under `workspace` whose name contains `query` (case-insensitive).
/// Returns at most 8 relative paths, sorted by path depth then alphabetically.
fn search_files(workspace: &std::path::Path, query: &str) -> Vec<std::path::PathBuf> {
    let query_lower = query.to_lowercase();
    let mut results: Vec<std::path::PathBuf> = Vec::new();
    collect_files_recursive(workspace, workspace, &query_lower, &mut results, 0);
    // Sort: shallower paths first, then alphabetically
    results.sort_by(|a, b| {
        let da = a.components().count();
        let db = b.components().count();
        da.cmp(&db).then_with(|| a.cmp(b))
    });
    results.truncate(8);
    results
}

/// Recursively collect files matching `query` under `dir` (relative to `base`).
fn collect_files_recursive(
    base: &std::path::Path,
    dir: &std::path::Path,
    query: &str,
    results: &mut Vec<std::path::PathBuf>,
    depth: usize,
) {
    if depth > 6 || results.len() >= 64 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        // Skip hidden entries and common large/non-source directories
        if name.starts_with('.') || matches!(name, "target" | "node_modules" | "dist" | "build") {
            continue;
        }
        if path.is_dir() {
            collect_files_recursive(base, &path, query, results, depth + 1);
        } else if path.is_file() {
            let name_lower = name.to_lowercase();
            if (query.is_empty() || name_lower.contains(query))
                && let Ok(rel) = path.strip_prefix(base)
            {
                results.push(rel.to_path_buf());
            }
        }
    }
}
