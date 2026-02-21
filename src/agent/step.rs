use crossterm::style::Stylize;
use serde_json::Value;

use crate::agent::provider::Message;
use crate::agent::react::parse_llm_response;
use crate::memory::store::MemoryStore;
use crate::tools::safety::{RiskLevel, assess_command};
use crate::types::{Event, LlmAction, Mode};
use crate::ui::format::{
    collapsed_lines, emit_live_event, sanitize_final_summary_for_tui, shorten_text,
};
use crate::ui::screen::Screen;
use crate::{
    App, KEEP_RECENT_MESSAGES_AFTER_COMPACTION, MAX_COMPACTION_SUMMARY_ITEMS,
    MAX_MESSAGES_BEFORE_COMPACTION,
};

pub(crate) fn start_task(app: &mut App, screen: &mut Screen, task: String) {
    if app.messages.len() > 1 {
        screen.emit(&[String::new()]);
    }
    screen.reset_task_lines();

    app.task = task.clone();
    app.steps_taken = 0;
    app.running = true;
    app.llm_stream_preview.clear();
    app.llm_preview_shown.clear();
    app.needs_agent_step = true;
    app.pending_confirm = None;
    app.pending_confirm_note = false;
    screen.confirm_selected = None;
    screen.input_focused = true;
    app.task_events.clear();
    app.final_summary = None;
    app.task_collapsed = false;
    app.messages.push(Message::user(task.clone()));

    emit_live_event(screen, &Event::UserTask { text: task });
}

pub(crate) fn process_llm_result(
    app: &mut App,
    screen: &mut Screen,
    result: anyhow::Result<String>,
) {
    if app.steps_taken >= app.max_steps {
        finish(
            app,
            screen,
            format!("Reached max steps ({}).", app.max_steps),
        );
        return;
    }
    app.steps_taken += 1;

    let response = match result {
        Ok(r) => r,
        Err(e) => {
            let ev = Event::Thinking {
                text: format!("[LLM error] {e}"),
            };
            emit_live_event(screen, &ev);
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
                "Your last response could not be parsed. Use one of:\n\
                 <thought>…</thought><tool>shell</tool><command>…</command>\n\
                 <thought>…</thought><tool>mcp_…</tool><arguments>{}</arguments>\n\
                 <thought>…</thought><final>…</final>"
                    .to_string(),
            ));
            screen.status = format!("↻ Retrying invalid response format: {e}")
                .dark_grey()
                .to_string();
            screen.refresh();
            app.needs_agent_step = true;
            return;
        }
    };

    if !thought.is_empty() {
        let ev = Event::Thinking { text: thought };
        if app.show_thinking {
            emit_live_event(screen, &ev);
        }
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
                    if matches!(app.mode, Mode::GeInterview | Mode::GeRun | Mode::GeIdle) {
                        let ev = Event::Thinking {
                            text: format!("GE auto-approved confirm command: {command}"),
                        };
                        emit_live_event(screen, &ev);
                        app.task_events.push(ev);
                        execute_command(app, screen, &command);
                        app.needs_agent_step = true;
                    } else {
                        let ev = Event::NeedsConfirmation {
                            command: command.clone(),
                            reason,
                        };
                        emit_live_event(screen, &ev);
                        app.task_events.push(ev);
                        app.pending_confirm = Some(command);
                        app.pending_confirm_note = false;
                        screen.confirm_selected = Some(0);
                        screen.input_focused = false;
                        screen.refresh();
                    }
                }
                RiskLevel::Block => {
                    let msg = "Command blocked by safety policy";
                    app.messages
                        .push(Message::user(format!("Tool result:\n{msg}")));
                    let ev = Event::ToolResult {
                        exit_code: -1,
                        output: msg.to_string(),
                    };
                    emit_live_event(screen, &ev);
                    app.task_events.push(ev);
                    app.needs_agent_step = true;
                }
            }
        }
        LlmAction::Mcp { tool, arguments } => {
            execute_mcp_tool(app, screen, &tool, &arguments);
            app.needs_agent_step = true;
        }
        LlmAction::Skill { name } => {
            load_skill(app, screen, &name);
            app.needs_agent_step = true;
        }
        LlmAction::CreateMcp { config } => {
            create_mcp(app, screen, &config);
            app.needs_agent_step = true;
        }
        LlmAction::Final { summary } => {
            finish(app, screen, summary);
        }
    }
}

