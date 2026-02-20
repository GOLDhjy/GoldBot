#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RiskLevel {
    Safe,
    Confirm,
    Block,
}

pub fn assess_command(command: &str) -> (RiskLevel, String) {
    let lower = command.to_lowercase();

    if contains_shell_word(&lower, "sudo")
        || contains_shell_word(&lower, "format")
        || contains_shell_word(&lower, "diskpart")
        || lower.contains(":(){")
    {
        return (RiskLevel::Block, "Blocked: system-critical command".into());
    }

    let risky_words = [
        "rm", "del", "rmdir", "mv", "ren", "cp", "mkdir", "chmod", "chown", "sed", "perl", "touch",
        "tee", "curl", "wget",
    ];
    let has_risky_word = risky_words.iter().any(|w| contains_shell_word(&lower, w));
    let has_redirection = lower.contains(" >")
        || lower.contains(">>")
        || lower.contains("<<")
        || lower.trim_start().starts_with('>');

    if has_risky_word || has_redirection {
        return (
            RiskLevel::Confirm,
            "Potentially destructive or mutating operation".into(),
        );
    }

    (RiskLevel::Safe, "Read-only / low-risk".into())
}

fn contains_shell_word(text: &str, word: &str) -> bool {
    let text_b = text.as_bytes();
    let word_b = word.as_bytes();
    if word_b.is_empty() || text_b.len() < word_b.len() {
        return false;
    }

    let mut i = 0;
    while i + word_b.len() <= text_b.len() {
        if &text_b[i..i + word_b.len()] == word_b {
            let left_ok = i == 0 || !is_shell_word_char(text_b[i - 1]);
            let right_idx = i + word_b.len();
            let right_ok = right_idx == text_b.len() || !is_shell_word_char(text_b[right_idx]);
            if left_ok && right_ok {
                return true;
            }
        }
        i += 1;
    }
    false
}

fn is_shell_word_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.' | b'/')
}

#[cfg(test)]
mod tests {
    use super::{RiskLevel, assess_command};

    #[test]
    fn rm_requires_confirmation() {
        let (risk, _) = assess_command("rm README_EN.md");
        assert_eq!(risk, RiskLevel::Confirm);
    }

    #[test]
    fn ls_is_safe() {
        let (risk, _) = assess_command("ls -la");
        assert_eq!(risk, RiskLevel::Safe);
    }
}
