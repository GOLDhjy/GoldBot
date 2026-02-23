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
        return (RiskLevel::Block, "已拦截：系统关键命令".into());
    }

    let has_output_redirection = contains_unquoted_output_redirection(command);
    let mut should_confirm = has_output_redirection;
    let mut confirm_reason = if has_output_redirection {
        Some("需要确认：命令包含重定向（> / >>），会写入文件".to_string())
    } else {
        None
    };

    for segment in split_unquoted_segments(command) {
        let tokens = tokenize_shell(&segment);
        let Some((cmd_index, cmd)) = primary_command(&tokens) else {
            continue;
        };

        // Hard blocks
        if matches!(cmd.as_str(), "sudo" | "format" | "diskpart") {
            return (RiskLevel::Block, "已拦截：系统关键命令".into());
        }

        if is_confirm_command(&cmd, &tokens, cmd_index) {
            should_confirm = true;
            if confirm_reason.is_none() {
                confirm_reason = Some("需要确认：该命令可能会修改文件或系统状态".to_string());
            }
        }
    }

    if should_confirm {
        (RiskLevel::Confirm, confirm_reason.unwrap_or_default())
    } else {
        (RiskLevel::Safe, "低风险只读命令".into())
    }
}

fn is_confirm_command(cmd: &str, tokens: &[String], cmd_index: usize) -> bool {
    if cmd == "sed" {
        // `sed -n ...` is read-only; only in-place edits need confirmation.
        return sed_in_place_edit(tokens, cmd_index);
    }

    if matches!(
        cmd,
        "rm" | "del"
            | "rmdir"
            | "mv"
            | "ren"
            | "cp"
            | "chmod"
            | "chown"
            | "perl"
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

fn sed_in_place_edit(tokens: &[String], cmd_index: usize) -> bool {
    tokens
        .iter()
        .skip(cmd_index + 1)
        .any(|t| t == "-i" || t.starts_with("-i"))
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

/// Strip the body lines of heredoc blocks so their content is not mistaken
/// for shell commands.  The delimiter line and the `<<`-bearing line are kept;
/// only the interior content lines are removed.
///
/// Example input:
///   cat > README.md << 'EOF'\nsudo make install\nEOF
/// becomes:
///   cat > README.md << 'EOF'\nEOF
fn strip_heredoc_bodies(command: &str) -> String {
    let mut out = String::new();
    let mut delimiter: Option<String> = None;

    for line in command.split('\n') {
        if let Some(ref delim) = delimiter {
            // Inside heredoc body — check for closing delimiter.
            if line.trim() == delim.as_str() {
                delimiter = None;
                out.push_str(line);
                out.push('\n');
            }
            // Skip body line — do NOT push it to `out`.
            continue;
        }

        // Outside heredoc — check if this line opens one.
        if let Some(delim) = extract_heredoc_delimiter(line) {
            delimiter = Some(delim);
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

/// Extract the heredoc closing delimiter from a line that contains `<<`.
/// Returns `None` if no heredoc marker is found.
fn extract_heredoc_delimiter(line: &str) -> Option<String> {
    // Find `<<` (heredoc) or `<<-` (strip-tabs heredoc).
    let pos = line.find("<<")?;
    let after = line[pos + 2..].trim_start_matches('-').trim_start();
    let delim = if after.starts_with('\'') {
        after[1..].split('\'').next().unwrap_or("").to_string()
    } else if after.starts_with('"') {
        after[1..].split('"').next().unwrap_or("").to_string()
    } else {
        after.split_whitespace().next().unwrap_or("").to_string()
    };
    if delim.is_empty() { None } else { Some(delim) }
}

fn split_unquoted_segments(command: &str) -> Vec<String> {
    let command = &strip_heredoc_bodies(command);
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

fn contains_unquoted_output_redirection(command: &str) -> bool {
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
                let mut j = i + 1;
                // >> (append) is safe — it can't destroy existing content.
                // >| (noclobber override) is treated same as >.
                if j < b.len() && b[j] == b'>' {
                    // append redirect — skip and continue
                    i = j + 1;
                    continue;
                }
                if j < b.len() && b[j] == b'|' {
                    j += 1;
                }

                while j < b.len() && b[j].is_ascii_whitespace() {
                    j += 1;
                }
                if j >= b.len() {
                    return true;
                }

                // FD duplication like 2>&1 is not a filesystem mutation.
                if b[j] == b'&' {
                    i = j + 1;
                    continue;
                }

                let start = j;
                while j < b.len() {
                    let ch = b[j];
                    if ch.is_ascii_whitespace() || matches!(ch, b';' | b'|' | b'&') {
                        break;
                    }
                    j += 1;
                }

                let target = command[start..j].trim_matches(|ch| matches!(ch, '"' | '\''));
                if is_safe_redirection_target(target) {
                    i = j;
                    continue;
                }

                return true;
            }
        }

        i += 1;
    }

    false
}

fn is_safe_redirection_target(target: &str) -> bool {
    if target == "/dev/null" {
        return true;
    }
    cfg!(target_os = "windows") && target.eq_ignore_ascii_case("nul")
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
        let cmd =
            "echo \"- File operations (ls, cat, mkdir, etc.)\" && echo \"- System information\"";
        let (risk, _) = assess_command(cmd);
        assert_eq!(risk, RiskLevel::Safe);
    }

    #[test]
    fn git_add_requires_confirmation() {
        let (risk, _) = assess_command("git add src/main.rs");
        assert_eq!(risk, RiskLevel::Confirm);
    }

    #[test]
    fn sed_print_mode_is_safe() {
        let (risk, _) = assess_command("sed -n '738,920p' src/main.rs");
        assert_eq!(risk, RiskLevel::Safe);
    }

    #[test]
    fn sed_in_place_requires_confirmation() {
        let (risk, _) = assess_command("sed -i '' 's/foo/bar/g' src/main.rs");
        assert_eq!(risk, RiskLevel::Confirm);
    }

    #[test]
    fn cat_read_is_safe() {
        let (risk, _) = assess_command("cat README.md");
        assert_eq!(risk, RiskLevel::Safe);
    }

    #[test]
    fn cat_heredoc_alone_is_safe() {
        let (risk, _) = assess_command("cat << 'EOF'");
        assert_eq!(risk, RiskLevel::Safe);
    }

    #[test]
    fn cat_heredoc_with_redirect_requires_confirmation() {
        let (risk, reason) = assess_command("cat > README_EN.md << 'EOF'");
        assert_eq!(risk, RiskLevel::Confirm);
        assert!(reason.contains("重定向"), "unexpected reason: {reason}");
    }

    #[test]
    fn find_with_stderr_redirect_to_dev_null_is_safe() {
        let (risk, _) =
            assess_command(r#"find .. -maxdepth 2 -type d -iname "*gold*" 2>/dev/null"#);
        assert_eq!(risk, RiskLevel::Safe);
    }

    #[test]
    fn ls_stderr_redirect_to_dev_null_is_safe() {
        let (risk, _) =
            assess_command(r#"ls -la .github/ 2>/dev/null || echo "No .github directory""#);
        assert_eq!(risk, RiskLevel::Safe);
    }

    #[test]
    fn ls_redirect_to_file_requires_confirmation() {
        let (risk, reason) = assess_command("ls > out.txt");
        assert_eq!(risk, RiskLevel::Confirm);
        assert!(reason.contains("重定向"), "unexpected reason: {reason}");
    }

    #[test]
    fn append_redirect_is_safe() {
        let (risk, _) = assess_command("cat >> README.md << 'EOF'");
        assert_eq!(risk, RiskLevel::Safe);
    }

    #[test]
    fn echo_append_is_safe() {
        let (risk, _) = assess_command("echo 'hello' >> notes.txt");
        assert_eq!(risk, RiskLevel::Safe);
    }

    #[test]
    fn mkdir_p_is_safe() {
        let (risk, _) = assess_command("mkdir -p .github/workflows Formula");
        assert_eq!(risk, RiskLevel::Safe);
    }

    #[test]
    fn heredoc_with_sudo_in_content_is_confirm_not_block() {
        // The heredoc body contains "sudo" but that's just README content —
        // should be Confirm (due to > redirect), never Block.
        let cmd = "cat > README.md << 'EOF'\n## Install\nsudo make install\nEOF";
        let (risk, _) = assess_command(cmd);
        assert_eq!(risk, RiskLevel::Confirm);
    }

    #[test]
    fn heredoc_without_redirect_is_safe() {
        let cmd = "cat << 'EOF'\nsudo make install\nEOF";
        let (risk, _) = assess_command(cmd);
        assert_eq!(risk, RiskLevel::Safe);
    }
}
