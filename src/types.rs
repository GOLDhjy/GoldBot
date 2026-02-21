use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Normal,
    GeInterview,
    GeRun,
    GeIdle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GeQuestionStep {
    Purpose,
    Rules,
    Scope,
    Clarify,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConsensusTrigger {
    Manual,
    TaskDone,
    Periodic,
    FileChanged,
}

impl ConsensusTrigger {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::TaskDone => "task_done",
            Self::Periodic => "periodic",
            Self::FileChanged => "file_changed",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutorOutcome {
    Success,
    Failed,
    BlockedConfirm,
    BlockedSafety,
}

impl ExecutorOutcome {
    pub fn as_status(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Failed => "failed",
            Self::BlockedConfirm => "blocked_confirm",
            Self::BlockedSafety => "blocked_safety",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuditEventKind {
    GeEntered,
    GeExited,
    GeInput,
    GeQuestionAsked,
    GeQuestionAnswered,
    ConsensusLoaded,
    ConsensusGenerated,
    TodoPlanGenerated,
    ClarifyGenerated,
    Preflight,
    Trigger,
    TodoSelected,
    ClaudeExec,
    CodexExec,
    SelfReview,
    GitCommit,
    Validation,
    TodoChecked,
    TodoDeferred,
    Error,
}

impl AuditEventKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::GeEntered => "ge_entered",
            Self::GeExited => "ge_exited",
            Self::GeInput => "ge_input",
            Self::GeQuestionAsked => "ge_question_asked",
            Self::GeQuestionAnswered => "ge_question_answered",
            Self::ConsensusLoaded => "consensus_loaded",
            Self::ConsensusGenerated => "consensus_generated",
            Self::TodoPlanGenerated => "todo_plan_generated",
            Self::ClarifyGenerated => "clarify_generated",
            Self::Preflight => "preflight",
            Self::Trigger => "trigger",
            Self::TodoSelected => "todo_selected",
            Self::ClaudeExec => "claude_exec",
            Self::CodexExec => "codex_exec",
            Self::SelfReview => "self_review",
            Self::GitCommit => "git_commit",
            Self::Validation => "validation",
            Self::TodoChecked => "todo_checked",
            Self::TodoDeferred => "todo_deferred",
            Self::Error => "error",
        }
    }
}

#[derive(Debug, Clone)]
pub enum Event {
    /// User's task/question — shown as "❯ ..." in the log.
    UserTask {
        text: String,
    },
    Thinking {
        text: String,
    },
    ToolCall {
        label: String,
        command: String,
    },
    ToolResult {
        exit_code: i32,
        output: String,
    },
    NeedsConfirmation {
        command: String,
        #[allow(dead_code)]
        reason: String,
    },
    Final {
        summary: String,
    },
}

#[derive(Debug, Clone)]
pub enum LlmAction {
    Shell { command: String },
    Mcp { tool: String, arguments: Value },
    Skill { name: String },
    CreateMcp { config: Value },
    Final { summary: String },
}
