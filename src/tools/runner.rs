use std::process::Command;

use anyhow::Result;

pub struct CommandResult {
    pub exit_code: i32,
    pub output: String,
}

pub fn run_command(cmd: &str) -> Result<CommandResult> {
    let output = if cfg!(target_os = "windows") {
        Command::new("powershell")
            .args(["-NoProfile", "-Command", cmd])
            .output()?
    } else {
        Command::new("bash").args(["-lc", cmd]).output()?
    };

    let mut text = String::new();
    text.push_str(&String::from_utf8_lossy(&output.stdout));
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    if text.len() > 8_000 {
        text.truncate(8_000);
        text.push_str("\n...[truncated]");
    }

    Ok(CommandResult {
        exit_code: output.status.code().unwrap_or(-1),
        output: text,
    })
}
