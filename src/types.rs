#[derive(Debug, Clone)]
pub enum Event {
    Thinking { text: String },
    ToolCall { command: String },
    ToolResult { command: String, exit_code: i32, output: String },
    NeedsConfirmation { command: String, reason: String },
    Final { summary: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfirmationChoice {
    Execute,
    Edit,
    Skip,
    Abort,
}
