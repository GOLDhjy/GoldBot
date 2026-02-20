#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RiskLevel {
    Safe,
    Confirm,
    Block,
}

pub fn assess_command(command: &str) -> (RiskLevel, String) {
    let lower = command.to_lowercase();

    // Always block obvious shell bomb patterns.
    if lower.contains(":(){") {
        return (RiskLevel::Block, "Blocked: system-critical command".into());
    }

    let mut should_confirm = contains_unquoted_redirection(command);

    for segment in split_unquoted_segments(command) {
        let tokens = tokenize_shell(&segment);
        let Some((cmd_index, cmd)) = primary_command(&tokens) else {
            continue;
        };

        // Hard blocks
        if matches!(cmd.as_str(), "sudo" | "format" | "diskpart") {
            return (RiskLevel::Block, "Blocked: system-critical command".into());
        }

        if is_confirm_command(&cmd, &tokens, cmd_index) {
            should_confirm = true;
        }
    }

    if should_confirm {
        (
            RiskLevel::Confirm,
            "Potentially destructive or mutating operation".into(),
        )
    } else {
        (RiskLevel::Safe, "Read-only / low-risk".into())
    }
}

fn is_confirm_command(cmd: &str, tokens: &[String], cmd_index: usize) -> bool {
    if matches!(
        cmd,
        "rm"
            | "del"
            | "rmdir"
            | "mv"
            | "ren"
            | "cp"
            | "mkdir"
            | "chmod"
            | "chown"
            | "sed"
            | "perl"
            | "touch"
            | "tee"
            | "curl"
            | "wget"
    ) {
        return true;
    }

    if cmd == "git" {
        // git <subcommand>
        if let Some(sub) = tokens
            .iter()
            .skip(cmd_index + 1)
            .find(|t| !t.starts_with('-'))
            .map(|s| normalize_command_token(s))
        {
            return matches!(
                sub.as_str(),
                "add"
                    | "rm"
                    | "mv"
                    | "commit"
                    | "push"
                    | "rebase"
                    | "reset"
                    | "checkout"
                    | "switch"
                    | "clean"
                    | "restore"
                    | "apply"
                    | "cherry-pick"
                    | "merge"
            );
        }
    }

    false
}

fn primary_command(tokens: &[String]) -> Option<(usize, String)> {
    if tokens.is_empty() {
        return None;
    }

    let mut i = 0;
    while i < tokens.len() && is_env_assignment(&tokens[i]) {
        i += 1;
    }
    if i >= tokens.len() {
        return None;
    }

    let mut cmd = normalize_command_token(&tokens[i]);
    if cmd == "env" {
        i += 1;
        while i < tokens.len() && (tokens[i].starts_with('-') || is_env_assignment(&tokens[i])) {
            i += 1;
        }
        if i >= tokens.len() {
            return None;
        }
        cmd = normalize_command_token(&tokens[i]);
    }

    Some((i, cmd))
}

fn normalize_command_token(token: &str) -> String {
    let base = token.rsplit('/').next().unwrap_or(token);
    base.to_lowercase()
}

fn is_env_assignment(token: &str) -> bool {
    if token.starts_with('-') {
        return false;
    }
    let Some(eq) = token.find('=') else {
        return false;
    };
    if eq == 0 {
        return false;
    }
    token[..eq]
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn split_unquoted_segments(command: &str) -> Vec<String> {
    let b = command.as_bytes();
    let mut out = Vec::new();
    let mut start = 0usize;
    let mut i = 0usize;
    let mut in_single = false;
    let mut in_double = false;
    let mut escaped = false;

    while i < b.len() {
        let c = b[i];
        if escaped {
            escaped = false;
            i += 1;
            continue;
        }
        if c == b'\\' && !in_single {
            escaped = true;
            i += 1;
            continue;
        }
        if c == b'\'' && !in_double {
            in_single = !in_single;
            i += 1;
            continue;
        }
        if c == b'"' && !in_single {
            in_double = !in_double;
            i += 1;
            continue;
        }

        if !in_single && !in_double {
            let split_len = if c == b';' || c == b'\n' {
                1
            } else if i + 1 < b.len()
                && ((c == b'&' && b[i + 1] == b'&') || (c == b'|' && b[i + 1] == b'|'))
            {
                2
            } else if c == b'|' || c == b'&' {
                1
            } else {
                0
            };

            if split_len > 0 {
                let seg = command[start..i].trim();
                if !seg.is_empty() {
                    out.push(seg.to_string());
                }
                i += split_len;
                start = i;
                continue;
            }
        }

        i += 1;
    }

    let seg = command[start..].trim();
    if !seg.is_empty() {
        out.push(seg.to_string());
    }
    out
}

fn tokenize_shell(segment: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_single = false;
    let mut in_double = false;
    let mut escaped = false;

    for ch in segment.chars() {
        if escaped {
            cur.push(ch);
            escaped = false;
            continue;
        }
        match ch {
            '\\' if !in_single => escaped = true,
            '\'' if !in_double => in_single = !in_single,
            '"' if !in_single => in_double = !in_double,
            c if c.is_whitespace() && !in_single && !in_double => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            c => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

fn contains_unquoted_redirection(command: &str) -> bool {
    let b = command.as_bytes();
    let mut i = 0usize;
    let mut in_single = false;
    let mut in_double = false;
    let mut escaped = false;

    while i < b.len() {
        let c = b[i];
        if escaped {
            escaped = false;
            i += 1;
            continue;
        }
        if c == b'\\' && !in_single {
            escaped = true;
            i += 1;
            continue;
        }
        if c == b'\'' && !in_double {
            in_single = !in_single;
            i += 1;
            continue;
        }
        if c == b'"' && !in_single {
            in_double = !in_double;
            i += 1;
            continue;
        }

        if !in_single && !in_double {
            if c == b'>' {
                if i + 1 < b.len() && b[i + 1] == b'&' {
                    i += 2;
                    continue;
                }
                return true;
            }
            if c == b'<' && i + 1 < b.len() && b[i + 1] == b'<' {
                return true;
            }
        }

        i += 1;
    }

    false
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

    #[test]
    fn echo_with_risky_words_in_quotes_is_safe() {
        let cmd = "echo \"- File operations (ls, cat, mkdir, etc.)\" && echo \"- System information\"";
        let (risk, _) = assess_command(cmd);
        assert_eq!(risk, RiskLevel::Safe);
    }

    #[test]
    fn git_add_requires_confirmation() {
        let (risk, _) = assess_command("git add src/main.rs");
        assert_eq!(risk, RiskLevel::Confirm);
    }
}
