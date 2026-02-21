use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use crate::types::ExecutorOutcome;

#[derive(Debug, Clone)]
pub struct ExecutorRun {
    pub executor: &'static str,
    pub command_line: String,
    pub exit_code: i32,
    pub output: String,
    pub outcome: ExecutorOutcome,
    pub error_code: Option<String>,
}

impl ExecutorRun {
    pub fn ok(&self) -> bool {
        self.outcome == ExecutorOutcome::Success
    }
}

pub fn preflight(cwd: &Path, cancel: &Arc<AtomicBool>) -> Vec<ExecutorRun> {
    const PREFLIGHT_PROMPT: &str =
        "Preflight check. Do not edit files or run tools. Reply with one line: OK";
    vec![
        run_claude(cwd, PREFLIGHT_PROMPT, cancel),
        run_codex_execute(cwd, PREFLIGHT_PROMPT, cancel),
    ]
}

pub fn run_claude(cwd: &Path, prompt: &str, cancel: &Arc<AtomicBool>) -> ExecutorRun {
    let args = [
        "-p",
        "--permission-mode",
        "bypassPermissions",
        "--dangerously-skip-permissions",
        prompt,
    ];
    run_process(
        "claude",
        &args,
        cwd,
        "claude",
        "claude -p --permission-mode bypassPermissions --dangerously-skip-permissions <prompt>",
        cancel,
    )
}

pub fn run_codex_execute(cwd: &Path, prompt: &str, cancel: &Arc<AtomicBool>) -> ExecutorRun {
    let args = [
        "exec",
        "--dangerously-bypass-approvals-and-sandbox",
        "--sandbox",
        "danger-full-access",
        "--skip-git-repo-check",
        prompt,
    ];
    run_process(
        "codex",
        &args,
        cwd,
        "codex",
        "codex exec --dangerously-bypass-approvals-and-sandbox --sandbox danger-full-access --skip-git-repo-check <prompt>",
        cancel,
    )
}

fn run_process(
    program: &str,
    args: &[&str],
    cwd: &Path,
    executor: &'static str,
    command_line: &str,
    cancel: &Arc<AtomicBool>,
) -> ExecutorRun {
    if cancel.load(Ordering::SeqCst) {
        return ExecutorRun {
            executor,
            command_line: command_line.to_string(),
            exit_code: 130,
            output: "cancelled by GE hard exit".to_string(),
            outcome: ExecutorOutcome::Failed,
            error_code: Some("cancelled".to_string()),
        };
    }

    let (stdout_path, stderr_path) = temp_capture_paths(program);
    let stdout_file = match fs::File::create(&stdout_path) {
        Ok(f) => f,
        Err(e) => {
            return ExecutorRun {
                executor,
                command_line: command_line.to_string(),
                exit_code: -1,
                output: format!("failed to create stdout capture file: {e}"),
                outcome: ExecutorOutcome::Failed,
                error_code: Some("exec_failed".to_string()),
            };
        }
    };
    let stderr_file = match fs::File::create(&stderr_path) {
        Ok(f) => f,
        Err(e) => {
            let _ = fs::remove_file(&stdout_path);
            return ExecutorRun {
                executor,
                command_line: command_line.to_string(),
                exit_code: -1,
                output: format!("failed to create stderr capture file: {e}"),
                outcome: ExecutorOutcome::Failed,
                error_code: Some("exec_failed".to_string()),
            };
        }
    };

    let mut child = match Command::new(program)
        .args(args)
        .current_dir(cwd)
        .stdout(Stdio::from(stdout_file))
        .stderr(Stdio::from(stderr_file))
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            let _ = fs::remove_file(&stdout_path);
            let _ = fs::remove_file(&stderr_path);
            return ExecutorRun {
                executor,
                command_line: command_line.to_string(),
                exit_code: -1,
                output: format!("failed to run `{program}`: {e}"),
                outcome: ExecutorOutcome::Failed,
                error_code: Some("exec_failed".to_string()),
            };
        }
    };

    let mut cancelled = false;
    let exit_code = loop {
        if cancel.load(Ordering::SeqCst) {
            cancelled = true;
            let _ = child.kill();
        }
        match child.try_wait() {
            Ok(Some(status)) => break status.code().unwrap_or(if cancelled { 130 } else { -1 }),
            Ok(None) => thread::sleep(Duration::from_millis(80)),
            Err(e) => {
                let _ = fs::remove_file(&stdout_path);
                let _ = fs::remove_file(&stderr_path);
                return ExecutorRun {
                    executor,
                    command_line: command_line.to_string(),
                    exit_code: -1,
                    output: format!("failed while waiting for `{program}`: {e}"),
                    outcome: ExecutorOutcome::Failed,
                    error_code: Some("exec_failed".to_string()),
                };
            }
        }
    };

    let mut combined = String::new();
    combined.push_str(&read_capture_file(&stdout_path));
    combined.push_str(&read_capture_file(&stderr_path));
    let _ = fs::remove_file(&stdout_path);
    let _ = fs::remove_file(&stderr_path);
    if cancelled {
        if !combined.ends_with('\n') {
            combined.push('\n');
        }
        combined.push_str("[GE] cancelled by hard exit\n");
    }
    if combined.trim().is_empty() {
        combined = "(no output)".to_string();
    }

    if cancelled {
        return ExecutorRun {
            executor,
            command_line: command_line.to_string(),
            exit_code: 130,
            output: combined,
            outcome: ExecutorOutcome::Failed,
            error_code: Some("cancelled".to_string()),
        };
    }

    let detected = detect_error_code(&combined);
    let (outcome, error_code) = if exit_code == 0 {
        // exit=0 means the executor completed. Ignore non-blocking warnings
        // like optional MCP auth failures from unrelated servers.
        (ExecutorOutcome::Success, None)
    } else if detected.as_deref() == Some("manual_confirm") {
        (ExecutorOutcome::BlockedConfirm, detected)
    } else {
        (ExecutorOutcome::Failed, detected)
    };

    ExecutorRun {
        executor,
        command_line: command_line.to_string(),
        exit_code,
        output: combined,
        outcome,
        error_code,
    }
}

