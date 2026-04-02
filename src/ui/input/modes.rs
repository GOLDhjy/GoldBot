use crossterm::{event::KeyCode, event::KeyModifiers, style::Stylize};

use crate::App;
use crate::agent::executor::{
    execute_command, finish, push_tool_result_to_llm, sync_context_budget,
};
use crate::agent::provider::Message;
use crate::memory::Session;
use crate::types::Event;
use crate::ui::format::emit_live_event;
use crate::ui::screen::Screen;

use super::insert_char_with_trigger;
use super::pickers::{
    attach_files_to_task, cancel_at_file_mode, cancel_command_mode, cancel_model_picker,
    enter_model_picker_backend_stage, select_at_file, select_command, select_model_item,
    submit_api_key_input, update_at_file_candidates, update_command_candidates,
};
use super::submit::{
    clear_input_buffer, expand_input_text, pop_input_at_cursor, submit_question_answer,
    submit_user_input,
};

pub(super) fn handle_confirm_mode(
    app: &mut App,
    screen: &mut Screen,
    key: KeyCode,
    modifiers: KeyModifiers,
) {
    let sel = screen.confirm_selected.unwrap();

    if app.pending_session_list.is_some() {
        let count = screen.question_labels.len();
        match key {
            KeyCode::Up => {
                screen.confirm_selected = Some(sel.saturating_sub(1));
                screen.refresh();
            }
            KeyCode::Down => {
                screen.confirm_selected = Some((sel + 1).min(count.saturating_sub(1)));
                screen.refresh();
            }
            KeyCode::Enter => {
                let sessions = app.pending_session_list.take().unwrap();
                screen.confirm_selected = None;
                screen.question_labels.clear();
                screen.input_focused = true;

                if let Some(id) = sessions.get(sel) {
                    let store = Session::current();
                    if let Err(e) = store.restore(app, screen, id) {
                        screen.emit(&[format!("  ✗ 无法读取会话：{e}")]);
                    }
                }
                screen.refresh();
            }
            KeyCode::Esc => {
                app.pending_session_list = None;
                screen.confirm_selected = None;
                screen.question_labels.clear();
                screen.input_focused = true;
                screen.refresh();
            }
            _ => {}
        }
        return;
    }

    if app.pending_question.is_some() {
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
                    app.answering_question = true;
                    screen.status = "✍ 请输入你的答案后按 Enter".dark_yellow().to_string();
                    screen.refresh();
                } else {
                    submit_question_answer(app, screen, raw_opt);
                }
            }
            KeyCode::Char(c) if modifiers.is_empty() || modifiers == KeyModifiers::SHIFT => {
                app.pending_question = None;
                screen.confirm_selected = None;
                screen.question_labels.clear();
                screen.input_focused = true;
                app.answering_question = true;
                screen.insert_char_at_cursor(c);
                screen.status = "✍ 请输入你的答案后按 Enter".dark_yellow().to_string();
                screen.refresh();
            }
            _ => {}
        }
    } else {
        match key {
            KeyCode::Up => {
                screen.confirm_selected = Some(sel.saturating_sub(1));
                screen.refresh();
            }
            KeyCode::Down => {
                screen.confirm_selected = Some((sel + 1).min(3));
                screen.refresh();
            }
            KeyCode::Enter => match sel {
                0 => {
                    screen.confirm_selected = None;
                    screen.input_focused = true;
                    app.pending_confirm_note = false;
                    let Some(cmd) = app.pending_confirm.take() else {
                        screen.refresh();
                        return;
                    };
                    execute_command(app, screen, &cmd);
                }
                1 => {
                    screen.confirm_selected = None;
                    screen.input_focused = true;
                    app.pending_confirm_note = false;
                    let Some(cmd) = app.pending_confirm.take() else {
                        screen.refresh();
                        return;
                    };
                    let msg = format!("User chose to skip this command: {cmd}");
                    push_tool_result_to_llm(app, "Tool result:", &msg);
                    let ev = Event::ToolResult {
                        exit_code: 0,
                        output: msg,
                    };
                    emit_live_event(screen, &ev);
                    app.task_events.push(ev);
                    app.needs_agent_executor = true;
                }
                2 => {
                    screen.confirm_selected = None;
                    app.pending_confirm_note = false;
                    app.pending_confirm = None;
                    finish(app, screen, "Task aborted by user".to_string());
                }
                _ => begin_confirm_note_mode(app, screen, None),
            },
            KeyCode::Char(c) if modifiers.is_empty() || modifiers == KeyModifiers::SHIFT => {
                begin_confirm_note_mode(app, screen, Some(c));
            }
            _ => {}
        }
    }
}

