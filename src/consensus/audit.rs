use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use chrono::Local;
use serde_json::json;

use crate::types::{AuditEventKind, ConsensusTrigger, ExecutorOutcome, Mode};

const SUMMARY_LIMIT_CHARS: usize = 600;
const COMMAND_LIMIT_CHARS: usize = 260;

#[derive(Debug, Clone)]
pub struct AuditLogger {
    path: PathBuf,
    run_id: String,
}

#[derive(Debug, Clone)]
pub struct AuditRecord<'a> {
    pub mode: Mode,
    pub event: AuditEventKind,
    pub todo_id: Option<&'a str>,
    pub trigger: Option<ConsensusTrigger>,
    pub executor: Option<&'a str>,
    pub command: Option<&'a str>,
    pub exit_code: Option<i32>,
    pub status: ExecutorOutcome,
    pub summary: Option<&'a str>,
    pub error_code: Option<&'a str>,
}

impl AuditLogger {
    pub fn new(consensus_path: &Path) -> Self {
        let dir = consensus_path.parent().unwrap_or_else(|| Path::new("."));
        let path = dir.join("GE_LOG.jsonl");
        let run_id = format!("ge-{}", Local::now().format("%Y%m%d-%H%M%S"));
        Self { path, run_id }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn write(&self, rec: AuditRecord<'_>) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .with_context(|| format!("failed to open `{}`", self.path.display()))?;

        let line = json!({
            "ts": Local::now().to_rfc3339(),
            "run_id": self.run_id,
            "mode": mode_str(rec.mode),
            "event": rec.event.as_str(),
            "todo_id": rec.todo_id,
            "trigger": rec.trigger.map(|t| t.as_str()),
            "executor": rec.executor,
            "command": rec.command.map(|c| truncate_chars(c, COMMAND_LIMIT_CHARS)),
            "exit_code": rec.exit_code,
            "status": rec.status.as_status(),
            "summary": rec.summary.map(|s| truncate_chars(s, SUMMARY_LIMIT_CHARS)),
            "error_code": rec.error_code,
        });

        writeln!(file, "{}", line)?;
        Ok(())
    }
}

fn truncate_chars(s: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    if s.chars().count() <= max {
        return s.to_string();
    }
    if max <= 14 {
        return "…(truncated)".chars().take(max).collect();
    }
    let keep = max - 13;
    let mut out = String::new();
    for (idx, ch) in s.chars().enumerate() {
        if idx >= keep {
            break;
        }
        out.push(ch);
    }
    out.push_str("…(truncated)");
    out
}

fn mode_str(mode: Mode) -> &'static str {
    match mode {
        Mode::Normal => "Normal",
        Mode::GeInterview => "GeInterview",
        Mode::GeRun => "GeRun",
        Mode::GeIdle => "GeIdle",
    }
}