fn detect_error_code(output: &str) -> Option<String> {
    let lower = output.to_lowercase();
    if has_any(
        &lower,
        &[
            "rate limit",
            "too many requests",
            "quota",
            "limit reached",
            "429",
            "套餐",
        ],
    ) {
        return Some("rate_limit".to_string());
    }
    if has_any(
        &lower,
        &[
            "not authenticated",
            "unauthorized",
            "login required",
            "authentication required",
            "invalid api key",
            "expired token",
            "access token",
        ],
    ) {
        return Some("auth_required".to_string());
    }
    if has_any(
        &lower,
        &[
            "requires approval",
            "approval required",
            "need approval",
            "needs approval",
            "needs your approval",
            "awaiting approval",
            "awaiting confirmation",
            "waiting for confirmation",
            "manual confirmation",
            "confirmation required",
            "confirm to continue",
            "press enter to continue",
            "press enter to confirm",
            "type y to continue",
            "type yes to continue",
            "需要确认",
            "请确认后继续",
        ],
    ) {
        return Some("manual_confirm".to_string());
    }
    if has_any(
        &lower,
        &[
            "unknown argument",
            "unexpected argument",
            "cannot be used with",
            "a value is required",
            "requires a value",
            "error: the argument",
            "for more information, try '--help'",
        ],
    ) {
        return Some("invalid_args".to_string());
    }
    None
}

fn has_any(hay: &str, needles: &[&str]) -> bool {
    needles.iter().any(|n| hay.contains(n))
}

pub fn build_claude_prompt(
    purpose: &[String],
    rules: &[String],
    todo_id: &str,
    todo_text: &str,
    done_when: &[String],
    git_context: Option<&str>,
) -> String {
    let context =
        fixed_execution_context(purpose, rules, todo_id, todo_text, done_when, git_context);
    format!(
        "You are Claude execution agent.\n\
         Fixed instruction: strictly follow Purpose and Rules. \
         Execute ONLY the current Todo.\n\
         Do not ask for confirmation. Apply changes directly.\n\n\
         {context}\n\n\
         Return a concise summary of what you changed.\n\
         At the very end, append exactly one verdict line:\n\
         GE_EXEC_VERDICT: PASS\n\
         or\n\
         GE_EXEC_VERDICT: FAIL - <short reason>"
    )
}