pub(crate) fn execute_command(app: &mut App, screen: &mut Screen, cmd: &str) {
    let intent = crate::tools::shell::classify_command(cmd);
    let call_ev = Event::ToolCall {
        label: intent.label(),
        command: cmd.to_string(),
    };
    emit_live_event(screen, &call_ev);
    app.task_events.push(call_ev);

    match crate::tools::shell::run_command(cmd) {
        Ok(out) => {
            app.messages.push(Message::user(format!(
                "Tool result (exit={}):\n{}",
                out.exit_code, out.output
            )));
            let ev = Event::ToolResult {
                exit_code: out.exit_code,
                output: out.output,
            };
            emit_live_event(screen, &ev);
            app.task_events.push(ev);
        }
        Err(e) => {
            let err = format!("execution failed: {e}");
            app.messages
                .push(Message::user(format!("Tool result (exit=-1):\n{err}")));
            let ev = Event::ToolResult {
                exit_code: -1,
                output: err,
            };
            emit_live_event(screen, &ev);
            app.task_events.push(ev);
        }
    }
}

pub(crate) fn execute_mcp_tool(app: &mut App, screen: &mut Screen, tool: &str, arguments: &Value) {
    let args_text = serde_json::to_string(arguments).unwrap_or_else(|_| "{}".to_string());
    let call_ev = Event::ToolCall {
        label: format!("MCP({tool})"),
        command: args_text,
    };
    emit_live_event(screen, &call_ev);
    app.task_events.push(call_ev);

    match app.mcp_registry.execute_tool(tool, arguments) {
        Ok(out) => {
            app.messages.push(Message::user(format!(
                "Tool result (exit={}):\n{}",
                out.exit_code, out.output
            )));
            let ev = Event::ToolResult {
                exit_code: out.exit_code,
                output: out.output,
            };
            emit_live_event(screen, &ev);
            app.task_events.push(ev);
        }
        Err(e) => {
            let err = format!("MCP execution failed: {e}");
            app.messages
                .push(Message::user(format!("Tool result (exit=-1):\n{err}")));
            let ev = Event::ToolResult {
                exit_code: -1,
                output: err,
            };
            emit_live_event(screen, &ev);
            app.task_events.push(ev);
        }
    }
}

pub(crate) fn load_skill(app: &mut App, screen: &mut Screen, name: &str) {
    let call_ev = Event::ToolCall {
        label: format!("Skill({name})"),
        command: String::new(),
    };
    emit_live_event(screen, &call_ev);
    app.task_events.push(call_ev);

    let msg = if let Some(skill) = app.skills.iter().find(|s| s.name == name) {
        format!("Skill '{}' content:\n{}", name, skill.content)
    } else {
        format!("Skill '{}' not found.", name)
    };
    app.messages.push(Message::user(msg));
}

pub(crate) fn create_mcp(app: &mut App, screen: &mut Screen, config: &serde_json::Value) {
    let call_ev = Event::ToolCall {
        label: "CreateMCP".to_string(),
        command: serde_json::to_string(config).unwrap_or_default(),
    };
    emit_live_event(screen, &call_ev);
    app.task_events.push(call_ev);

    let name = config
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    // create_mcp_server handles spec cleanup (strips name, type, empty fields).
    let (exit_code, result_msg) = match crate::tools::mcp::create_mcp_server(&name, config) {
        Ok(path) => (
            0,
            format!(
                "MCP server `{name}` saved to `{}`. Restart GoldBot to activate it.",
                path.display()
            ),
        ),
        Err(e) => (1, format!("Failed to create MCP server: {e}")),
    };

    let ev = Event::ToolResult {
        exit_code,
        output: result_msg.clone(),
    };
    emit_live_event(screen, &ev);
    app.task_events.push(ev);
    app.messages
        .push(Message::user(format!("Tool result:\n{result_msg}")));
}

