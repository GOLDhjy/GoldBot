use std::path::Path;

use crate::{
    tools::{
        safety::{RiskLevel, assess_command},
        shell::run_command,
    },
    types::ExecutorOutcome,
};

#[derive(Debug, Clone)]
pub struct ValidationReport {
    pub outcome: ExecutorOutcome,
    pub summary: String,
    pub exit_code: i32,
}

#[derive(Debug, Clone)]
pub struct CommitReport {
    pub outcome: ExecutorOutcome,
    pub summary: String,
    pub exit_code: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReviewDecision {
    Pass,
    Blocked(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecDecision {
    Pass,
    Fail(String),
}

pub fn validate_done_when(done_when: &[String], cwd: &Path) -> ValidationReport {
    let _ = cwd;
    if done_when.is_empty() {
        return ValidationReport {
            outcome: ExecutorOutcome::Success,
            summary: "No done_when constraints. Treated as pass.".to_string(),
            exit_code: 0,
        };
    }

    let mut notes = Vec::new();
    for cond in done_when {
        let trimmed = cond.trim();
        if let Some(cmd) = trimmed.strip_prefix("cmd:") {
            let cmd = cmd.trim();
            if cmd.is_empty() {
                return ValidationReport {
                    outcome: ExecutorOutcome::Failed,
                    summary: "Empty done_when command.".to_string(),
                    exit_code: -1,
                };
            }
            let (risk, reason) = assess_command(cmd);
            if risk != RiskLevel::Safe {
                return ValidationReport {
                    outcome: ExecutorOutcome::BlockedSafety,
                    summary: format!("Blocked done_when command `{cmd}`: {reason}"),
                    exit_code: -1,
                };
            }

            match run_command(cmd, None) {
                Ok(out) => {
                    notes.push(format!("cmd `{cmd}` => exit {}", out.exit_code));
                    if out.exit_code != 0 {
                        return ValidationReport {
                            outcome: ExecutorOutcome::Failed,
                            summary: format!(
                                "done_when command failed: `{cmd}` | {}",
                                truncate(&out.output, 240)
                            ),
                            exit_code: out.exit_code,
                        };
                    }
                }
                Err(e) => {
                    return ValidationReport {
                        outcome: ExecutorOutcome::Failed,
                        summary: format!("failed to run done_when command `{cmd}`: {e}"),
                        exit_code: -1,
                    };
                }
            }
        } else {
            notes.push(format!("semantic: {trimmed}"));
        }
    }

    ValidationReport {
        outcome: ExecutorOutcome::Success,
        summary: notes.join(" | "),
        exit_code: 0,
    }
}

pub fn codex_review_decision(output: &str, exit_code: i32) -> ReviewDecision {
    if exit_code != 0 {
        return ReviewDecision::Blocked(format!("non_zero_exit:{exit_code}"));
    }
    let relevant = codex_relevant_output(output);

    if let Some(verdict) = extract_verdict(&relevant) {
        return if verdict == "blocked" {
            ReviewDecision::Blocked("explicit_verdict_blocked".to_string())
        } else {
            ReviewDecision::Pass
        };
    }

    let lower = relevant.to_lowercase();
    if has_any(
        &lower,
        &[
            "阻塞问题：无",
            "阻塞问题: 无",
            "无阻塞问题",
            "no blockers",
            "no blocker",
            "no blocking issues",
            "blockers: none",
            "blocking issues: none",
            "可判定完成",
            "can be considered complete",
        ],
    ) {
        return ReviewDecision::Pass;
    }

    if has_any(
        &lower,
        &[
            "p0",
            "p1",
            "p2",
            "priority 0",
            "priority 1",
            "priority 2",
            "blocking issue",
            "blocking issues",
            "blocker",
            "阻塞问题",
            "阻塞项",
        ],
    ) {
        return ReviewDecision::Blocked("blocking_terms_detected".to_string());
    }

    // Enforce deterministic contract: reviewer must provide explicit verdict line.
    ReviewDecision::Blocked("missing_explicit_verdict".to_string())
}

pub fn claude_exec_decision(output: &str, exit_code: i32) -> ExecDecision {
    if exit_code != 0 {
        return ExecDecision::Fail(format!("non_zero_exit:{exit_code}"));
    }
    let relevant = codex_relevant_output(output);
    for line in relevant.lines().rev() {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix("GE_EXEC_VERDICT:") {
            let v = rest.trim().to_lowercase();
            if v.starts_with("pass") {
                return ExecDecision::Pass;
            }
            if v.starts_with("fail") {
                return ExecDecision::Fail("explicit_exec_fail".to_string());
            }
        }
    }
    ExecDecision::Fail("missing_exec_verdict".to_string())
}

fn codex_relevant_output(output: &str) -> String {
    if let Some(idx) = output.rfind("\nuser\n") {
        return output[..idx].to_string();
    }
    output.to_string()
}

fn extract_verdict(text: &str) -> Option<&'static str> {
    for line in text.lines().rev() {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix("GE_REVIEW_VERDICT:") {
            let v = rest.trim().to_lowercase();
            if v.starts_with("pass") {
                return Some("pass");
            }
            if v.starts_with("blocked") {
                return Some("blocked");
            }
        }
        if let Some(rest) = t.strip_prefix("GE_VERDICT:") {
            let v = rest.trim().to_lowercase();
            if v.starts_with("pass") {
                return Some("pass");
            }
            if v.starts_with("blocked") {
                return Some("blocked");
            }
        }
    }
    None
}

fn has_any(hay: &str, needles: &[&str]) -> bool {
    needles.iter().any(|n| hay.contains(n))
}

pub fn self_review(cwd: &Path) -> ValidationReport {
    let _ = cwd;
    match run_command("git rev-parse --is-inside-work-tree", None) {
        Ok(out) if out.exit_code == 0 => {}
        Ok(out) => {
            return ValidationReport {
                outcome: ExecutorOutcome::Failed,
                summary: format!("not a git repository: {}", truncate(&out.output, 220)),
                exit_code: out.exit_code,
            };
        }
        Err(e) => {
            return ValidationReport {
                outcome: ExecutorOutcome::Failed,
                summary: format!("failed to check git repository: {e}"),
                exit_code: -1,
            };
        }
    }

    match run_command("git diff --check", None) {
        Ok(out) if out.exit_code == 0 => {}
        Ok(out) => {
            return ValidationReport {
                outcome: ExecutorOutcome::Failed,
                summary: format!("git diff --check failed: {}", truncate(&out.output, 260)),
                exit_code: out.exit_code,
            };
        }
        Err(e) => {
            return ValidationReport {
                outcome: ExecutorOutcome::Failed,
                summary: format!("failed to run git diff --check: {e}"),
                exit_code: -1,
            };
        }
    }

    let status = run_command("git status --short", None)
        .map(|o| truncate(&o.output, 220))
        .unwrap_or_else(|e| format!("status unavailable: {e}"));
    let stat = run_command("git diff --stat", None)
        .map(|o| truncate(&o.output, 220))
        .unwrap_or_else(|e| format!("diffstat unavailable: {e}"));

    ValidationReport {
        outcome: ExecutorOutcome::Success,
        summary: format!("status: {} | diff: {}", status, stat),
        exit_code: 0,
    }
}

pub fn commit_todo(todo_id: &str, todo_text: &str) -> CommitReport {
    let msg = format!("GE({todo_id}): {}", shorten_for_commit(todo_text));
    let quoted = shell_single_quote(&msg);
    if let Err(e) = run_command("git add -A -- . ':(exclude)GE_LOG.jsonl'", None) {
        return CommitReport {
            outcome: ExecutorOutcome::Failed,
            summary: format!("git add failed: {e}"),
            exit_code: -1,
        };
    }

    let commit_cmd = format!("git commit --allow-empty -m {quoted}");
    let commit_out = match run_command(&commit_cmd, None) {
        Ok(out) => out,
        Err(e) => {
            return CommitReport {
                outcome: ExecutorOutcome::Failed,
                summary: format!("git commit failed to start: {e}"),
                exit_code: -1,
            };
        }
    };
    if commit_out.exit_code != 0 {
        return CommitReport {
            outcome: ExecutorOutcome::Failed,
            summary: format!("git commit failed: {}", truncate(&commit_out.output, 280)),
            exit_code: commit_out.exit_code,
        };
    }

    let show = run_command("git show --stat --oneline --no-color -1", None)
        .map(|o| truncate(&o.output, 320))
        .unwrap_or_else(|e| format!("commit created; show failed: {e}"));
    CommitReport {
        outcome: ExecutorOutcome::Success,
        summary: show,
        exit_code: 0,
    }
}

pub fn latest_commit_context() -> Option<String> {
    let out = run_command("git show --stat --oneline --no-color -1", None).ok()?;
    if out.exit_code != 0 {
        return None;
    }
    Some(truncate(&out.output, 500))
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    s.chars().take(max).collect()
}

fn shorten_for_commit(s: &str) -> String {
    let one = s.split_whitespace().collect::<Vec<_>>().join(" ");
    truncate(&one, 64)
}

fn shell_single_quote(s: &str) -> String {
    let escaped = s.replace('\'', "'\"'\"'");
    format!("'{escaped}'")
}

#[cfg(test)]
mod tests {
    use super::{ExecDecision, ReviewDecision, claude_exec_decision, codex_review_decision};

    #[test]
    fn codex_review_blocks_ignores_prompt_echo_blocking_phrase() {
        let out = "阻塞问题：无。\nGE_REVIEW_VERDICT: PASS\nOpenAI Codex v0\nuser\nReport blocking issues only.";
        assert_eq!(codex_review_decision(out, 0), ReviewDecision::Pass);
    }

    #[test]
    fn codex_review_blocks_true_on_explicit_blocked_verdict() {
        let out = "Some notes\nGE_REVIEW_VERDICT: BLOCKED - missing validation";
        assert_eq!(
            codex_review_decision(out, 0),
            ReviewDecision::Blocked("explicit_verdict_blocked".to_string())
        );
    }

    #[test]
    fn codex_review_decision_requires_explicit_verdict_when_ambiguous() {
        let out = "Checked files. Looks fine.";
        assert_eq!(
            codex_review_decision(out, 0),
            ReviewDecision::Blocked("missing_explicit_verdict".to_string())
        );
    }

    #[test]
    fn codex_review_decision_pass_on_explicit_pass_verdict() {
        let out = "All checks done.\nGE_REVIEW_VERDICT: PASS";
        assert_eq!(codex_review_decision(out, 0), ReviewDecision::Pass);
    }

    #[test]
    fn claude_exec_decision_pass_on_explicit_verdict() {
        let out = "changed files...\nGE_EXEC_VERDICT: PASS";
        assert_eq!(claude_exec_decision(out, 0), ExecDecision::Pass);
    }

    #[test]
    fn claude_exec_decision_fails_without_verdict() {
        let out = "done.";
        assert_eq!(
            claude_exec_decision(out, 0),
            ExecDecision::Fail("missing_exec_verdict".to_string())
        );
    }
}
