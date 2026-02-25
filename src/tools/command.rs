use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
};

// ── 数据类型 ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct Command {
    pub name: String,
    pub description: String,
    pub action: CommandAction,
}

#[derive(Debug, Clone)]
pub enum CommandAction {
    Builtin(BuiltinCommand),
    /// 用户自定义命令：把 COMMAND.md body 填入输入框供用户编辑后提交。
    Template(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuiltinCommand {
    Help,
    Clear,
    Compact,
    Memory,
    Thinking,
    Skills,
    Mcp,
    Status,
}

// ── 内置命令列表（单一数据源）────────────────────────────────────────────────

/// (variant, name, description)
const BUILTIN_COMMANDS: &[(BuiltinCommand, &str, &str)] = &[
    (BuiltinCommand::Help, "help", "显示键位绑定和可用命令列表"),
    (BuiltinCommand::Clear, "clear", "清除会话历史，重新开始对话"),
    (
        BuiltinCommand::Compact,
        "compact",
        "立即压缩上下文（节省 Token）",
    ),
    (
        BuiltinCommand::Memory,
        "memory",
        "查看当前长期和短期记忆内容",
    ),
    (
        BuiltinCommand::Thinking,
        "thinking",
        "切换原生 Thinking 模式（同 Tab）",
    ),
    (BuiltinCommand::Skills, "skills", "列出所有已发现的 Skill"),
    (BuiltinCommand::Mcp, "mcp", "列出所有已注册的 MCP 工具"),
    (
        BuiltinCommand::Status,
        "status",
        "显示 workspace、模型、环境配置摘要",
    ),
];

// ── 目录发现 ──────────────────────────────────────────────────────────────────

/// GoldBot 用户命令目录：`~/.goldbot/command/`
pub fn goldbot_command_dir() -> PathBuf {
    crate::tools::mcp::goldbot_home_dir().join("command")
}

/// 扫描 `~/.goldbot/command/` 目录，返回所有有效的用户自定义命令。
pub fn discover_commands() -> Vec<Command> {
    let mut commands = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    scan_dir(&goldbot_command_dir(), &mut commands, &mut seen);
    commands
}

/// 返回内置命令 + 用户自定义命令的完整列表（内置命令优先）。
pub fn all_commands(user_commands: &[Command]) -> Vec<Command> {
    let mut out: Vec<Command> = BUILTIN_COMMANDS
        .iter()
        .map(|&(variant, name, desc)| Command {
            name: name.to_string(),
            description: desc.to_string(),
            action: CommandAction::Builtin(variant),
        })
        .collect();
    out.extend_from_slice(user_commands);
    out
}

/// 按 query 前缀/包含匹配过滤命令列表（大小写不敏感）。
/// query 为空时返回全部。
pub fn filter_commands<'a>(commands: &'a [Command], query: &str) -> Vec<&'a Command> {
    if query.is_empty() {
        return commands.iter().collect();
    }
    let q = query.to_lowercase();
    commands
        .iter()
        .filter(|c| c.name.to_lowercase().contains(&q) || c.description.to_lowercase().contains(&q))
        .collect()
}

/// 若存在用户自定义命令，返回启动时状态行提示字符串。
pub fn format_commands_status_line(user_commands: &[Command]) -> Option<String> {
    if user_commands.is_empty() {
        return None;
    }
    Some(format!(
        "  {} user command{} loaded from ~/.goldbot/command/",
        user_commands.len(),
        if user_commands.len() == 1 { "" } else { "s" }
    ))
}

// ── 私有辅助 ──────────────────────────────────────────────────────────────────

fn scan_dir(dir: &Path, commands: &mut Vec<Command>, seen: &mut HashSet<String>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let cmd_file = path.join("COMMAND.md");
        if !cmd_file.exists() {
            continue;
        }
        if let Some(cmd) = parse_command(&cmd_file, &path)
            && seen.insert(cmd.name.clone())
        {
            commands.push(cmd);
        }
    }
}

fn parse_command(file: &Path, dir: &Path) -> Option<Command> {
    let raw = fs::read_to_string(file).ok()?;
    let (meta, body) = parse_frontmatter(&raw)?;

    let name = meta.get("name")?.trim().to_string();
    if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return None;
    }
    // frontmatter name 必须与目录名匹配
    if dir.file_name().and_then(|n| n.to_str()) != Some(name.as_str()) {
        return None;
    }

    let description = meta
        .get("description")
        .map(|s| s.trim().to_string())
        .unwrap_or_default();

    Some(Command {
        name,
        description,
        action: CommandAction::Template(body.trim().to_string()),
    })
}

fn parse_frontmatter(content: &str) -> Option<(std::collections::HashMap<String, String>, String)> {
    let mut lines = content.lines();
    let first = lines.next()?;
    let first = first.strip_prefix('\u{feff}').unwrap_or(first);
    if first.trim() != "---" {
        return None;
    }
    let mut meta = std::collections::HashMap::new();
    let mut body_lines: Vec<&str> = Vec::new();
    let mut in_body = false;
    for line in lines {
        if !in_body && line.trim() == "---" {
            in_body = true;
            continue;
        }
        if in_body {
            body_lines.push(line);
        } else if let Some((k, v)) = line.split_once(':') {
            meta.insert(k.trim().to_string(), strip_yaml_quotes(v.trim()));
        }
    }
    if !in_body {
        return None;
    }
    Some((meta, body_lines.join("\n")))
}

fn strip_yaml_quotes(s: &str) -> String {
    if (s.starts_with('"') && s.ends_with('"')) || (s.starts_with('\'') && s.ends_with('\'')) {
        s[1..s.len() - 1].to_string()
    } else {
        s.to_string()
    }
}
