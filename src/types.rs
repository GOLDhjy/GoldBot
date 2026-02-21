use serde_json::Value;

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
    Final { summary: String },
}
