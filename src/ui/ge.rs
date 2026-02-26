use crossterm::style::Stylize;

use crate::App;
use crate::types::Mode;
use crate::ui::screen::{Screen, strip_ansi};

pub(crate) fn drain_ge_events(app: &mut App, screen: &mut Screen) {
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
        app.needs_agent_executor = false;
        app.pending_confirm = None;

        app.pending_confirm_note = false;
        screen.status.clear();
        screen.refresh();
    }
}

pub(crate) fn is_ge_mode(mode: Mode) -> bool {
    matches!(mode, Mode::GeInterview | Mode::GeRun | Mode::GeIdle)
}

pub(crate) fn parse_ge_command(task: &str) -> Option<&str> {
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
        return line.grey().to_string();
    }
    if trimmed.starts_with("GE controls:") || trimmed.starts_with("Audit log:") {
        return line.grey().to_string();
    }
    if trimmed == "GE: input received." {
        return line.grey().to_string();
    }
    if trimmed.starts_with("GE: Planning") {
        return line.dark_yellow().bold().to_string();
    }
    if trimmed.starts_with("GE: Working on current todo")
        || trimmed.starts_with("GE: still running current step")
    {
        return line.dark_yellow().to_string();
    }
    if trimmed.starts_with("================================") {
        return line.grey().to_string();
    }
    if trimmed.starts_with("GE STAGE [") {
        return line.cyan().bold().to_string();
    }
    if trimmed.starts_with("GE PROMPT EXPANDED [") {
        return line.yellow().bold().to_string();
    }
    if trimmed.starts_with("GE RESULT EXPANDED [") {
        return line.yellow().bold().to_string();
    }
    if trimmed.starts_with("GE ->") && trimmed.contains("prompt:") {
        if trimmed.contains("[collapsed,") {
            return line.dark_cyan().to_string();
        }
        return line.cyan().to_string();
    }
    if trimmed.starts_with("GE <-") && trimmed.contains("result:") {
        if trimmed.contains("status=success") {
            return line.green().to_string();
        }
        return line.dark_yellow().to_string();
    }
    if trimmed.starts_with('T') && trimmed.contains("->") && trimmed.contains("prompt:") {
        if trimmed.contains("[collapsed,") {
            return line.dark_cyan().to_string();
        }
        return line.cyan().to_string();
    }
    if trimmed.starts_with('T') && trimmed.contains("<-") && trimmed.contains("result:") {
        if trimmed.contains("status=success") {
            return line.green().to_string();
        }
        return line.dark_yellow().to_string();
    }
    if trimmed.starts_with("...(truncated in console view;") {
        return line.grey().to_string();
    }
    if trimmed.starts_with("Summary:") {
        return line.white().bold().to_string();
    }
    if trimmed.starts_with("...(collapsed ") {
        return line.grey().to_string();
    }
    if trimmed.starts_with("GE: use `GE 展开提示词`") {
        return line.grey().to_string();
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
        return line.grey().to_string();
    }
    if trimmed.starts_with("GE:") {
        return line.dark_yellow().to_string();
    }
    line.to_string()
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