pub(crate) fn finish(app: &mut App, screen: &mut Screen, summary: String) {
    let summary = sanitize_final_summary_for_tui(&summary);
    app.final_summary = Some(summary.clone());
    app.task_collapsed = true;

    screen.collapse_to(&collapsed_lines(app));

    let store = MemoryStore::new();
    let _ = store.append_short_term(&app.task, &summary);
    for note in store.derive_long_term_notes(&app.task, &summary) {
        let _ = store.append_long_term_if_new(&note);
    }
    let _ = store.promote_repeated_short_term_to_long_term();

    app.running = false;
    app.llm_stream_preview.clear();
    app.llm_preview_shown.clear();
    app.pending_confirm = None;
    app.pending_confirm_note = false;
    screen.confirm_selected = None;
    screen.input_focused = true;
    screen.status = "[Ctrl+d] full details".dark_grey().to_string();
    screen.refresh();
}

/// Called with native thinking block deltas from the LLM.
/// Shows them directly in the status bar as preview.
pub(crate) fn handle_llm_thinking_delta(app: &mut App, screen: &mut Screen, chunk: &str) {
    if !app.llm_calling || chunk.is_empty() {
        return;
    }

    app.llm_stream_preview.push_str(chunk);
    trim_left_to_max_bytes(&mut app.llm_stream_preview, 16_384);

    let collapsed = app
        .llm_stream_preview
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let preview = tail_chars(&collapsed, 240);
    if preview.is_empty() {
        return;
    }

    let punctuation_flush = preview.ends_with(['。', '！', '？', '.', '!', '?', ';', '；']);
    let grew_by = preview
        .chars()
        .count()
        .saturating_sub(app.llm_preview_shown.chars().count());
    let should_refresh = app.llm_preview_shown.is_empty()
        || preview.chars().count() < app.llm_preview_shown.chars().count()
        || !preview.starts_with(&app.llm_preview_shown)
        || grew_by >= 8
        || punctuation_flush;
    if !should_refresh {
        return;
    }

    app.llm_preview_shown = preview.clone();
    screen.status = format!("⏳ {}", preview);
    screen.refresh();
}

pub(crate) fn handle_llm_stream_delta(app: &mut App, screen: &mut Screen, delta: &str) {
    if !app.llm_calling || delta.is_empty() {
        return;
    }

    app.llm_stream_preview.push_str(delta);
    trim_left_to_max_bytes(&mut app.llm_stream_preview, 16_384);

    // When native thinking is on, preview comes from ThinkingDelta events; skip here.
    if app.show_thinking {
        return;
    }

    let preview = extract_live_preview(&app.llm_stream_preview);
    if preview.is_empty() {
        return;
    }

    let punctuation_flush = preview.ends_with(['。', '！', '？', '.', '!', '?', ';', '；']);
    let grew_by = preview
        .chars()
        .count()
        .saturating_sub(app.llm_preview_shown.chars().count());
    let should_refresh = app.llm_preview_shown.is_empty()
        || preview.chars().count() < app.llm_preview_shown.chars().count()
        || !preview.starts_with(&app.llm_preview_shown)
        || grew_by >= 8
        || punctuation_flush;
    if !should_refresh {
        return;
    }

    app.llm_preview_shown = preview.clone();
    screen.status = format!("⏳ {}", preview);
    screen.refresh();
}

fn extract_live_preview(raw: &str) -> String {
    let mut s = if let Some(start) = raw.rfind("<thought>") {
        &raw[start + "<thought>".len()..]
    } else {
        raw
    };
    if let Some(end) = s.rfind("</thought>") {
        s = &s[..end];
    }

    let no_tags = strip_xml_tags(s);
    let collapsed = no_tags.split_whitespace().collect::<Vec<_>>().join(" ");
    tail_chars(&collapsed, 240)
}

