use std::{
    collections::hash_map::DefaultHasher,
    fs,
    hash::{Hash, Hasher},
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use chrono::Local;
use serde_json::Value;

use crate::{
    consensus::{
        audit::{AuditLogger, AuditRecord},
        evaluate::{
            ExecDecision, ReviewDecision, claude_exec_decision, codex_review_decision, commit_todo,
            latest_commit_context, self_review, validate_done_when,
        },
        external::{
            ExecutorRun, build_clarify_questions_prompt, build_claude_prompt,
            build_codex_optimize_prompt, build_consensus_builder_prompt,
            build_followup_clarify_questions_prompt, build_todo_planner_prompt, preflight,
            run_claude, run_codex_execute, summarize_output,
        },
        model::{ConsensusDoc, TodoItem, build_from_interview, consensus_file_path, load, save},
    },
    types::{AuditEventKind, ConsensusTrigger, ExecutorOutcome, GeQuestionStep, Mode},
};

const PERIODIC_SCAN_INTERVAL: Duration = Duration::from_secs(30 * 60);
const FILE_SCAN_INTERVAL: Duration = Duration::from_secs(5);
const IDLE_TICK_INTERVAL: Duration = Duration::from_secs(2);
const MAX_CLARIFY_ROUNDS: usize = 4;
const MAX_CLARIFY_QUESTIONS_PER_BATCH: usize = 8;
const EXECUTOR_OUTPUT_PREVIEW_CHARS: usize = 2800;
const EXECUTOR_PREVIEW_MAX_LINES: usize = 40;

#[derive(Debug, Clone)]
struct InterviewState {
    step: GeQuestionStep,
    purpose: String,
    rules: String,
    scope: String,
    clarify_questions: Vec<ClarifyQuestion>,
    clarify_answers: Vec<String>,
    clarify_index: usize,
    clarify_round: usize,
}

#[derive(Debug, Clone)]
struct ClarifyQuestion {
    question: String,
    options: [String; 3],
}

#[derive(Debug, Clone)]
struct GeneratedConsensus {
    purpose_lines: Vec<String>,
    rules_lines: Vec<String>,
    scope: String,
    todos: Vec<TodoItem>,
}

#[derive(Debug, Clone)]
struct PromptSnapshot {
    todo_id: String,
    stage: String,
    prompt: String,
}

#[derive(Debug, Clone)]
struct ResultSnapshot {
    todo_id: String,
    stage: String,
    output: String,
}

#[derive(Debug, Clone)]
pub struct GeRuntime {
    mode: Mode,
    cwd: PathBuf,
    consensus_path: PathBuf,
    logger: AuditLogger,
    interview: Option<InterviewState>,
    last_hash: Option<u64>,
    next_periodic_scan: Instant,
    next_file_scan: Instant,
    next_action: Instant,
    pending_trigger: Option<ConsensusTrigger>,
    cancel_flag: Arc<AtomicBool>,
    preflight_done: bool,
    last_prompt: Option<PromptSnapshot>,
    last_result: Option<ResultSnapshot>,
}

impl GeRuntime {
    pub fn enter(
        cwd: PathBuf,
        initial_payload: &str,
        cancel_flag: Arc<AtomicBool>,
    ) -> Result<(Self, Vec<String>)> {
        let consensus_path = consensus_file_path(&cwd);
        let logger = AuditLogger::new(&consensus_path);
        let now = Instant::now();
        let mut lines = Vec::new();
        let mut runtime = Self {
            mode: Mode::GeInterview,
            cwd,
            consensus_path,
            logger,
            interview: None,
            last_hash: None,
            next_periodic_scan: now + PERIODIC_SCAN_INTERVAL,
            next_file_scan: now + FILE_SCAN_INTERVAL,
            next_action: now,
            pending_trigger: Some(ConsensusTrigger::Manual),
            cancel_flag,
            preflight_done: false,
            last_prompt: None,
            last_result: None,
        };

        runtime.log(AuditRecord {
            mode: runtime.mode,
            event: AuditEventKind::GeEntered,
            todo_id: None,
            trigger: Some(ConsensusTrigger::Manual),
            executor: Some("goldbot"),
            command: None,
            exit_code: None,
            status: ExecutorOutcome::Success,
            summary: Some("GE mode entered."),
            error_code: None,
        });

        if runtime.consensus_path.exists() {
            let doc = load(&runtime.consensus_path)?;
            runtime.last_hash = hash_file(&runtime.consensus_path).ok();
            runtime.mode = if doc.all_done() {
                Mode::GeIdle
            } else {
                Mode::GeRun
            };
            runtime.pending_trigger = Some(ConsensusTrigger::Manual);

            runtime.log(AuditRecord {
                mode: runtime.mode,
                event: AuditEventKind::ConsensusLoaded,
                todo_id: None,
                trigger: Some(ConsensusTrigger::Manual),
                executor: Some("goldbot"),
                command: None,
                exit_code: None,
                status: ExecutorOutcome::Success,
                summary: Some("Existing CONSENSUS.md loaded."),
                error_code: None,
            });
            lines.push("  GE mode enabled (existing CONSENSUS.md loaded).".to_string());
            lines.push(format!(
                "  Audit log: {}",
                runtime.logger.path().to_string_lossy()
            ));
        } else {
            let payload = initial_payload.trim();
            let mut interview = InterviewState {
                step: GeQuestionStep::Purpose,
                purpose: String::new(),
                rules: String::new(),
                scope: String::new(),
                clarify_questions: Vec::new(),
                clarify_answers: Vec::new(),
                clarify_index: 0,
                clarify_round: 0,
            };
            if !payload.is_empty() {
                interview.purpose = payload.to_string();
                interview.step = GeQuestionStep::Rules;
                runtime.log(AuditRecord {
                    mode: runtime.mode,
                    event: AuditEventKind::GeInput,
                    todo_id: None,
                    trigger: Some(ConsensusTrigger::Manual),
                    executor: Some("goldbot"),
                    command: None,
                    exit_code: None,
                    status: ExecutorOutcome::Success,
                    summary: Some(&format!(
                        "Initial GE payload: {}",
                        truncate_text(payload, 220)
                    )),
                    error_code: None,
                });
            }
            runtime.interview = Some(interview);
            runtime.mode = Mode::GeInterview;
            if let Some(prompt) = runtime.ask_next_question() {
                lines.push(prompt);
            }
        }

        Ok((runtime, lines))
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    pub fn interview_needs_long_wait(&self) -> bool {
        let Some(interview) = self.interview.as_ref() else {
            return false;
        };
        match interview.step {
            GeQuestionStep::Scope => true,
            GeQuestionStep::Clarify => {
                interview.clarify_index + 1 >= interview.clarify_questions.len()
            }
            _ => false,
        }
    }

    fn ensure_preflight<F>(&mut self, emit: &mut F)
    where
        F: FnMut(String),
    {
        if self.preflight_done || self.cancelled() {
            return;
        }
        emit_line(emit, "  GE: running executor preflight in background...");
        for p in preflight(&self.cwd, &self.cancel_flag) {
            self.log_executor_run(AuditEventKind::Preflight, None, p);
        }
        self.preflight_done = true;
        emit_line(emit, "  GE: executor preflight completed.");
    }

    pub fn handle_interview_reply(&mut self, text: &str) -> Result<(bool, Vec<String>)> {
        let mut lines = Vec::new();
        if self.mode != Mode::GeInterview {
            return Ok((false, lines));
        }
        let Some(step) = self.interview.as_ref().map(|i| i.step) else {
            return Ok((false, lines));
        };
        let answer = text.trim();
        if answer.is_empty() {
            lines.push("  GE: answer cannot be empty.".to_string());
            return Ok((true, lines));
        }

        match step {
            GeQuestionStep::Purpose => {
                if let Some(interview) = self.interview.as_mut() {
                    interview.purpose = answer.to_string();
                    interview.step = GeQuestionStep::Rules;
                }
                self.log(AuditRecord {
                    mode: self.mode,
                    event: AuditEventKind::GeQuestionAnswered,
                    todo_id: None,
                    trigger: Some(ConsensusTrigger::Manual),
                    executor: Some("goldbot"),
                    command: None,
                    exit_code: None,
                    status: ExecutorOutcome::Success,
                    summary: Some(&format!("Purpose: {}", truncate_text(answer, 220))),
                    error_code: None,
                });
                if let Some(prompt) = self.ask_next_question() {
                    lines.push(prompt);
                }
            }
            GeQuestionStep::Rules => {
                if let Some(interview) = self.interview.as_mut() {
                    interview.rules = answer.to_string();
                    interview.step = GeQuestionStep::Scope;
                }
                self.log(AuditRecord {
                    mode: self.mode,
                    event: AuditEventKind::GeQuestionAnswered,
                    todo_id: None,
                    trigger: Some(ConsensusTrigger::Manual),
                    executor: Some("goldbot"),
                    command: None,
                    exit_code: None,
                    status: ExecutorOutcome::Success,
                    summary: Some(&format!("Rules: {}", truncate_text(answer, 220))),
                    error_code: None,
                });
                if let Some(prompt) = self.ask_next_question() {
                    lines.push(prompt);
                }
            }
            GeQuestionStep::Scope => {
                let (purpose, rules, scope) = {
                    let Some(interview) = self.interview.as_mut() else {
                        return Ok((false, lines));
                    };
                    interview.scope = answer.to_string();
                    (
                        interview.purpose.clone(),
                        interview.rules.clone(),
                        interview.scope.clone(),
                    )
                };
                self.log(AuditRecord {
                    mode: self.mode,
                    event: AuditEventKind::GeQuestionAnswered,
                    todo_id: None,
                    trigger: Some(ConsensusTrigger::Manual),
                    executor: Some("goldbot"),
                    command: None,
                    exit_code: None,
                    status: ExecutorOutcome::Success,
                    summary: Some(&format!("Scope: {}", truncate_text(answer, 220))),
                    error_code: None,
                });
                self.ensure_preflight(&mut |line| lines.push(line));
                if self.cancelled() {
                    lines.push("  GE: planning cancelled by hard exit.".to_string());
                    return Ok((true, lines));
                }
                lines.push(
                    "  GE: analyzing Purpose/Rules and generating clarify options...".to_string(),
                );
                let (questions, clarify_note) =
                    self.generate_clarify_questions(&purpose, &rules, &scope, &mut lines);
                if self.cancelled() {
                    lines.push("  GE: planning cancelled by hard exit.".to_string());
                    return Ok((true, lines));
                }
                lines.push(format!("  GE: {clarify_note}"));
                if let Some(interview) = self.interview.as_mut() {
                    interview.clarify_questions = questions;
                    interview.clarify_answers.clear();
                    interview.clarify_index = 0;
                    interview.clarify_round = 1;
                    if interview.clarify_questions.is_empty() {
                        interview.step = GeQuestionStep::Scope;
                    } else {
                        interview.step = GeQuestionStep::Clarify;
                    }
                }

                if let Some(question_lines) = self.ask_current_clarify_question() {
                    lines.extend(question_lines);
                } else {
                    lines.push(
                        "  GE: no extra clarification needed, generating final consensus..."
                            .to_string(),
                    );
                    lines.extend(self.finish_interview_and_generate_consensus(
                        &purpose,
                        &rules,
                        &scope,
                        &[],
                    )?);
                }
            }
            GeQuestionStep::Clarify => {
                let (question, selected_kind, selected, done) = {
                    let Some(interview) = self.interview.as_mut() else {
                        return Ok((false, lines));
                    };
                    let Some(q) = interview
                        .clarify_questions
                        .get(interview.clarify_index)
                        .cloned()
                    else {
                        interview.step = GeQuestionStep::Scope;
                        return Ok((true, lines));
                    };
                    let (selected_kind, selected) =
                        if let Some(choice) = parse_option_choice(answer, 3) {
                            (format!("option {}", choice + 1), q.options[choice].clone())
                        } else {
                            ("custom".to_string(), answer.to_string())
                        };
                    interview.clarify_answers.push(format!(
                        "Q{} {} => {}",
                        interview.clarify_index + 1,
                        q.question,
                        selected
                    ));
                    interview.clarify_index += 1;
                    let done = interview.clarify_index >= interview.clarify_questions.len();
                    (q.question, selected_kind, selected, done)
                };
                self.log(AuditRecord {
                    mode: self.mode,
                    event: AuditEventKind::GeQuestionAnswered,
                    todo_id: None,
                    trigger: Some(ConsensusTrigger::Manual),
                    executor: Some("goldbot"),
                    command: None,
                    exit_code: None,
                    status: ExecutorOutcome::Success,
                    summary: Some(&format!(
                        "Clarify answer: Q `{}` -> {} `{}`",
                        truncate_text(&question, 80),
                        selected_kind,
                        truncate_text(&selected, 80)
                    )),
                    error_code: None,
                });

                if !done {
                    if let Some(question_lines) = self.ask_current_clarify_question() {
                        lines.extend(question_lines);
                    }
                    return Ok((true, lines));
                }

                let (purpose, rules, scope, clarify_answers, clarify_round) = {
                    let Some(interview) = self.interview.as_ref() else {
                        return Ok((false, lines));
                    };
                    (
                        interview.purpose.clone(),
                        interview.rules.clone(),
                        interview.scope.clone(),
                        interview.clarify_answers.clone(),
                        interview.clarify_round,
                    )
                };
                if clarify_round < MAX_CLARIFY_ROUNDS {
                    lines.push(
                        "  GE: evaluating whether additional clarification is needed..."
                            .to_string(),
                    );
                    let (followups, follow_note) = self.generate_followup_clarify_questions(
                        &purpose,
                        &rules,
                        &scope,
                        &clarify_answers,
                        clarify_round + 1,
                        &mut lines,
                    );
                    lines.push(format!("  GE: {follow_note}"));
                    if !followups.is_empty() {
                        if let Some(interview) = self.interview.as_mut() {
                            interview.clarify_questions = followups;
                            interview.clarify_index = 0;
                            interview.clarify_round = clarify_round + 1;
                            interview.step = GeQuestionStep::Clarify;
                        }
                        if let Some(question_lines) = self.ask_current_clarify_question() {
                            lines.extend(question_lines);
                        }
                        return Ok((true, lines));
                    }
                }
                lines.push(
                    "  GE: clarification complete. Generating optimized CONSENSUS content..."
                        .to_string(),
                );
                lines.extend(self.finish_interview_and_generate_consensus(
                    &purpose,
                    &rules,
                    &scope,
                    &clarify_answers,
                )?);
            }
        }
        Ok((true, lines))
    }

    pub fn tick_with_emit<F>(&mut self, mut emit: F) -> Result<()>
    where
        F: FnMut(String),
    {
        if self.cancelled() {
            return Ok(());
        }
        if self.mode == Mode::GeInterview || self.mode == Mode::Normal {
            return Ok(());
        }
        let now = Instant::now();
        if now < self.next_action {
            return Ok(());
        }

        if let Some(trigger) = self.pending_trigger.take() {
            self.run_once(trigger, &mut emit)?;
            return Ok(());
        }

        if now >= self.next_periodic_scan {
            self.next_periodic_scan = now + PERIODIC_SCAN_INTERVAL;
            self.run_once(ConsensusTrigger::Periodic, &mut emit)?;
            return Ok(());
        }

        if now >= self.next_file_scan {
            self.next_file_scan = now + FILE_SCAN_INTERVAL;
            let current_hash = hash_file(&self.consensus_path).ok();
            if current_hash.is_some() && current_hash != self.last_hash {
                self.last_hash = current_hash;
                self.run_once(ConsensusTrigger::FileChanged, &mut emit)?;
                return Ok(());
            }
        }

        if self.mode == Mode::GeRun {
            self.run_once(ConsensusTrigger::Manual, &mut emit)?;
        }
        Ok(())
    }

    pub fn exit(&mut self) -> Vec<String> {
        self.log(AuditRecord {
            mode: self.mode,
            event: AuditEventKind::GeExited,
            todo_id: None,
            trigger: Some(ConsensusTrigger::Manual),
            executor: Some("goldbot"),
            command: None,
            exit_code: None,
            status: ExecutorOutcome::Success,
            summary: Some("GE mode exited."),
            error_code: None,
        });
        vec!["  GE mode disabled.".to_string()]
    }

    pub fn expand_last_prompt(&self) -> Vec<String> {
        let Some(snapshot) = self.last_prompt.as_ref() else {
            return vec!["  GE: no cached prompt to expand yet.".to_string()];
        };
        let mut lines = vec![
            "  ================================================".to_string(),
            format!(
                "  GE PROMPT EXPANDED [{}] {}",
                snapshot.todo_id, snapshot.stage
            ),
            "  ================================================".to_string(),
        ];
        for line in preview_block_lines(&snapshot.prompt, 14_000, 220) {
            lines.push(format!("    {line}"));
        }
        lines
    }

    pub fn expand_last_result(&self) -> Vec<String> {
        let Some(snapshot) = self.last_result.as_ref() else {
            return vec!["  GE: no cached result to expand yet.".to_string()];
        };
        let mut lines = vec![
            "  ================================================".to_string(),
            format!(
                "  GE RESULT EXPANDED [{}] {}",
                snapshot.todo_id, snapshot.stage
            ),
            "  ================================================".to_string(),
        ];
        for line in preview_block_lines(&snapshot.output, 14_000, 260) {
            lines.push(format!("    {line}"));
        }
        lines
    }

    pub fn replan_todos(&mut self) -> Result<(bool, Vec<String>)> {
        let mut lines = Vec::new();
        let mut doc = load(&self.consensus_path)?;
        let purpose = join_section_lines(&doc.purpose_lines);
        let rules = join_section_lines(&doc.rules_lines);
        let scope = extract_scope(&doc.purpose_lines);
        let (mut generated, note) = self.generate_todos(&purpose, &rules, &scope, &mut lines);
        if generated.is_empty() {
            lines.push("  GE replan failed; kept existing todos.".to_string());
            return Ok((false, lines));
        }

        let checked: Vec<TodoItem> = doc.todos.iter().filter(|t| t.checked).cloned().collect();
        let base = checked.len();
        for (i, todo) in generated.iter_mut().enumerate() {
            todo.id = format!("T{:03}", base + i + 1);
            todo.checked = false;
        }

        let mut merged = checked;
        merged.extend(generated);
        doc.todos = merged;
        doc.append_status(format!("- {} Todos replanned to finer steps.", now_hms()));
        doc.append_journal(format!("- {} todo replan: {}", now_hms(), note));
        save(&self.consensus_path, &doc)?;
        self.last_hash = hash_file(&self.consensus_path).ok();
        self.pending_trigger = Some(ConsensusTrigger::Manual);
        self.next_action = Instant::now();
        self.mode = if doc.all_done() {
            Mode::GeIdle
        } else {
            Mode::GeRun
        };
        lines.push("  GE replanned todo list (finer steps).".to_string());
        Ok((true, lines))
    }

    fn ask_current_clarify_question(&self) -> Option<Vec<String>> {
        let interview = self.interview.as_ref()?;
        if interview.step != GeQuestionStep::Clarify {
            return None;
        }
        let idx = interview.clarify_index;
        let total = interview.clarify_questions.len();
        let question = interview.clarify_questions.get(idx)?;
        let mut lines = vec![
            format!("  GE Clarify {}/{}: {}", idx + 1, total, question.question),
            format!("    1) {}", question.options[0]),
            format!("    2) {}", question.options[1]),
            format!("    3) {}", question.options[2]),
            "  Reply with 1/2/3, or type your own answer.".to_string(),
        ];
        self.log(AuditRecord {
            mode: self.mode,
            event: AuditEventKind::GeQuestionAsked,
            todo_id: None,
            trigger: Some(ConsensusTrigger::Manual),
            executor: Some("goldbot"),
            command: None,
            exit_code: None,
            status: ExecutorOutcome::Success,
            summary: Some(&format!(
                "Clarify question {}/{}: {}",
                idx + 1,
                total,
                truncate_text(&question.question, 180)
            )),
            error_code: None,
        });
        if idx + 1 == total {
            lines.push("  GE: this is the last clarification question.".to_string());
        }
        Some(lines)
    }

    fn finish_interview_and_generate_consensus(
        &mut self,
        purpose: &str,
        rules: &str,
        scope: &str,
        clarify_answers: &[String],
    ) -> Result<Vec<String>> {
        let mut lines = Vec::new();
        lines.push(
            "  GE: requesting LLM to generate final CONSENSUS Purpose/Rules/Todo...".to_string(),
        );
        let (mut doc, note) =
            self.generate_consensus_doc(purpose, rules, scope, clarify_answers, &mut lines);
        if self.cancelled() {
            lines.push("  GE: consensus generation cancelled by hard exit.".to_string());
            return Ok(lines);
        }
        doc.append_status(format!("- {} GE initialized.", now_hms()));
        doc.append_journal(format!(
            "- {} consensus planner: {}",
            now_hms(),
            truncate_text(&note, 220)
        ));
        save(&self.consensus_path, &doc)?;
        self.last_hash = hash_file(&self.consensus_path).ok();
        self.mode = if doc.all_done() {
            Mode::GeIdle
        } else {
            Mode::GeRun
        };
        self.interview = None;
        self.pending_trigger = Some(ConsensusTrigger::Manual);
        self.next_action = Instant::now();

        self.log(AuditRecord {
            mode: self.mode,
            event: AuditEventKind::ConsensusGenerated,
            todo_id: None,
            trigger: Some(ConsensusTrigger::Manual),
            executor: Some("goldbot"),
            command: None,
            exit_code: None,
            status: ExecutorOutcome::Success,
            summary: Some("Generated CONSENSUS.md from LLM planned interview flow."),
            error_code: None,
        });

        lines.push(format!("  GE: {note}"));
        lines.push("  GE interview complete; generated CONSENSUS.md".to_string());
        lines.push(format!(
            "  Audit log: {}",
            self.logger.path().to_string_lossy()
        ));
        Ok(lines)
    }

    fn ask_next_question(&self) -> Option<String> {
        let Some(interview) = &self.interview else {
            return None;
        };
        let prompt = match interview.step {
            GeQuestionStep::Purpose => "  GE Q1/3: What is the purpose/goal?",
            GeQuestionStep::Rules => "  GE Q2/3: What rules must always be followed?",
            GeQuestionStep::Scope => "  GE Q3/3: What are the scope boundaries?",
            GeQuestionStep::Clarify => return None,
        };
        self.log(AuditRecord {
            mode: self.mode,
            event: AuditEventKind::GeQuestionAsked,
            todo_id: None,
            trigger: Some(ConsensusTrigger::Manual),
            executor: Some("goldbot"),
            command: None,
            exit_code: None,
            status: ExecutorOutcome::Success,
            summary: Some(prompt),
            error_code: None,
        });
        Some(prompt.to_string())
    }

    fn run_once<F>(&mut self, trigger: ConsensusTrigger, emit: &mut F) -> Result<()>
    where
        F: FnMut(String),
    {
        if self.cancelled() {
            emit_line(emit, "  GE hard exit requested; aborting current step.");
            return Ok(());
        }
        self.ensure_preflight(emit);
        if self.cancelled() {
            emit_line(emit, "  GE hard exit requested; aborting current step.");
            return Ok(());
        }
        let mut doc = match load(&self.consensus_path) {
            Ok(d) => d,
            Err(e) => {
                self.mode = Mode::GeInterview;
                self.interview = Some(InterviewState {
                    step: GeQuestionStep::Purpose,
                    purpose: String::new(),
                    rules: String::new(),
                    scope: String::new(),
                    clarify_questions: Vec::new(),
                    clarify_answers: Vec::new(),
                    clarify_index: 0,
                    clarify_round: 0,
                });
                self.log(AuditRecord {
                    mode: self.mode,
                    event: AuditEventKind::Error,
                    todo_id: None,
                    trigger: Some(trigger),
                    executor: Some("goldbot"),
                    command: None,
                    exit_code: None,
                    status: ExecutorOutcome::Failed,
                    summary: Some("CONSENSUS.md missing; switching to interview."),
                    error_code: Some("missing_consensus"),
                });
                emit_line(emit, format!("  GE: {}", e.to_string().trim()));
                emit_line(emit, "  GE Q1/3: What is the purpose/goal?");
                return Ok(());
            }
        };

        self.log(AuditRecord {
            mode: self.mode,
            event: AuditEventKind::Trigger,
            todo_id: None,
            trigger: Some(trigger),
            executor: Some("goldbot"),
            command: None,
            exit_code: None,
            status: ExecutorOutcome::Success,
            summary: Some("GE execution trigger."),
            error_code: None,
        });

        if doc.todos.is_empty() || doc.all_done() {
            self.mode = Mode::GeIdle;
            doc.append_status(format!("- {} All todos completed. Waiting.", now_hms()));
            save(&self.consensus_path, &doc)?;
            self.last_hash = hash_file(&self.consensus_path).ok();
            self.next_action = Instant::now() + IDLE_TICK_INTERVAL;
            emit_line(emit, "  GE idle: all todos completed.");
            return Ok(());
        }

        self.mode = Mode::GeRun;
        let Some(todo_idx) = doc.first_open_todo_index() else {
            self.mode = Mode::GeIdle;
            self.next_action = Instant::now() + IDLE_TICK_INTERVAL;
            return Ok(());
        };
        let todo = doc.todos[todo_idx].clone();
        emit_line(emit, format!("  GE running {} {}", todo.id, todo.text));
        self.log(AuditRecord {
            mode: self.mode,
            event: AuditEventKind::TodoSelected,
            todo_id: Some(&todo.id),
            trigger: Some(trigger),
            executor: Some("goldbot"),
            command: None,
            exit_code: None,
            status: ExecutorOutcome::Success,
            summary: Some(&todo.text),
            error_code: None,
        });
        let git_context = latest_commit_context();

        let claude_prompt = build_claude_prompt(
            &doc.purpose_lines,
            &doc.rules_lines,
            &todo.id,
            &todo.text,
            &todo.done_when,
            git_context.as_deref(),
        );
        let codex_opt_prompt = build_codex_optimize_prompt(
            &doc.purpose_lines,
            &doc.rules_lines,
            &todo.id,
            &todo.text,
            &todo.done_when,
            git_context.as_deref(),
        );

        emit_stage_header(emit, &todo.id, "Claude execute");
        self.cache_prompt(&todo.id, "Claude execute", &claude_prompt);
        emit_executor_prompt(emit, &todo.id, "Claude execute", &claude_prompt);
        let mut execution = run_claude(&self.cwd, &claude_prompt, &self.cancel_flag);
        self.cache_result(&todo.id, "Claude execute", &execution.output);
        emit_executor_result(emit, &todo.id, "Claude execute", &execution);
        if self.cancelled() || execution.error_code.as_deref() == Some("cancelled") {
            emit_line(
                emit,
                format!("  {} hard-exit requested; execution cancelled.", todo.id),
            );
            return Ok(());
        }
        self.log_executor_run(
            AuditEventKind::ClaudeExec,
            Some(&todo.id),
            execution.clone(),
        );

        let mut claude_fallback_reason: Option<String> = None;
        let mut fallback_codex_opt: Option<ExecutorRun> = None;
        let claude_exec_status = claude_exec_decision(&execution.output, execution.exit_code);
        if execution.outcome == ExecutorOutcome::BlockedConfirm {
            claude_fallback_reason = Some("blocked_confirm".to_string());
        } else if execution.error_code.as_deref() == Some("rate_limit") {
            claude_fallback_reason = Some("rate_limit".to_string());
        } else if !execution.ok() {
            claude_fallback_reason = Some("execution_failed".to_string());
        } else if let ExecDecision::Fail(reason) = claude_exec_status {
            claude_fallback_reason = Some(reason);
        }

        if let Some(reason) = claude_fallback_reason.as_deref() {
            emit_line(
                emit,
                format!(
                    "  {} Claude step incomplete ({}); switching to Codex optimize+review.",
                    todo.id, reason
                ),
            );
            self.cache_prompt(
                &todo.id,
                "Codex optimize+review (fallback execute)",
                &codex_opt_prompt,
            );
            emit_executor_prompt(
                emit,
                &todo.id,
                "Codex optimize+review (fallback execute)",
                &codex_opt_prompt,
            );
            execution = run_codex_execute(&self.cwd, &codex_opt_prompt, &self.cancel_flag);
            self.cache_result(
                &todo.id,
                "Codex optimize+review (fallback execute)",
                &execution.output,
            );
            emit_executor_result(
                emit,
                &todo.id,
                "Codex optimize+review (fallback execute)",
                &execution,
            );
            if self.cancelled() || execution.error_code.as_deref() == Some("cancelled") {
                emit_line(
                    emit,
                    format!("  {} hard-exit requested; execution cancelled.", todo.id),
                );
                return Ok(());
            }
            self.log_executor_run(AuditEventKind::CodexExec, Some(&todo.id), execution.clone());
            fallback_codex_opt = Some(execution.clone());
        }

        if execution.outcome == ExecutorOutcome::BlockedConfirm {
            defer_todo(
                &mut doc,
                &todo.id,
                format!("- {} {} blocked by manual confirm.", now_hms(), todo.id),
            );
            emit_line(
                emit,
                format!("  {} deferred: blocked by manual confirm.", todo.id),
            );
            self.log(AuditRecord {
                mode: self.mode,
                event: AuditEventKind::TodoDeferred,
                todo_id: Some(&todo.id),
                trigger: Some(trigger),
                executor: Some("goldbot"),
                command: None,
                exit_code: Some(execution.exit_code),
                status: ExecutorOutcome::BlockedConfirm,
                summary: Some("Execution blocked by manual confirm."),
                error_code: Some("manual_confirm"),
            });
            save(&self.consensus_path, &doc)?;
            self.last_hash = hash_file(&self.consensus_path).ok();
            self.next_action = Instant::now() + PERIODIC_SCAN_INTERVAL;
            return Ok(());
        }

        if !execution.ok() {
            defer_todo(
                &mut doc,
                &todo.id,
                format!(
                    "- {} {} execution failed: {}",
                    now_hms(),
                    todo.id,
                    summarize_output(&execution.output, 4)
                ),
            );
            emit_line(emit, format!("  {} deferred: execution failed.", todo.id));
            save(&self.consensus_path, &doc)?;
            self.last_hash = hash_file(&self.consensus_path).ok();
            self.next_action = Instant::now() + IDLE_TICK_INTERVAL;
            return Ok(());
        }

        emit_stage_header(emit, &todo.id, "Codex optimize+review");
        let codex_opt = if let Some(run) = fallback_codex_opt.take() {
            emit_line(
                emit,
                format!(
                    "  {} reusing fallback Codex optimize+review result.",
                    todo.id
                ),
            );
            run
        } else {
            self.cache_prompt(&todo.id, "Codex optimize+review", &codex_opt_prompt);
            emit_executor_prompt(emit, &todo.id, "Codex optimize+review", &codex_opt_prompt);
            let run = run_codex_execute(&self.cwd, &codex_opt_prompt, &self.cancel_flag);
            self.cache_result(&todo.id, "Codex optimize+review", &run.output);
            emit_executor_result(emit, &todo.id, "Codex optimize+review", &run);
            run
        };
        if self.cancelled() || codex_opt.error_code.as_deref() == Some("cancelled") {
            emit_line(
                emit,
                format!(
                    "  {} hard-exit requested; optimize+review cancelled.",
                    todo.id
                ),
            );
            return Ok(());
        }
        self.log_executor_run(AuditEventKind::CodexExec, Some(&todo.id), codex_opt.clone());
        if codex_opt.outcome == ExecutorOutcome::BlockedConfirm {
            defer_todo(
                &mut doc,
                &todo.id,
                format!(
                    "- {} {} blocked by codex optimize+review confirm.",
                    now_hms(),
                    todo.id
                ),
            );
            emit_line(
                emit,
                format!(
                    "  {} deferred: Codex optimize+review blocked by confirm.",
                    todo.id
                ),
            );
            save(&self.consensus_path, &doc)?;
            self.last_hash = hash_file(&self.consensus_path).ok();
            self.next_action = Instant::now() + PERIODIC_SCAN_INTERVAL;
            return Ok(());
        }
        if !codex_opt.ok() {
            defer_todo(
                &mut doc,
                &todo.id,
                format!("- {} {} codex optimize+review failed.", now_hms(), todo.id),
            );
            emit_line(
                emit,
                format!("  {} deferred: Codex optimize+review failed.", todo.id),
            );
            save(&self.consensus_path, &doc)?;
            self.last_hash = hash_file(&self.consensus_path).ok();
            self.next_action = Instant::now() + IDLE_TICK_INTERVAL;
            return Ok(());
        }

        let review_decision = codex_review_decision(&codex_opt.output, codex_opt.exit_code);
        if let ReviewDecision::Blocked(reason) = review_decision {
            defer_todo(
                &mut doc,
                &todo.id,
                format!(
                    "- {} {} codex optimize+review reported blockers: {}",
                    now_hms(),
                    todo.id,
                    reason
                ),
            );
            self.log(AuditRecord {
                mode: self.mode,
                event: AuditEventKind::TodoDeferred,
                todo_id: Some(&todo.id),
                trigger: Some(trigger),
                executor: Some("goldbot"),
                command: None,
                exit_code: Some(codex_opt.exit_code),
                status: ExecutorOutcome::Failed,
                summary: Some(&format!(
                    "Codex optimize+review reported blockers: {}",
                    reason
                )),
                error_code: Some("review_blocking"),
            });
            emit_line(
                emit,
                format!(
                    "  {} deferred: Codex optimize+review reported blockers ({}).",
                    todo.id, reason
                ),
            );
            save(&self.consensus_path, &doc)?;
            self.last_hash = hash_file(&self.consensus_path).ok();
            self.next_action = Instant::now() + IDLE_TICK_INTERVAL;
            return Ok(());
        }

        emit_line(emit, format!("  {} done_when validation started.", todo.id));
        let validation = validate_done_when(&todo.done_when, &self.cwd);
        self.log(AuditRecord {
            mode: self.mode,
            event: AuditEventKind::Validation,
            todo_id: Some(&todo.id),
            trigger: Some(trigger),
            executor: Some("goldbot"),
            command: None,
            exit_code: Some(validation.exit_code),
            status: validation.outcome,
            summary: Some(&validation.summary),
            error_code: None,
        });
        if validation.outcome != ExecutorOutcome::Success {
            defer_todo(
                &mut doc,
                &todo.id,
                format!(
                    "- {} {} validation failed: {}",
                    now_hms(),
                    todo.id,
                    validation.summary
                ),
            );
            emit_line(
                emit,
                format!("  {} deferred: done_when validation failed.", todo.id),
            );
            save(&self.consensus_path, &doc)?;
            self.last_hash = hash_file(&self.consensus_path).ok();
            self.next_action = Instant::now() + IDLE_TICK_INTERVAL;
            return Ok(());
        }

        emit_line(emit, format!("  {} GoldBot self-review started.", todo.id));
        let self_review_report = self_review(&self.cwd);
        self.log(AuditRecord {
            mode: self.mode,
            event: AuditEventKind::SelfReview,
            todo_id: Some(&todo.id),
            trigger: Some(trigger),
            executor: Some("goldbot"),
            command: Some("git diff --check && git status --short && git diff --stat"),
            exit_code: Some(self_review_report.exit_code),
            status: self_review_report.outcome,
            summary: Some(&self_review_report.summary),
            error_code: None,
        });
        if self_review_report.outcome != ExecutorOutcome::Success {
            defer_todo(
                &mut doc,
                &todo.id,
                format!(
                    "- {} {} self review failed: {}",
                    now_hms(),
                    todo.id,
                    self_review_report.summary
                ),
            );
            emit_line(
                emit,
                format!("  {} deferred: GoldBot self-review failed.", todo.id),
            );
            save(&self.consensus_path, &doc)?;
            self.last_hash = hash_file(&self.consensus_path).ok();
            self.next_action = Instant::now() + IDLE_TICK_INTERVAL;
            return Ok(());
        }

        emit_line(emit, format!("  {} Git commit started.", todo.id));
        let commit = commit_todo(&todo.id, &todo.text);
        self.log(AuditRecord {
            mode: self.mode,
            event: AuditEventKind::GitCommit,
            todo_id: Some(&todo.id),
            trigger: Some(trigger),
            executor: Some("goldbot"),
            command: Some("git add -A && git commit --allow-empty -m '<GE todo>'"),
            exit_code: Some(commit.exit_code),
            status: commit.outcome,
            summary: Some(&commit.summary),
            error_code: None,
        });
        if commit.outcome != ExecutorOutcome::Success {
            defer_todo(
                &mut doc,
                &todo.id,
                format!(
                    "- {} {} git commit failed: {}",
                    now_hms(),
                    todo.id,
                    commit.summary
                ),
            );
            emit_line(emit, format!("  {} deferred: git commit failed.", todo.id));
            save(&self.consensus_path, &doc)?;
            self.last_hash = hash_file(&self.consensus_path).ok();
            self.next_action = Instant::now() + IDLE_TICK_INTERVAL;
            return Ok(());
        }

        let _ = doc.mark_checked(&todo.id);
        doc.append_status(format!("- {} {} checked.", now_hms(), todo.id));
        doc.append_journal(format!(
            "- {} {} done. exec: {} | codex(opt+review): {} | commit: {}",
            now_hms(),
            todo.id,
            summarize_output(&execution.output, 2),
            summarize_output(&codex_opt.output, 2),
            commit.summary
        ));
        self.log(AuditRecord {
            mode: self.mode,
            event: AuditEventKind::TodoChecked,
            todo_id: Some(&todo.id),
            trigger: Some(trigger),
            executor: Some("goldbot"),
            command: None,
            exit_code: Some(0),
            status: ExecutorOutcome::Success,
            summary: Some("Todo checked after validation and review."),
            error_code: None,
        });
        save(&self.consensus_path, &doc)?;
        self.last_hash = hash_file(&self.consensus_path).ok();
        self.pending_trigger = Some(ConsensusTrigger::TaskDone);
        self.next_action = Instant::now() + IDLE_TICK_INTERVAL;
        emit_line(emit, format!("  {} checked.", todo.id));
        Ok(())
    }

    fn generate_clarify_questions(
        &mut self,
        purpose: &str,
        rules: &str,
        scope: &str,
        lines: &mut Vec<String>,
    ) -> (Vec<ClarifyQuestion>, String) {
        if self.cancelled() {
            return (Vec::new(), "clarification cancelled".to_string());
        }
        let prompt = build_clarify_questions_prompt(purpose, rules, scope);
        self.cache_prompt("GE", "Claude clarify planner", &prompt);
        emit_executor_prompt(
            &mut |line| lines.push(line),
            "GE",
            "Claude clarify planner",
            &prompt,
        );
        let claude = run_claude(&self.cwd, &prompt, &self.cancel_flag);
        self.cache_result("GE", "Claude clarify planner", &claude.output);
        emit_executor_result(
            &mut |line| lines.push(line),
            "GE",
            "Claude clarify planner",
            &claude,
        );
        self.log_executor_run(AuditEventKind::ClarifyGenerated, None, claude.clone());
        if claude.error_code.as_deref() == Some("cancelled") {
            return (Vec::new(), "clarification cancelled".to_string());
        }
        if claude.ok()
            && let Some(questions) = parse_clarify_questions_json(&claude.output)
        {
            return (
                questions,
                "clarification questions generated by claude".to_string(),
            );
        }

        self.cache_prompt("GE", "Codex clarify planner (fallback)", &prompt);
        emit_executor_prompt(
            &mut |line| lines.push(line),
            "GE",
            "Codex clarify planner (fallback)",
            &prompt,
        );
        let codex = run_codex_execute(&self.cwd, &prompt, &self.cancel_flag);
        self.cache_result("GE", "Codex clarify planner (fallback)", &codex.output);
        emit_executor_result(
            &mut |line| lines.push(line),
            "GE",
            "Codex clarify planner (fallback)",
            &codex,
        );
        self.log_executor_run(AuditEventKind::ClarifyGenerated, None, codex.clone());
        if codex.error_code.as_deref() == Some("cancelled") {
            return (Vec::new(), "clarification cancelled".to_string());
        }
        if codex.ok()
            && let Some(questions) = parse_clarify_questions_json(&codex.output)
        {
            return (
                questions,
                "clarification questions generated by codex (fallback)".to_string(),
            );
        }

        (
            Vec::new(),
            "clarification planner returned invalid output; skip clarify stage".to_string(),
        )
    }

    fn generate_followup_clarify_questions(
        &mut self,
        purpose: &str,
        rules: &str,
        scope: &str,
        clarify_answers: &[String],
        round: usize,
        lines: &mut Vec<String>,
    ) -> (Vec<ClarifyQuestion>, String) {
        if self.cancelled() {
            return (Vec::new(), "follow-up clarification cancelled".to_string());
        }
        let prompt =
            build_followup_clarify_questions_prompt(purpose, rules, scope, clarify_answers, round);
        self.cache_prompt("GE", "Claude follow-up planner", &prompt);
        emit_executor_prompt(
            &mut |line| lines.push(line),
            "GE",
            "Claude follow-up planner",
            &prompt,
        );
        let claude = run_claude(&self.cwd, &prompt, &self.cancel_flag);
        self.cache_result("GE", "Claude follow-up planner", &claude.output);
        emit_executor_result(
            &mut |line| lines.push(line),
            "GE",
            "Claude follow-up planner",
            &claude,
        );
        self.log_executor_run(AuditEventKind::ClarifyGenerated, None, claude.clone());
        if claude.error_code.as_deref() == Some("cancelled") {
            return (Vec::new(), "follow-up clarification cancelled".to_string());
        }
        if claude.ok()
            && let Some(questions) = parse_clarify_questions_json(&claude.output)
        {
            if questions.is_empty() {
                return (
                    Vec::new(),
                    "clarification complete according to claude".to_string(),
                );
            }
            return (
                questions,
                "additional clarification questions generated by claude".to_string(),
            );
        }

        self.cache_prompt("GE", "Codex follow-up planner (fallback)", &prompt);
        emit_executor_prompt(
            &mut |line| lines.push(line),
            "GE",
            "Codex follow-up planner (fallback)",
            &prompt,
        );
        let codex = run_codex_execute(&self.cwd, &prompt, &self.cancel_flag);
        self.cache_result("GE", "Codex follow-up planner (fallback)", &codex.output);
        emit_executor_result(
            &mut |line| lines.push(line),
            "GE",
            "Codex follow-up planner (fallback)",
            &codex,
        );
        self.log_executor_run(AuditEventKind::ClarifyGenerated, None, codex.clone());
        if codex.error_code.as_deref() == Some("cancelled") {
            return (Vec::new(), "follow-up clarification cancelled".to_string());
        }
        if codex.ok()
            && let Some(questions) = parse_clarify_questions_json(&codex.output)
        {
            if questions.is_empty() {
                return (
                    Vec::new(),
                    "clarification complete according to codex".to_string(),
                );
            }
            return (
                questions,
                "additional clarification questions generated by codex (fallback)".to_string(),
            );
        }

        (
            Vec::new(),
            "follow-up clarification output invalid; continue with current data".to_string(),
        )
    }

    fn generate_consensus_doc(
        &mut self,
        purpose: &str,
        rules: &str,
        scope: &str,
        clarify_answers: &[String],
        lines: &mut Vec<String>,
    ) -> (ConsensusDoc, String) {
        if self.cancelled() {
            return (
                build_from_interview(purpose, rules, scope),
                "consensus generation cancelled, fallback template used".to_string(),
            );
        }

        let prompt = build_consensus_builder_prompt(purpose, rules, scope, clarify_answers);
        self.cache_prompt("GE", "Claude consensus planner", &prompt);
        emit_executor_prompt(
            &mut |line| lines.push(line),
            "GE",
            "Claude consensus planner",
            &prompt,
        );
        let claude = run_claude(&self.cwd, &prompt, &self.cancel_flag);
        self.cache_result("GE", "Claude consensus planner", &claude.output);
        emit_executor_result(
            &mut |line| lines.push(line),
            "GE",
            "Claude consensus planner",
            &claude,
        );
        self.log_executor_run(AuditEventKind::TodoPlanGenerated, None, claude.clone());
        if claude.error_code.as_deref() == Some("cancelled") {
            return (
                build_from_interview(purpose, rules, scope),
                "consensus generation cancelled, fallback template used".to_string(),
            );
        }
        if claude.ok()
            && let Some(generated) = parse_consensus_payload_json(&claude.output)
        {
            return (
                build_consensus_doc_from_generated(generated, purpose, rules, scope),
                "consensus generated by claude".to_string(),
            );
        }

        self.cache_prompt("GE", "Codex consensus planner (fallback)", &prompt);
        emit_executor_prompt(
            &mut |line| lines.push(line),
            "GE",
            "Codex consensus planner (fallback)",
            &prompt,
        );
        let codex = run_codex_execute(&self.cwd, &prompt, &self.cancel_flag);
        self.cache_result("GE", "Codex consensus planner (fallback)", &codex.output);
        emit_executor_result(
            &mut |line| lines.push(line),
            "GE",
            "Codex consensus planner (fallback)",
            &codex,
        );
        self.log_executor_run(AuditEventKind::TodoPlanGenerated, None, codex.clone());
        if codex.error_code.as_deref() == Some("cancelled") {
            return (
                build_from_interview(purpose, rules, scope),
                "consensus generation cancelled, fallback template used".to_string(),
            );
        }
        if codex.ok()
            && let Some(generated) = parse_consensus_payload_json(&codex.output)
        {
            return (
                build_consensus_doc_from_generated(generated, purpose, rules, scope),
                "consensus generated by codex (fallback)".to_string(),
            );
        }

        let mut doc = build_from_interview(purpose, rules, scope);
        let (generated_todos, planner_note) = self.generate_todos(purpose, rules, scope, lines);
        if !generated_todos.is_empty() {
            doc.todos = generated_todos;
        }
        (
            doc,
            format!("consensus fallback used; todo planner note: {planner_note}"),
        )
    }

    fn generate_todos(
        &mut self,
        purpose: &str,
        rules: &str,
        scope: &str,
        lines: &mut Vec<String>,
    ) -> (Vec<crate::consensus::model::TodoItem>, String) {
        let prompt = build_todo_planner_prompt(purpose, rules, scope);
        self.cache_prompt("GE", "Claude todo planner", &prompt);
        emit_executor_prompt(
            &mut |line| lines.push(line),
            "GE",
            "Claude todo planner",
            &prompt,
        );
        let claude = run_claude(&self.cwd, &prompt, &self.cancel_flag);
        self.cache_result("GE", "Claude todo planner", &claude.output);
        emit_executor_result(
            &mut |line| lines.push(line),
            "GE",
            "Claude todo planner",
            &claude,
        );
        self.log_executor_run(AuditEventKind::TodoPlanGenerated, None, claude.clone());
        if claude.ok()
            && let Some(todos) = parse_todo_plan_json(&claude.output)
        {
            return (todos, "generated by claude planner".to_string());
        }

        self.cache_prompt("GE", "Codex todo planner (fallback)", &prompt);
        emit_executor_prompt(
            &mut |line| lines.push(line),
            "GE",
            "Codex todo planner (fallback)",
            &prompt,
        );
        let codex = run_codex_execute(&self.cwd, &prompt, &self.cancel_flag);
        self.cache_result("GE", "Codex todo planner (fallback)", &codex.output);
        emit_executor_result(
            &mut |line| lines.push(line),
            "GE",
            "Codex todo planner (fallback)",
            &codex,
        );
        self.log_executor_run(AuditEventKind::TodoPlanGenerated, None, codex.clone());
        if codex.ok()
            && let Some(todos) = parse_todo_plan_json(&codex.output)
        {
            return (todos, "generated by codex planner (fallback)".to_string());
        }

        (
            vec![],
            "planner output invalid; fallback to default template todos".to_string(),
        )
    }

    fn log_executor_run(&self, event: AuditEventKind, todo_id: Option<&str>, run: ExecutorRun) {
        self.log(AuditRecord {
            mode: self.mode,
            event,
            todo_id,
            trigger: None,
            executor: Some(run.executor),
            command: Some(&run.command_line),
            exit_code: Some(run.exit_code),
            status: run.outcome,
            summary: Some(&summarize_output(&run.output, 6)),
            error_code: run.error_code.as_deref(),
        });
    }

    fn log(&self, rec: AuditRecord<'_>) {
        let _ = self.logger.write(rec);
    }

    fn cache_prompt(&mut self, todo_id: &str, stage: &str, prompt: &str) {
        self.last_prompt = Some(PromptSnapshot {
            todo_id: todo_id.to_string(),
            stage: stage.to_string(),
            prompt: prompt.to_string(),
        });
    }

    fn cache_result(&mut self, todo_id: &str, stage: &str, output: &str) {
        self.last_result = Some(ResultSnapshot {
            todo_id: todo_id.to_string(),
            stage: stage.to_string(),
            output: output.to_string(),
        });
    }

    fn cancelled(&self) -> bool {
        self.cancel_flag.load(Ordering::SeqCst)
    }
}

fn now_hms() -> String {
    Local::now().format("%H:%M:%S").to_string()
}

fn defer_todo(doc: &mut ConsensusDoc, todo_id: &str, msg: String) {
    doc.append_status(msg.clone());
    doc.append_journal(format!("- {}", msg.trim_start_matches("- ")));
    if let Some(todo) = doc.todos.iter_mut().find(|t| t.id == todo_id) {
        todo.checked = false;
    }
}

fn emit_line<F>(emit: &mut F, line: impl Into<String>)
where
    F: FnMut(String),
{
    emit(line.into());
}

fn emit_stage_header<F>(emit: &mut F, todo_id: &str, stage: &str)
where
    F: FnMut(String),
{
    emit_line(emit, "  ================================================");
    emit_line(emit, format!("  GE STAGE [{todo_id}] {stage}"));
    emit_line(emit, "  ================================================");
}

fn emit_executor_prompt<F>(emit: &mut F, todo_id: &str, stage: &str, prompt: &str)
where
    F: FnMut(String),
{
    let line_count = prompt.lines().count().max(1);
    let char_count = prompt.chars().count();
    emit_line(
        emit,
        format!(
            "  {todo_id} -> {stage} prompt: [collapsed, {line_count} lines, {char_count} chars]"
        ),
    );
    emit_line(
        emit,
        "  GE: use `GE 展开提示词` (or `GE expand prompt`) to expand.",
    );
}

fn emit_executor_result<F>(emit: &mut F, todo_id: &str, stage: &str, run: &ExecutorRun)
where
    F: FnMut(String),
{
    let error = run.error_code.as_deref().unwrap_or("none");
    let verdict = extract_executor_verdict(&run.output);
    let verdict_suffix = verdict
        .as_deref()
        .map(|v| format!(" verdict={v}"))
        .unwrap_or_default();
    emit_line(
        emit,
        format!(
            "  {todo_id} <- {stage} result: status={} exit={} error={}{}",
            run.outcome.as_status(),
            run.exit_code,
            error,
            verdict_suffix
        ),
    );

    let (summary_lines, hidden_line_count) = summarize_executor_output_for_console(
        &run.output,
        EXECUTOR_OUTPUT_PREVIEW_CHARS,
        EXECUTOR_PREVIEW_MAX_LINES,
    );
    if summary_lines.is_empty() {
        emit_line(emit, "    (no concise output)");
    } else {
        emit_line(emit, "    Summary:");
        for line in summary_lines {
            emit_line(emit, format!("    • {}", line));
        }
    }
    if hidden_line_count > 0 {
        emit_line(
            emit,
            format!(
                "    ...(collapsed {} lines; use `GE 展开结果` or `GE expand result` to view full output)",
                hidden_line_count
            ),
        );
    }
}

fn extract_executor_verdict(output: &str) -> Option<String> {
    for line in output.lines().rev() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("GE_REVIEW_VERDICT:") {
            return Some(rest.trim().to_string());
        }
        if let Some(rest) = trimmed.strip_prefix("GE_EXEC_VERDICT:") {
            return Some(rest.trim().to_string());
        }
    }
    None
}

fn summarize_executor_output_for_console(
    output: &str,
    max_chars: usize,
    max_lines: usize,
) -> (Vec<String>, usize) {
    if max_chars == 0 || max_lines == 0 {
        return (Vec::new(), 0);
    }

    let body = strip_executor_trailer(output);
    let line_max = max_chars.clamp(80, 220);
    let summary_limit = max_lines.min(10);
    let mut total_kept = 0usize;
    let mut shown = Vec::new();
    for raw in body.lines() {
        let trimmed = raw.trim();
        if trimmed.is_empty() || is_executor_noise_line(trimmed) {
            continue;
        }
        if trimmed.starts_with("GE_REVIEW_VERDICT:") || trimmed.starts_with("GE_EXEC_VERDICT:") {
            continue;
        }
        total_kept += 1;
        if shown.len() >= summary_limit {
            continue;
        }
        shown.push(truncate_text(trimmed, line_max));
    }

    let hidden = total_kept.saturating_sub(shown.len());
    (shown, hidden)
}

fn strip_executor_trailer(output: &str) -> &str {
    let mut cut = output.len();
    for marker in [
        "\nOpenAI Codex v",
        "\nOpenAI Claude",
        "\nuser\n",
        "\ntokens used",
    ] {
        if let Some(idx) = output.find(marker) {
            cut = cut.min(idx);
        }
    }
    if output.starts_with("OpenAI Codex v") || output.starts_with("OpenAI Claude") {
        cut = 0;
    }
    &output[..cut]
}

fn is_executor_noise_line(line: &str) -> bool {
    let t = line.trim();
    t == "--------"
        || t.starts_with("workdir:")
        || t.starts_with("model:")
        || t.starts_with("provider:")
        || t.starts_with("approval:")
        || t.starts_with("sandbox:")
        || t.starts_with("reasoning effort:")
        || t.starts_with("reasoning summaries:")
        || t.starts_with("session id:")
        || t.starts_with("mcp:")
        || t.eq_ignore_ascii_case("thinking")
}

fn preview_block_lines(text: &str, max_chars: usize, max_lines: usize) -> Vec<String> {
    if max_chars == 0 || max_lines == 0 {
        return vec!["(output hidden)".to_string()];
    }

    let mut chars = text.chars();
    let mut clipped = String::new();
    for _ in 0..max_chars {
        let Some(ch) = chars.next() else {
            break;
        };
        clipped.push(ch);
    }
    let chars_truncated = chars.next().is_some();

    let mut lines = clipped
        .lines()
        .map(|line| {
            if line.trim().is_empty() {
                "(blank)".to_string()
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>();
    if lines.is_empty() {
        lines.push("(no output)".to_string());
    }

    let mut lines_truncated = false;
    if lines.len() > max_lines {
        lines.truncate(max_lines);
        lines_truncated = true;
    }

    if chars_truncated || lines_truncated {
        lines.push("...(truncated in console view; full details in GE_LOG.jsonl)".to_string());
    }
    lines
}

fn hash_file(path: &PathBuf) -> Result<u64> {
    let bytes = fs::read(path).with_context(|| format!("failed to read `{}`", path.display()))?;
    let mut hasher = DefaultHasher::new();
    bytes.hash(&mut hasher);
    Ok(hasher.finish())
}

fn parse_option_choice(text: &str, option_count: usize) -> Option<usize> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    let token = trimmed.split_whitespace().next().unwrap_or(trimmed);
    let numeric = token.trim_matches(|c: char| !c.is_ascii_digit()).trim();
    if numeric.is_empty() {
        return None;
    }
    let number = numeric.parse::<usize>().ok()?;
    if number == 0 || number > option_count {
        return None;
    }
    Some(number - 1)
}

fn parse_clarify_questions_json(raw: &str) -> Option<Vec<ClarifyQuestion>> {
    let json = extract_json_object(raw)?;
    let value: Value = serde_json::from_str(&json).ok()?;
    let items = value.get("questions")?.as_array()?;
    if items.is_empty() {
        return Some(Vec::new());
    }
    let mut out = Vec::new();
    for item in items.iter().take(MAX_CLARIFY_QUESTIONS_PER_BATCH) {
        let question = item.get("question")?.as_str()?.trim();
        if question.is_empty() {
            return None;
        }
        let options = item.get("options")?.as_array()?;
        if options.len() < 3 {
            return None;
        }
        let o1 = options[0].as_str()?.trim();
        let o2 = options[1].as_str()?.trim();
        let o3 = options[2].as_str()?.trim();
        if o1.is_empty() || o2.is_empty() || o3.is_empty() {
            return None;
        }
        out.push(ClarifyQuestion {
            question: question.to_string(),
            options: [o1.to_string(), o2.to_string(), o3.to_string()],
        });
    }
    Some(out)
}

fn parse_consensus_payload_json(raw: &str) -> Option<GeneratedConsensus> {
    let json = extract_json_object(raw)?;
    let value: Value = serde_json::from_str(&json).ok()?;
    let purpose_lines = value
        .get("purpose_lines")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(ToString::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let rules_lines = value
        .get("rules_lines")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(ToString::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let scope = value
        .get("scope")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or("")
        .to_string();
    let todos = parse_todos_from_value(value.get("todos")?)?;

    if todos.is_empty() {
        return None;
    }
    Some(GeneratedConsensus {
        purpose_lines,
        rules_lines,
        scope,
        todos,
    })
}

fn parse_todos_from_value(todos_value: &Value) -> Option<Vec<TodoItem>> {
    let todos = todos_value.as_array()?;
    let mut out = Vec::new();
    for (idx, item) in todos.iter().enumerate() {
        let text = item.get("text")?.as_str()?.trim();
        if text.is_empty() {
            return None;
        }
        let id = item
            .get("id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| s.starts_with('T'))
            .unwrap_or("");
        let id = if id.is_empty() {
            format!("T{:03}", idx + 1)
        } else {
            id.to_string()
        };
        let done_when = item
            .get("done_when")
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(Value::as_str)
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let assist = item
            .get("assist")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| ["auto", "claude", "codex"].contains(s))
            .map(ToString::to_string)
            .or_else(|| Some("auto".to_string()));
        out.push(TodoItem {
            id,
            text: text.to_string(),
            checked: false,
            done_when: if done_when.is_empty() {
                vec!["Completed and verified by Codex review".to_string()]
            } else {
                done_when
            },
            assist,
        });
    }

    if !(8..=12).contains(&out.len()) {
        return None;
    }
    if !has_sequential_todo_ids(&out) {
        return None;
    }
    Some(out)
}

fn has_sequential_todo_ids(todos: &[TodoItem]) -> bool {
    todos
        .iter()
        .enumerate()
        .all(|(idx, t)| t.id == format!("T{:03}", idx + 1))
}

fn build_consensus_doc_from_generated(
    generated: GeneratedConsensus,
    base_purpose: &str,
    base_rules: &str,
    base_scope: &str,
) -> ConsensusDoc {
    let mut purpose_lines = if generated.purpose_lines.is_empty() {
        lines_from_text(base_purpose)
    } else {
        generated
            .purpose_lines
            .iter()
            .map(|l| normalize_bullet_line(l))
            .collect::<Vec<_>>()
    };
    let rules_lines = if generated.rules_lines.is_empty() {
        lines_from_text(base_rules)
    } else {
        generated
            .rules_lines
            .iter()
            .map(|l| normalize_bullet_line(l))
            .collect::<Vec<_>>()
    };

    let scope = if generated.scope.trim().is_empty() {
        base_scope.trim()
    } else {
        generated.scope.trim()
    };
    if !scope.is_empty() {
        let scope_line = format!("- Scope: {scope}");
        if !purpose_lines
            .iter()
            .any(|l| l.eq_ignore_ascii_case(&scope_line))
        {
            purpose_lines.push(scope_line);
        }
    }

    ConsensusDoc {
        purpose_lines,
        rules_lines,
        todos: generated.todos,
        bot_status_lines: vec!["- GE initialized and waiting for first execution.".to_string()],
        bot_journal_lines: vec![],
    }
}

fn normalize_bullet_line(line: &str) -> String {
    let trimmed = line.trim().trim_start_matches("- ").trim();
    format!("- {trimmed}")
}

fn lines_from_text(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(normalize_bullet_line)
        .collect::<Vec<_>>()
}

fn parse_todo_plan_json(raw: &str) -> Option<Vec<crate::consensus::model::TodoItem>> {
    let json = extract_json_object(raw)?;
    let value: Value = serde_json::from_str(&json).ok()?;
    parse_todos_from_value(value.get("todos")?)
}

fn extract_json_object(raw: &str) -> Option<String> {
    if let Some(start) = raw.find("```json")
        && let Some(end) = raw[start + 7..].find("```")
    {
        let json = raw[start + 7..start + 7 + end].trim();
        if json.starts_with('{') && json.ends_with('}') {
            return Some(json.to_string());
        }
    }

    let start = raw.find('{')?;
    let end = raw.rfind('}')?;
    if end <= start {
        return None;
    }
    let candidate = raw[start..=end].trim();
    if candidate.starts_with('{') && candidate.ends_with('}') {
        Some(candidate.to_string())
    } else {
        None
    }
}

fn truncate_text(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    s.chars().take(max).collect::<String>()
}

fn join_section_lines(lines: &[String]) -> String {
    lines
        .iter()
        .map(|l| l.trim().trim_start_matches("- ").trim())
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn extract_scope(purpose_lines: &[String]) -> String {
    for line in purpose_lines {
        let trimmed = line.trim().trim_start_matches("- ").trim();
        if let Some(scope) = trimmed.strip_prefix("Scope:") {
            return scope.trim().to_string();
        }
        if let Some(scope) = trimmed.strip_prefix("scope:") {
            return scope.trim().to_string();
        }
    }
    String::new()
}

#[cfg(test)]
mod tests {
    use super::{parse_clarify_questions_json, parse_consensus_payload_json, parse_todo_plan_json};

    #[test]
    fn parse_todo_plan_json_accepts_valid_payload() {
        let raw = r#"{"todos":[
            {"id":"T001","text":"Create project scaffold","done_when":["cmd: test -d raw-viewer"],"assist":"claude"},
            {"id":"T002","text":"Implement raw decode pipeline","done_when":["Decode one sample file"],"assist":"codex"},
            {"id":"T003","text":"Build desktop UI shell","done_when":["UI opens and loads sample"],"assist":"auto"},
            {"id":"T004","text":"Add cross-platform packaging","done_when":["cmd: cargo check"],"assist":"auto"},
            {"id":"T005","text":"Add image navigation controls","done_when":["controls work"],"assist":"auto"},
            {"id":"T006","text":"Add metadata panel","done_when":["metadata visible"],"assist":"auto"},
            {"id":"T007","text":"Run validation checks","done_when":["cmd: cargo check"],"assist":"codex"},
            {"id":"T008","text":"Document outcome and next steps","done_when":["journal updated"],"assist":"auto"}
        ]}"#;
        let todos = parse_todo_plan_json(raw).expect("should parse todos");
        assert_eq!(todos.len(), 8);
        assert_eq!(todos[0].id, "T001");
        assert_eq!(todos[0].assist.as_deref(), Some("claude"));
    }

    #[test]
    fn parse_todo_plan_json_rejects_out_of_range_count() {
        let raw =
            r#"{"todos":[{"id":"T001","text":"only one","done_when":["x"],"assist":"auto"}]}"#;
        assert!(parse_todo_plan_json(raw).is_none());
    }

    #[test]
    fn parse_clarify_questions_json_accepts_three_options() {
        let raw = r#"{"questions":[
            {"question":"Pick stack","options":["Rust","C++","Qt"]},
            {"question":"Delivery style","options":["CLI first","GUI first","Both"]}
        ]}"#;
        let questions = parse_clarify_questions_json(raw).expect("should parse");
        assert_eq!(questions.len(), 2);
        assert_eq!(questions[0].options[0], "Rust");
        assert_eq!(questions[0].options[2], "Qt");
    }

    #[test]
    fn parse_clarify_questions_json_allows_empty_list() {
        let raw = r#"{"questions":[]}"#;
        let questions = parse_clarify_questions_json(raw).expect("should parse");
        assert!(questions.is_empty());
    }

    #[test]
    fn parse_consensus_payload_json_accepts_full_payload() {
        let raw = r#"{"purpose_lines":["Build RAW viewer","Cross-platform support"],"rules_lines":["Small steps","Run checks"],"scope":"Only edit rawviewer folder","todos":[
            {"id":"T001","text":"Init app","done_when":["cmd: cargo check"],"assist":"claude"},
            {"id":"T002","text":"Add loader","done_when":["loader compiles"],"assist":"auto"},
            {"id":"T003","text":"Add decode","done_when":["decode sample"],"assist":"codex"},
            {"id":"T004","text":"Render image","done_when":["window shows image"],"assist":"auto"},
            {"id":"T005","text":"Add zoom","done_when":["zoom works"],"assist":"auto"},
            {"id":"T006","text":"Add metadata panel","done_when":["metadata visible"],"assist":"auto"},
            {"id":"T007","text":"Add folder navigation","done_when":["navigate files"],"assist":"auto"},
            {"id":"T008","text":"Run final checks","done_when":["cmd: cargo check"],"assist":"codex"}
        ]}"#;
        let payload = parse_consensus_payload_json(raw).expect("should parse payload");
        assert_eq!(payload.purpose_lines.len(), 2);
        assert_eq!(payload.rules_lines.len(), 2);
        assert_eq!(payload.todos.len(), 8);
        assert_eq!(payload.todos[0].id, "T001");
    }

    #[test]
    fn parse_option_choice_accepts_suffix_forms() {
        assert_eq!(super::parse_option_choice("1", 3), Some(0));
        assert_eq!(super::parse_option_choice("2)", 3), Some(1));
        assert_eq!(super::parse_option_choice("3）", 3), Some(2));
        assert_eq!(super::parse_option_choice("自定义", 3), None);
    }
}
