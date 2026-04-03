use std::collections::VecDeque;

use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TodoStatus {
    Pending,
    Running,
    Done,
}

#[derive(Debug, Clone)]
pub struct TodoItem {
    pub label: String,
    pub status: TodoStatus,
}

#[derive(Debug, Clone)]
pub(crate) struct QueuedInput {
    pub(crate) text: String,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct InputQueue {
    items: VecDeque<QueuedInput>,
}

impl InputQueue {
    pub(crate) fn push(&mut self, text: String) -> usize {
        self.items.push_back(QueuedInput { text });
        self.items.len()
    }

    pub(crate) fn pop(&mut self) -> Option<QueuedInput> {
        self.items.pop_front()
    }

    pub(crate) fn clear(&mut self) {
        self.items.clear();
    }

    pub(crate) fn len(&self) -> usize {
        self.items.len()
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub(crate) fn labels(&self) -> Vec<String> {
        self.items.iter().map(|item| item.text.clone()).collect()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Normal,
    GeInterview,
    GeRun,
    GeIdle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AssistMode {
    #[default]
    Off,
    AcceptEdits,
    Plan,
}

impl AssistMode {
    pub fn cycle(self) -> Self {
        match self {
            Self::Off => Self::AcceptEdits,
            Self::AcceptEdits => Self::Plan,
            Self::Plan => Self::Off,
        }
    }

    pub fn as_llm_name(self) -> &'static str {
        match self {
            Self::Off => "agent",
            Self::AcceptEdits => "accept_edits",
            Self::Plan => "plan",
        }
    }

    pub fn parse_llm_name(raw: &str) -> Option<Self> {
        let normalized = raw.trim().to_ascii_lowercase().replace(['-', ' '], "_");
        match normalized.as_str() {
            "agent" | "off" | "normal" => Some(Self::Off),
            "accept" | "accept_edits" | "acceptedits" => Some(Self::AcceptEdits),
            "plan" => Some(Self::Plan),
            _ => None,
        }
    }
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
    PhaseSummary {
        text: String,
    },
    ToolCall {
        label: String,
        command: String,
        /// Show all lines of `command` in the live view (e.g. Explorer tree).
        /// When false only the first line is shown.
        multiline: bool,
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
    /// 上下文压缩完成后向 TUI 面板发出的持久事件。
    ConversationCompacted {
        summary: String,
        messages_dropped: usize,
    },
}

#[derive(Debug, Clone)]
pub enum LlmAction {
    Shell {
        command: String,
    },
    WebSearch {
        query: String,
    },
    Plan {
        content: String,
    },
    Phase {
        text: String,
    },
    Question {
        text: String,
        options: Vec<String>,
    },
    SetMode {
        mode: AssistMode,
    },
    Mcp {
        tool: String,
        arguments: Value,
    },
    Skill {
        name: String,
    },
    CreateMcp {
        config: Value,
    },
    Todo {
        items: Vec<TodoItem>,
    },
    WriteFile {
        path: String,
        content: String,
    },
    UpdateFile {
        path: String,
        line_start: usize,
        line_end: usize,
        new_string: String,
    },
    SearchFiles {
        pattern: String,
        path: String,
    },
    GlobFiles {
        pattern: String,
        path: String,
    },
    Task {
        description: String,
        subagent_type: String,
        prompt: String,
    },
    ReadFile {
        path: String,
        offset: Option<usize>,
        limit: Option<usize>,
    },
    /// 派发子Agent任务 DAG（一次性提交完整依赖图）
    SubAgent {
        graph: crate::agent::sub_agent::TaskGraph,
    },
    /// Save a note to project-scoped long-term memory (non-blocking; may appear alongside Final).
    Memory {
        note: String,
    },
    Final {
        summary: String,
    },
}

#[cfg(test)]
mod tests {
    use super::{AssistMode, InputQueue};

    #[test]
    fn assist_mode_cycle_off_to_accept_edits() {
        assert_eq!(AssistMode::Off.cycle(), AssistMode::AcceptEdits);
    }

    #[test]
    fn assist_mode_cycle_accept_edits_to_plan() {
        assert_eq!(AssistMode::AcceptEdits.cycle(), AssistMode::Plan);
    }

    #[test]
    fn assist_mode_cycle_plan_to_off() {
        assert_eq!(AssistMode::Plan.cycle(), AssistMode::Off);
    }

    #[test]
    fn assist_mode_parse_llm_aliases() {
        assert_eq!(AssistMode::parse_llm_name("agent"), Some(AssistMode::Off));
        assert_eq!(AssistMode::parse_llm_name("normal"), Some(AssistMode::Off));
        assert_eq!(
            AssistMode::parse_llm_name("accept_edits"),
            Some(AssistMode::AcceptEdits)
        );
        assert_eq!(AssistMode::parse_llm_name("plan"), Some(AssistMode::Plan));
    }

    #[test]
    fn input_queue_preserves_fifo_order() {
        let mut queue = InputQueue::default();
        queue.push("first".to_string());
        queue.push("second".to_string());

        assert_eq!(queue.pop().map(|item| item.text), Some("first".to_string()));
        assert_eq!(
            queue.pop().map(|item| item.text),
            Some("second".to_string())
        );
        assert!(queue.pop().is_none());
    }

    #[test]
    fn input_queue_labels_reflect_pending_items() {
        let mut queue = InputQueue::default();
        queue.push("alpha".to_string());
        queue.push("beta".to_string());

        assert_eq!(
            queue.labels(),
            vec!["alpha".to_string(), "beta".to_string()]
        );
        assert_eq!(queue.len(), 2);
        assert!(!queue.is_empty());
    }
}