pub(super) fn handle_note_mode(
    app: &mut App,
    screen: &mut Screen,
    key: KeyCode,
    modifiers: KeyModifiers,
) {
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
            sync_context_budget(app, screen);
            let ev = Event::Thinking {
                text: format!("User note: {note}"),
            };
            emit_live_event(screen, &ev);
            app.task_events.push(ev);

            app.pending_confirm = None;
            app.pending_confirm_note = false;
            app.needs_agent_executor = true;
            screen.status.clear();
            clear_input_buffer(app, screen);
            screen.input_focused = true;
            screen.refresh();
        }
        KeyCode::Esc if modifiers.is_empty() => exit_confirm_note_mode(app, screen, true),
        KeyCode::Left => {
            screen.cursor_left();
            screen.refresh();
        }
        KeyCode::Right => {
            screen.cursor_right();
            screen.refresh();
        }
        KeyCode::Home => {
            screen.cursor_home();
            screen.refresh();
        }
        KeyCode::End => {
            screen.cursor_end();
            screen.refresh();
        }
        KeyCode::Backspace => {
            pop_input_at_cursor(app, screen);
            screen.refresh();
        }
        KeyCode::Char(c) if modifiers.is_empty() || modifiers == KeyModifiers::SHIFT => {
            screen.insert_char_at_cursor(c);
            screen.refresh();
        }
        _ => {}
    }
}

fn handle_api_key_input_mode(
    app: &mut App,
    screen: &mut Screen,
    key: KeyCode,
    modifiers: KeyModifiers,
) {
    match key {
        KeyCode::Enter => {
            let raw = expand_input_text(app, &screen.input);
            submit_api_key_input(app, screen, raw);
        }
        KeyCode::Esc if modifiers.is_empty() => {
            app.pending_api_key_name = None;
            clear_input_buffer(app, screen);
            screen.status.clear();
            screen.emit(&["  API key input canceled. Task is paused.".to_string()]);
            screen.refresh();
        }
        KeyCode::Left => {
            screen.cursor_left();
            screen.refresh();
        }
        KeyCode::Right => {
            screen.cursor_right();
            screen.refresh();
        }
        KeyCode::Home => {
            screen.cursor_home();
            screen.refresh();
        }
        KeyCode::End => {
            screen.cursor_end();
            screen.refresh();
        }
        KeyCode::Backspace => {
            pop_input_at_cursor(app, screen);
            screen.refresh();
        }
        KeyCode::Char(c) if modifiers.is_empty() || modifiers == KeyModifiers::SHIFT => {
            screen.insert_char_at_cursor(c);
            screen.refresh();
        }
        _ => {}
    }
}