fn strip_xml_tags(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for ch in s.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(ch),
            _ => {}
        }
    }
    out
}

fn tail_chars(s: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }
    let count = s.chars().count();
    if count <= max_chars {
        return s.to_string();
    }
    s.chars().skip(count - max_chars).collect()
}

fn trim_left_to_max_bytes(s: &mut String, max_bytes: usize) {
    if s.len() <= max_bytes {
        return;
    }

    let mut cut = s.len().saturating_sub(max_bytes);
    while cut < s.len() && !s.is_char_boundary(cut) {
        cut += 1;
    }
    s.drain(..cut);
}

pub(crate) fn maybe_flush_and_compact_before_call(app: &mut App, screen: &mut Screen) {
    if app.messages.len() <= MAX_MESSAGES_BEFORE_COMPACTION {
        return;
    }
    if app.messages.len() <= KEEP_RECENT_MESSAGES_AFTER_COMPACTION + 1 {
        return;
    }

    let split_at = app
        .messages
        .len()
        .saturating_sub(KEEP_RECENT_MESSAGES_AFTER_COMPACTION);
    if split_at <= 1 {
        return;
    }

    let older: Vec<Message> = app.messages[1..split_at].to_vec();
    let store = MemoryStore::new();
    let mut flushed = 0usize;
    let mut last_user_task: Option<String> = None;

    for msg in &older {
        match msg.role {
            crate::agent::provider::Role::User => {
                if msg.content.starts_with("Tool result")
                    || msg
                        .content
                        .starts_with("Your last response could not be parsed")
                    || msg.content.starts_with("[Context compacted]")
                {
                    continue;
                }
                last_user_task = Some(msg.content.clone());
            }
            crate::agent::provider::Role::Assistant => {
                if let Some(final_text) = extract_last_tag_text(&msg.content, "final") {
                    if let Some(task) = last_user_task.as_deref() {
                        for note in store.derive_long_term_notes(task, &final_text) {
                            if let Ok(true) = store.append_long_term_if_new(&note) {
                                flushed += 1;
                            }
                        }
                    }
                }
            }
            crate::agent::provider::Role::System => {}
        }
    }

    let summary = summarize_for_compaction(&older);
    let mut compacted = Vec::new();
    compacted.push(app.messages[0].clone());
    if !summary.is_empty() {
        compacted.push(Message::user(format!("[Context compacted]\n{summary}")));
    }
    compacted.extend_from_slice(&app.messages[split_at..]);
    app.messages = compacted;

    screen.status = if flushed > 0 {
        format!("🧠 pre-compaction flush: {flushed} long-term notes")
            .dark_grey()
            .to_string()
    } else {
        "🧠 context compacted".dark_grey().to_string()
    };
    screen.refresh();
}

fn summarize_for_compaction(messages: &[Message]) -> String {
    let mut items = Vec::new();
    for msg in messages.iter().rev() {
        match msg.role {
            crate::agent::provider::Role::User => {
                if msg.content.starts_with("Tool result")
                    || msg
                        .content
                        .starts_with("Your last response could not be parsed")
                    || msg.content.starts_with("[Context compacted]")
                {
                    continue;
                }
                let one_line = msg.content.split_whitespace().collect::<Vec<_>>().join(" ");
                items.push(format!("- user: {}", shorten_text(&one_line, 120)));
            }
            crate::agent::provider::Role::Assistant => {
                if let Some(final_text) = extract_last_tag_text(&msg.content, "final") {
                    let one_line = final_text.split_whitespace().collect::<Vec<_>>().join(" ");
                    items.push(format!("- final: {}", shorten_text(&one_line, 120)));
                }
            }
            crate::agent::provider::Role::System => {}
        }
        if items.len() >= MAX_COMPACTION_SUMMARY_ITEMS {
            break;
        }
    }
    items.reverse();
    items.join("\n")
}

fn extract_last_tag_text(text: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let end = text.rfind(&close)?;
    let head = &text[..end];
    let start = head.rfind(&open)? + open.len();
    Some(head[start..].trim().to_string())
}