pub fn build_codex_optimize_prompt(
    purpose: &[String],
    rules: &[String],
    todo_id: &str,
    todo_text: &str,
    done_when: &[String],
    git_context: Option<&str>,
) -> String {
    let context =
        fixed_execution_context(purpose, rules, todo_id, todo_text, done_when, git_context);
    format!(
        "You are Codex reviewer+optimizer.\n\
         Primary objective: ensure the current Todo is functionally complete and correct.\n\
         Workflow (strict):\n\
         1) Review first and detect blockers / correctness gaps.\n\
         2) If no meaningful issue is found, DO NOT change files and return PASS directly.\n\
         3) If issues exist, apply minimal necessary fixes/optimizations.\n\
         4) Run essential verification for this Todo; only then return PASS.\n\
         5) If still not correct after your attempt, return BLOCKED with short reason.\n\
         Fixed instruction: strictly follow Purpose and Rules. \
         Operate ONLY on the current Todo. Prioritize correctness and tests.\n\
         Keep output minimal to save tokens (no long explanations).\n\
         At the very end, output exactly one verdict line:\n\
         GE_REVIEW_VERDICT: PASS\n\
         or\n\
         GE_REVIEW_VERDICT: BLOCKED - <short reason>\n\n\
         {context}"
    )
}

pub fn build_todo_planner_prompt(purpose: &str, rules: &str, scope: &str) -> String {
    format!(
        "You are a project planner.\n\
         Generate a concrete implementation todo plan for this project.\n\
         Return ONLY JSON in this schema:\n\
         {{\"todos\":[{{\"id\":\"T001\",\"text\":\"...\",\"done_when\":[\"...\"],\"assist\":\"claude|codex|auto\"}}]}}\n\
         Constraints:\n\
         - 8 to 12 todos\n\
         - Every todo must be actionable and specific\n\
         - Break tasks into small steps that are each directly executable\n\
         - done_when must be verifiable and explicit\n\
         - IDs must be sequential T001..T00N\n\
         - Keep scope boundaries strict\n\
         - No markdown, no explanations, JSON only\n\n\
         Purpose:\n{}\n\nRules:\n{}\n\nScope Boundaries:\n{}\n",
        purpose, rules, scope
    )
}

pub fn build_clarify_questions_prompt(purpose: &str, rules: &str, scope: &str) -> String {
    format!(
        "You are a planning interviewer.\n\
         Use planning mode only. Do NOT modify files or run tools.\n\
         Generate clarification questions that help refine project purpose/rules/scope.\n\
         If the available info is already sufficient, return an empty questions list.\n\
         Each question must provide exactly 3 selectable options.\n\
         Return ONLY JSON in this schema:\n\
         {{\"questions\":[{{\"question\":\"...\",\"options\":[\"...\",\"...\",\"...\"]}}]}}\n\
         Constraints:\n\
         - Generate 0 to 8 questions\n\
         - Questions must be actionable and decision-oriented\n\
         - Keep wording short and unambiguous\n\
         - No markdown, no explanations, JSON only\n\n\
         Purpose:\n{}\n\nRules:\n{}\n\nScope Boundaries:\n{}\n",
        purpose, rules, scope
    )
}