pub(super) fn handle_idle_mode(
    app: &mut App,
    screen: &mut Screen,
    key: KeyCode,
    modifiers: KeyModifiers,
) {
    if app.pending_api_key_name.is_some() {
        handle_api_key_input_mode(app, screen, key, modifiers);
        return;
    }

    if screen.input_focused {
        if app.at_file.query.is_some() {
            match key {
                KeyCode::Up => {
                    app.at_file.sel = app.at_file.sel.saturating_sub(1);
                    screen.at_file_sel = app.at_file.sel;
                    screen.refresh();
                    return;
                }
                KeyCode::Down => {
                    let max = app.at_file.candidates.len().saturating_sub(1);
                    app.at_file.sel = (app.at_file.sel + 1).min(max);
                    screen.at_file_sel = app.at_file.sel;
                    screen.refresh();
                    return;
                }
                KeyCode::Enter | KeyCode::Tab => {
                    if app.at_file.candidates.is_empty() {
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
                    let query = app.at_file.query.as_mut().unwrap();
                    if query.is_empty() {
                        screen.delete_char_before_cursor();
                        cancel_at_file_mode(app, screen);
                    } else {
                        query.pop();
                        screen.delete_char_before_cursor();
                        let q = query.clone();
                        update_at_file_candidates(app, screen, &q);
                    }
                    screen.refresh();
                    return;
                }
                KeyCode::Char(c) if modifiers.is_empty() || modifiers == KeyModifiers::SHIFT => {
                    let query = app.at_file.query.as_mut().unwrap();
                    query.push(c);
                    screen.insert_char_at_cursor(c);
                    let q = query.clone();
                    update_at_file_candidates(app, screen, &q);
                    screen.refresh();
                    return;
                }
                _ => {}
            }
        }

        if app.model_picker.stage != crate::ModelPickerStage::Backend
            || !app.model_picker.labels.is_empty()
        {
            if !app.model_picker.labels.is_empty() {
                match key {
                    KeyCode::Up => {
                        app.model_picker.sel = app.model_picker.sel.saturating_sub(1);
                        screen.model_picker_sel = app.model_picker.sel;
                        screen.refresh();
                        return;
                    }
                    KeyCode::Down => {
                        let max = app.model_picker.labels.len().saturating_sub(1);
                        app.model_picker.sel = (app.model_picker.sel + 1).min(max);
                        screen.model_picker_sel = app.model_picker.sel;
                        screen.refresh();
                        return;
                    }
                    KeyCode::Enter | KeyCode::Tab => {
                        select_model_item(app, screen);
                        return;
                    }
                    KeyCode::Esc if modifiers.is_empty() => {
                        if app.model_picker.stage == crate::ModelPickerStage::Model {
                            enter_model_picker_backend_stage(app, screen);
                        } else {
                            cancel_model_picker(app, screen);
                            clear_input_buffer(app, screen);
                        }
                        screen.refresh();
                        return;
                    }
                    _ => {
                        return;
                    }
                }
            }
        }

        if app.cmd_picker.query.is_some() {
            match key {
                KeyCode::Up => {
                    app.cmd_picker.sel = app.cmd_picker.sel.saturating_sub(1);
                    screen.command_sel = app.cmd_picker.sel;
                    screen.refresh();
                    return;
                }
                KeyCode::Down => {
                    let max = app.cmd_picker.candidates.len().saturating_sub(1);
                    app.cmd_picker.sel = (app.cmd_picker.sel + 1).min(max);
                    screen.command_sel = app.cmd_picker.sel;
                    screen.refresh();
                    return;
                }
                KeyCode::Enter | KeyCode::Tab => {
                    if app.cmd_picker.candidates.is_empty() {
                        cancel_command_mode(app, screen);
                        clear_input_buffer(app, screen);
                    } else {
                        select_command(app, screen);
                    }
                    screen.refresh();
                    return;
                }
                KeyCode::Esc if modifiers.is_empty() => {
                    cancel_command_mode(app, screen);
                    clear_input_buffer(app, screen);
                    screen.refresh();
                    return;
                }
                KeyCode::Backspace => {
                    let query = app.cmd_picker.query.as_mut().unwrap();
                    if query.is_empty() {
                        screen.delete_char_before_cursor();
                        cancel_command_mode(app, screen);
                    } else {
                        query.pop();
                        screen.delete_char_before_cursor();
                        let q = query.clone();
                        update_command_candidates(app, screen, &q);
                    }
                    screen.refresh();
                    return;
                }
                KeyCode::Char(c) if modifiers.is_empty() || modifiers == KeyModifiers::SHIFT => {
                    let query = app.cmd_picker.query.as_mut().unwrap();
                    query.push(c);
                    screen.insert_char_at_cursor(c);
                    let q = query.clone();
                    update_command_candidates(app, screen, &q);
                    screen.refresh();
                    return;
                }
                _ => {}
            }
        }

        match key {
            KeyCode::Char('j') if modifiers.contains(KeyModifiers::CONTROL) => {
                screen.insert_char_at_cursor('\n');
                screen.refresh();
            }
            KeyCode::Enter
                if modifiers.contains(KeyModifiers::SHIFT)
                    || modifiers.contains(KeyModifiers::CONTROL) =>
            {
                screen.insert_char_at_cursor('\n');
                screen.refresh();
            }
            KeyCode::Enter => {
                let raw = expand_input_text(app, &screen.input);
                let task = if let Some((ph, content)) = app.cmd_picker.pending_template.take() {
                    app.task_display_override = Some(raw.trim().to_string());
                    raw.replace(&ph, &content).trim().to_string()
                } else {
                    raw.trim().to_string()
                };
                if !task.is_empty() {
                    let at_file_chunks = std::mem::take(&mut app.at_file.chunks);
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
                if app.at_file.query.is_some() {
                    cancel_at_file_mode(app, screen);
                } else {
                    screen.input_focused = false;
                }
                screen.refresh();
            }
            KeyCode::Left if modifiers.contains(KeyModifiers::CONTROL) => {
                screen.cursor_word_left();
                screen.refresh();
            }
            KeyCode::Right if modifiers.contains(KeyModifiers::CONTROL) => {
                screen.cursor_word_right();
                screen.refresh();
            }
            KeyCode::Left => {
                screen.cursor_left();
                screen.refresh();
            }
            KeyCode::Right => {
                screen.cursor_right();
                screen.refresh();
            }
            KeyCode::Up if screen.input.contains('\n') => {
                screen.cursor_up();
                screen.refresh();
            }
            KeyCode::Down if screen.input.contains('\n') => {
                screen.cursor_down();
                screen.refresh();
            }
            KeyCode::Home => {
                screen.cursor_home();
                screen.refresh();
            }
            KeyCode::End => {
                screen.cursor_end();
                screen.refresh();
            }
            KeyCode::Char(c) if modifiers.is_empty() || modifiers == KeyModifiers::SHIFT => {
                insert_char_with_trigger(app, screen, c);
                screen.refresh();
            }
            KeyCode::Backspace => {
                pop_input_at_cursor(app, screen);
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
                insert_char_with_trigger(app, screen, c);
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

pub(super) fn handle_running_mode(
    app: &mut App,
    screen: &mut Screen,
    key: KeyCode,
    modifiers: KeyModifiers,
) {
    match key {
        KeyCode::Enter
            if !modifiers.contains(KeyModifiers::SHIFT)
                && !modifiers.contains(KeyModifiers::CONTROL) =>
        {
            let raw = expand_input_text(app, &screen.input);
            let task = raw.trim().to_string();
            if !task.is_empty() {
                let at_file_chunks = std::mem::take(&mut app.at_file.chunks);
                cancel_at_file_mode(app, screen);
                let final_task = attach_files_to_task(&at_file_chunks, &task);
                clear_input_buffer(app, screen);
                let preview: String = final_task.chars().take(40).collect();
                let queue_len = app.enqueue_message(screen, final_task);
                screen.status = format!("Queued ({queue_len}): {preview}")
                    .dark_yellow()
                    .to_string();
                screen.refresh();
            }
        }
        KeyCode::Enter
            if modifiers.contains(KeyModifiers::SHIFT)
                || modifiers.contains(KeyModifiers::CONTROL) =>
        {
            screen.insert_char_at_cursor('\n');
            screen.refresh();
        }
        KeyCode::Left => {
            screen.cursor_left();
            screen.refresh();
        }
        KeyCode::Right => {
            screen.cursor_right();
            screen.refresh();
        }
        KeyCode::Home => {
            screen.cursor_home();
            screen.refresh();
        }
        KeyCode::End => {
            screen.cursor_end();
            screen.refresh();
        }
        KeyCode::Char(c) if modifiers.is_empty() || modifiers == KeyModifiers::SHIFT => {
            screen.input_focused = true;
            insert_char_with_trigger(app, screen, c);
            screen.refresh();
        }
        KeyCode::Backspace => {
            screen.input_focused = true;
            pop_input_at_cursor(app, screen);
            screen.refresh();
        }
        _ => {}
    }
}

pub(super) fn begin_confirm_note_mode(
    app: &mut App,
    screen: &mut Screen,
    first_char: Option<char>,
) {
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
        screen.insert_char_at_cursor(c);
    }
    screen.refresh();
}

pub(super) fn exit_confirm_note_mode(app: &mut App, screen: &mut Screen, back_to_menu: bool) {
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
