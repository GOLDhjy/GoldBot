/// 解析 CLI 参数，返回 (prompt, auto_accept)。
/// 支持的标志：
///   -p / --prompt <text>   启动时发送的初始任务消息。
///   -y / --yes             自动接受所有 Confirm 级别的命令（非 Block）。
pub(crate) fn parse_cli_args() -> (Option<String>, bool) {
    let args: Vec<String> = std::env::args().collect();
    let mut prompt = None;
    let mut yes = false;
    let mut i = 1;
    while i < args.len() {
        if (args[i] == "-p" || args[i] == "--prompt") && i + 1 < args.len() {
            prompt = Some(args[i + 1].clone());
            i += 2;
        } else if args[i] == "-y" || args[i] == "--yes" {
            yes = true;
            i += 1;
        } else {
            i += 1;
        }
    }
    (prompt, yes)
}

/// 若 `~/.goldbot/.env` 不存在，则从内置模板创建。
pub(crate) fn ensure_dot_env() {
    let home = crate::tools::mcp::goldbot_home_dir();
    let env_path = home.join(".env");
    if env_path.exists() {
        return;
    }
    let _ = std::fs::create_dir_all(&home);
    let _ = std::fs::write(&env_path, include_str!("../.env.example"));
}