pub fn build_followup_clarify_questions_prompt(
    purpose: &str,
    rules: &str,
    scope: &str,
    clarify_answers: &[String],
    round: usize,
) -> String {
    let answers = if clarify_answers.is_empty() {
        "(none)".to_string()
    } else {
        clarify_answers
            .iter()
            .map(|a| format!("- {a}"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    format!(
        "You are a planning interviewer.\n\
         Use planning mode only. Do NOT modify files or run tools.\n\
         Decide whether extra clarifications are still needed after previous answers.\n\
         If still unclear, return more questions; otherwise return an empty list.\n\
         Each question must provide exactly 3 selectable options.\n\
         Return ONLY JSON in this schema:\n\
         {{\"questions\":[{{\"question\":\"...\",\"options\":[\"...\",\"...\",\"...\"]}}]}}\n\
         Constraints:\n\
         - Generate 0 to 8 questions\n\
         - No markdown, no explanations, JSON only\n\
         - Current round: {}\n\n\
         Purpose:\n{}\n\nRules:\n{}\n\nScope Boundaries:\n{}\n\n\
         Clarification Answers So Far:\n{}\n",
        round, purpose, rules, scope, answers
    )
}

pub fn build_consensus_builder_prompt(
    purpose: &str,
    rules: &str,
    scope: &str,
    clarify_answers: &[String],
) -> String {
    let answers = if clarify_answers.is_empty() {
        "(none)".to_string()
    } else {
        clarify_answers
            .iter()
            .map(|a| format!("- {a}"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    format!(
        "You are a consensus document planner.\n\
         Use planning mode only. Do NOT modify files or run tools.\n\
         Produce the final optimized consensus content for this project.\n\
         Return ONLY JSON in this schema:\n\
         {{\"purpose_lines\":[\"...\"],\"rules_lines\":[\"...\"],\"scope\":\"...\",\
         \"todos\":[{{\"id\":\"T001\",\"text\":\"...\",\"done_when\":[\"...\"],\"assist\":\"claude|codex|auto\"}}]}}\n\
         Constraints:\n\
         - purpose_lines must be concrete and concise\n\
         - rules_lines must be enforceable\n\
         - todos must be 8 to 12 items, sequential IDs T001..T00N\n\
         - every todo must be specific and executable\n\
         - every done_when must be verifiable\n\
         - keep scope strict\n\
         - No markdown, no explanations, JSON only\n\n\
         Original Purpose:\n{}\n\nOriginal Rules:\n{}\n\nOriginal Scope:\n{}\n\n\
         Clarification Answers:\n{}\n",
        purpose, rules, scope, answers
    )
}

pub fn summarize_output(text: &str, max_lines: usize) -> String {
    let mut lines = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        lines.push(trimmed.to_string());
        if lines.len() >= max_lines {
            break;
        }
    }
    if lines.is_empty() {
        return "(no meaningful output)".to_string();
    }
    lines.join(" | ")
}

fn fixed_execution_context(
    purpose: &[String],
    rules: &[String],
    todo_id: &str,
    todo_text: &str,
    done_when: &[String],
    git_context: Option<&str>,
) -> String {
    let git_section = git_context
        .map(|g| format!("\n\nRecent Git Context:\n{}\n", g))
        .unwrap_or_default();
    format!(
        "Purpose:\n{}\n\nRules:\n{}\n\nCurrent Todo:\n{} {}\n\nDone When:\n{}{}",
        purpose.join("\n"),
        rules.join("\n"),
        todo_id,
        todo_text,
        done_when
            .iter()
            .map(|d| format!("- {d}"))
            .collect::<Vec<_>>()
            .join("\n"),
        git_section
    )
}

fn temp_capture_paths(program: &str) -> (PathBuf, PathBuf) {
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let pid = std::process::id();
    let base = format!("goldbot-ge-{program}-{pid}-{ts}");
    let dir = std::env::temp_dir();
    (
        dir.join(format!("{base}.stdout.log")),
        dir.join(format!("{base}.stderr.log")),
    )
}

fn read_capture_file(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::detect_error_code;

    #[test]
    fn detect_error_code_does_not_mark_approval_never_as_manual_confirm() {
        let out = "approval: never\nsandbox: danger-full-access\ncompleted";
        assert_ne!(detect_error_code(out).as_deref(), Some("manual_confirm"));
    }

    #[test]
    fn detect_error_code_marks_interactive_confirm_prompts() {
        let out = "Permission denied: approval required. Press enter to continue.";
        assert_eq!(detect_error_code(out).as_deref(), Some("manual_confirm"));
    }

    #[test]
    fn detect_error_code_marks_invalid_args_separately() {
        let out = "error: the argument '--uncommitted' cannot be used with '[PROMPT]'";
        assert_eq!(detect_error_code(out).as_deref(), Some("invalid_args"));
    }
}
