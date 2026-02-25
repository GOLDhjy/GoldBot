use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
};

const BUILTIN_EXAMPLE_COMMAND_NAME: &str = "commit";
const BUILTIN_EXAMPLE_COMMAND_MD: &str = include_str!("../../builtin-commands/commit.md");

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
    /// 用户自定义命令：选中后把 body 内容展开发送给 LLM。
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
    Model,
}

// ── 内置命令列表（单一数据源）────────────────────────────────────────────────

/// (variant, name, description)
const BUILTIN_COMMANDS: &[(BuiltinCommand, &str, &str)] = &[
    (BuiltinCommand::Clear, "clear", "清除会话历史，重新开始对话"),
    (
        BuiltinCommand::Compact,
        "compact",
        "立即压缩上下文（节省 Token）",
    ),
    (BuiltinCommand::Help, "help", "显示键位绑定和可用命令列表"),
    (BuiltinCommand::Mcp, "mcp", "列出所有已注册的 MCP 工具"),
    (
        BuiltinCommand::Memory,
        "memory",
        "查看当前长期和短期记忆内容",
    ),
    (BuiltinCommand::Model, "model", "切换 LLM 后端与模型"),
    (BuiltinCommand::Skills, "skills", "列出所有已发现的 Skill"),
    (
        BuiltinCommand::Status,
        "status",
        "显示 workspace、模型、环境配置摘要",
    ),
    (
        BuiltinCommand::Thinking,
        "thinking",
        "切换原生 Thinking 模式（同 Tab）",
    ),
];

// ── 目录发现 ──────────────────────────────────────────────────────────────────

/// GoldBot 用户命令目录：`~/.goldbot/commands/`
pub fn goldbot_command_dir() -> PathBuf {
    crate::tools::mcp::goldbot_home_dir().join("commands")
}

/// 将内置示例命令安装到 `~/.goldbot/commands/commit.md`，已存在则跳过。
pub fn ensure_builtin_commands() -> Vec<String> {
    let mut warnings = Vec::new();
    let dir = goldbot_command_dir();
    let path = dir.join(format!("{}.md", BUILTIN_EXAMPLE_COMMAND_NAME));
    if path.exists() {
        return warnings;
    }
    if let Err(e) =
        fs::create_dir_all(&dir).and_then(|_| fs::write(&path, BUILTIN_EXAMPLE_COMMAND_MD))
    {
        warnings.push(format!(
            "failed to install built-in command `{}`: {e}",
            BUILTIN_EXAMPLE_COMMAND_NAME
        ));
    }
    warnings
}

/// 扫描用户命令目录，返回所有有效的用户自定义命令。
/// 优先级：`~/.goldbot/commands/` → `~/.claude/commands/`
pub fn discover_commands() -> Vec<Command> {
    let mut commands = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    // GoldBot 专属目录
    scan_flat_dir(&goldbot_command_dir(), &mut commands, &mut seen);

    // Claude Code 兼容目录
    if let Some(home) = crate::tools::home_dir() {
        scan_flat_dir(
            &home.join(".claude").join("commands"),
            &mut commands,
            &mut seen,
        );
    }

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

/// 按 query 包含匹配过滤命令列表（大小写不敏感）。query 为空返回全部。
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
// ── 私有辅助 ──────────────────────────────────────────────────────────────────

/// 扫描平铺目录：每个 `<name>.md` 文件即一个命令。
fn scan_flat_dir(dir: &Path, commands: &mut Vec<Command>, seen: &mut HashSet<String>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        if let Some(cmd) = parse_flat_command(&path)
            && seen.insert(cmd.name.clone())
        {
            commands.push(cmd);
        }
    }
}

/// 解析平铺命令文件（`<name>.md`）。
/// - 文件名（不含扩展名）作为命令名
/// - frontmatter 中 `description` 作为描述（可选）
/// - frontmatter body 作为模板内容；无 frontmatter 则整个文件作为模板
fn parse_flat_command(file: &Path) -> Option<Command> {
    let name = file.file_stem()?.to_str()?.to_string();
    if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return None;
    }
    let raw = fs::read_to_string(file).ok()?;
    let (description, body) = if let Some((meta, body)) = parse_frontmatter(&raw) {
        let desc = meta
            .get("description")
            .map(|s| s.trim().to_string())
            .unwrap_or_default();
        (desc, body.trim().to_string())
    } else {
        (String::new(), raw.trim().to_string())
    };

    Some(Command {
        name,
        description,
        action: CommandAction::Template(body),
    })
}

fn parse_frontmatter(content: &str) -> Option<(HashMap<String, String>, String)> {
    let mut lines = content.lines();
    let first = lines.next()?;
    let first = first.strip_prefix('\u{feff}').unwrap_or(first);
    if first.trim() != "---" {
        return None;
    }
    let mut meta = HashMap::new();
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
