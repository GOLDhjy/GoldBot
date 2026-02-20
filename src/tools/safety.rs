#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RiskLevel {
    Safe,
    Confirm,
    Block,
}

pub fn assess_command(command: &str) -> (RiskLevel, String) {
    let lower = command.to_lowercase();

    if lower.contains("sudo")
        || lower.contains("format")
        || lower.contains("diskpart")
        || lower.contains(":(){")
    {
        return (RiskLevel::Block, "Blocked: system-critical command".into());
    }

    let risky = [" rm ", "rm -", "del ", "rmdir", "mv ", "ren ", ">", "curl ", "wget "];
    if risky.iter().any(|k| lower.contains(k)) {
        return (RiskLevel::Confirm, "Potentially destructive or mutating operation".into());
    }

    (RiskLevel::Safe, "Read-only / low-risk".into())
}
