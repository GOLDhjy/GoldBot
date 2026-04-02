use crossterm::style::Stylize;

use crate::agent::executor::{finish, sync_context_budget};
use crate::agent::provider::Message;
use crate::agent::react::build_interjection_user_message;
use crate::types::{Event, Mode};
use crate::ui::format::emit_live_event;
use crate::ui::ge::{drain_ge_events, parse_ge_command};
use crate::ui::screen::Screen;
use crate::{App, PasteChunk};

use super::modes::begin_confirm_note_mode;

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

pub(super) fn submit_user_input(app: &mut App, screen: &mut Screen, task: String) {
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

pub(super) fn should_interrupt_llm_chat_loop(app: &App) -> bool {
    app.mode == Mode::Normal
        && app.running
        && (app.llm_calling || app.needs_agent_executor || app.shell_task_running)
        && app.pending_confirm.is_none()
        && app.pending_question.is_none()
        && !app.pending_confirm_note
}

pub(super) fn interrupt_llm_chat_loop(app: &mut App, screen: &mut Screen) {
    let canceling_shell = app.shell_task_running;
    if canceling_shell {
        crate::tools::shell::request_cancel_running_shell_commands();
    }
    app.interrupt_llm_loop_requested = true;
    app.interjection_mode = true;
    app.running = false;
    app.needs_agent_executor = false;
    app.llm_calling = false;
    app.llm_stream_preview.clear();
    app.llm_preview_shown.clear();
    screen.input_focused = true;
    screen.status = if canceling_shell {
        "Interrupt requested. Cancelling shell command..."
    } else {
        "LLM loop interrupted. Type a message and press Enter to interject."
    }
    .dark_yellow()
    .to_string();
    if canceling_shell {
        screen.emit(&[String::from(
            "  Interrupt requested. Cancelling shell command...",
        )]);
    } else {
        screen.emit(&[String::from(
            "  LLM loop interrupted. Type a message and press Enter to interject.",
        )]);
    }
    screen.refresh();
}

fn submit_interjection_input(app: &mut App, screen: &mut Screen, task: &str) {
    app.interjection_mode = false;
    app.interrupt_llm_loop_requested = false;
    app.running = true;
    app.needs_agent_executor = true;
    app.llm_stream_preview.clear();
    app.llm_preview_shown.clear();
    app.final_summary = None;
    let wrapped = build_interjection_user_message(task);
    app.messages.push(Message::user(wrapped));
    sync_context_budget(app, screen);
    let ev = Event::UserTask {
        text: task.to_string(),
    };
    emit_live_event(screen, &ev);
    app.task_events.push(ev);
    screen.status = "Interjection sent. Continuing...".grey().to_string();
    screen.refresh();
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

    if app.interjection_mode {
        submit_interjection_input(app, screen, task);
        return Ok(());
    }

    crate::agent::executor::start_task(app, screen, task.to_string());
    Ok(())
}

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
        if !screen.input.is_empty()
            && screen.input_cursor > 0
            && !screen.input[..screen.input_cursor].ends_with(char::is_whitespace)
        {
            screen.insert_char_at_cursor(' ');
        }
        screen.insert_at_cursor(&placeholder);
        app.paste_chunks.push(PasteChunk {
            placeholder,
            content: pasted.to_string(),
        });
        return;
    }

    for ch in pasted.chars() {
        if ch != '\r' && ch != '\n' {
            screen.insert_char_at_cursor(ch);
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

pub(super) fn expand_input_text(app: &App, input: &str) -> String {
    let mut expanded = input.to_string();
    for chunk in &app.paste_chunks {
        expanded = expanded.replace(&chunk.placeholder, &chunk.content);
    }
    expanded
}

pub(super) fn clear_input_buffer(app: &mut App, screen: &mut Screen) {
    screen.input.clear();
    screen.input_cursor = 0;
    app.paste_chunks.clear();
    app.at_file.chunks.clear();
}

pub(super) fn pop_input_at_cursor(app: &mut App, screen: &mut Screen) {
    if screen.input.is_empty() || screen.input_cursor == 0 {
        return;
    }

    let before = &screen.input[..screen.input_cursor];

    for (idx, chunk) in app.paste_chunks.iter().enumerate().rev() {
        if before.ends_with(&chunk.placeholder) {
            let ph_len = chunk.placeholder.len();
            let start = screen.input_cursor - ph_len;
            screen.input.drain(start..screen.input_cursor);
            screen.input_cursor = start;
            app.paste_chunks.remove(idx);
            if screen.input_cursor > 0
                && screen.input.as_bytes().get(screen.input_cursor - 1) == Some(&b' ')
            {
                screen.input_cursor -= 1;
                screen.input.remove(screen.input_cursor);
            }
            return;
        }
    }

    for (idx, chunk) in app.at_file.chunks.iter().enumerate().rev() {
        if before.ends_with(&chunk.placeholder) {
            let ph_len = chunk.placeholder.len();
            let start = screen.input_cursor - ph_len;
            screen.input.drain(start..screen.input_cursor);
            screen.input_cursor = start;
            app.at_file.chunks.remove(idx);
            if screen.input_cursor > 0
                && screen.input.as_bytes().get(screen.input_cursor - 1) == Some(&b' ')
            {
                screen.input_cursor -= 1;
                screen.input.remove(screen.input_cursor);
            }
            return;
        }
    }

    screen.delete_char_before_cursor();
}
