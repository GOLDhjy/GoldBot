use crossterm::{event::KeyCode, event::KeyModifiers, style::Stylize};

use crate::App;
use crate::types::Mode;
use crate::ui::format::toggle_collapse;
use crate::ui::ge::is_ge_mode;
use crate::ui::screen::Screen;

mod modes;
mod pickers;
mod submit;

use self::modes::{handle_confirm_mode, handle_idle_mode, handle_note_mode, handle_running_mode};
use self::pickers::{enter_at_file_mode, enter_command_mode};
use self::submit::{interrupt_llm_chat_loop, should_interrupt_llm_chat_loop};

pub(crate) use self::pickers::apply_at_file_filter;
pub(crate) use self::submit::handle_paste;

pub(crate) fn handle_key(
    app: &mut App,
    screen: &mut Screen,
    key: KeyCode,
    modifiers: KeyModifiers,
) -> bool {
    if key == KeyCode::Char('c') && modifiers.contains(KeyModifiers::CONTROL) {
        return true;
    }
    if key == KeyCode::Char('v') && modifiers.contains(KeyModifiers::CONTROL) {
        if let Ok(mut cb) = arboard::Clipboard::new() {
            if let Ok(text) = cb.get_text() {
                if !text.is_empty() {
                    handle_paste(app, screen, &text);
                }
            }
        }
        return false;
    }
    if key == KeyCode::Esc && modifiers.is_empty() && should_interrupt_llm_chat_loop(app) {
        interrupt_llm_chat_loop(app, screen);
        return false;
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
        app.rebuild_system_message();
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

pub(super) fn insert_char_with_trigger(app: &mut App, screen: &mut Screen, c: char) {
    screen.insert_char_at_cursor(c);
    if c == '@' {
        enter_at_file_mode(app, screen);
    } else if c == '/' && screen.input == "/" {
        enter_command_mode(app, screen);
    }
}
