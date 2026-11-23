use anyhow::{Context, Result};
use std::{process::Command, time::Duration};

// 官方安装脚本地址，与 README 中的安装命令保持一致。
const INSTALL_PS1_URL: &str =
    "https://raw.githubusercontent.com/GOLDhjy/GoldBot/master/scripts/install.ps1";
const INSTALL_SH_URL: &str =
    "https://raw.githubusercontent.com/GOLDhjy/GoldBot/master/scripts/install.sh";
const GITHUB_REPO: &str = "GOLDhjy/GoldBot";
const UPDATE_CHECK_TIMEOUT_SECS: u64 = 10;

/// `goldbot update` 子命令入口：检查最新版本后按平台执行官方安装脚本自更新。
pub(crate) fn run() -> Result<()> {
    let current = env!("CARGO_PKG_VERSION");
    println!("GoldBot v{current}");

    // reqwest::blocking 自带独立 runtime，在 tokio 异步上下文里创建/销毁会 panic，放到独立线程执行。
    let latest = std::thread::spawn(fetch_latest_tag).join().ok().flatten();
    if let Some(latest) = latest {
        if !is_newer_tag(&latest, current) {
            println!("已是最新版本 {latest}，无需更新。");
            return Ok(());
        }
        println!("发现新版本 {latest}，开始更新...");
    } else {
        println!("无法检查最新版本，直接执行更新脚本...");
    }

    cleanup_stale_backup();
    let (program, args) = build_update_command(cfg!(windows));
    let status = Command::new(&program)
        .args(&args)
        .status()
        .with_context(|| format!("无法启动更新命令 {program}"))?;
    if !status.success() {
        anyhow::bail!(
            "更新脚本执行失败（退出码 {:?}），可稍后重试或参考 README 手动更新。",
            status.code()
        );
    }
    // 旧进程仍在运行时备份文件可能删不掉，留待下次启动清理。
    cleanup_stale_backup();
    Ok(())
}

/// 构造各平台的更新命令：
/// Windows 用 PowerShell 执行 install.ps1，其余平台用 bash 执行 install.sh。
fn build_update_command(windows: bool) -> (String, Vec<String>) {
    if windows {
        (
            "powershell".to_string(),
            vec![
                "-NoProfile".to_string(),
                "-ExecutionPolicy".to_string(),
                "Bypass".to_string(),
                "-Command".to_string(),
                format!("irm '{INSTALL_PS1_URL}' | iex"),
            ],
        )
    } else {
        (
            "bash".to_string(),
            vec![
                "-c".to_string(),
                format!("curl -fsSL {INSTALL_SH_URL} | bash"),
            ],
        )
    }
}

/// 查询最新 release tag：优先走 GitHub API，失败（如匿名限流 403）时降级为
/// 解析 github.com 的 `/releases/latest` 重定向；两者都失败才返回 None。
fn fetch_latest_tag() -> Option<String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(UPDATE_CHECK_TIMEOUT_SECS))
        .user_agent(concat!("goldbot/", env!("CARGO_PKG_VERSION")))
        .build()
        .ok()?;
    fetch_tag_from_api(&client).or_else(fetch_tag_from_redirect)
}

/// 通过 api.github.com 查询；任何失败返回 None，交给降级方案兜底。
fn fetch_tag_from_api(client: &reqwest::blocking::Client) -> Option<String> {
    let json: serde_json::Value = client
        .get(format!(
            "https://api.github.com/repos/{GITHUB_REPO}/releases/latest"
        ))
        .send()
        .ok()?
        .error_for_status()
        .ok()?
        .json()
        .ok()?;
    json.get("tag_name")?.as_str().map(str::to_string)
}

/// 降级方案：`https://github.com/<repo>/releases/latest` 会 302 到
/// `/releases/tag/<tag>`，该端点不受 API 匿名限流约束。
fn fetch_tag_from_redirect() -> Option<String> {
    let no_redirect = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(UPDATE_CHECK_TIMEOUT_SECS))
        .user_agent(concat!("goldbot/", env!("CARGO_PKG_VERSION")))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .ok()?;
    let resp = no_redirect
        .get(format!("https://github.com/{GITHUB_REPO}/releases/latest"))
        .send()
        .ok()?
        .error_for_status()
        .ok()?;
    tag_from_location(
        resp.headers()
            .get(reqwest::header::LOCATION)?
            .to_str()
            .ok()?,
    )
}

/// 从重定向地址（如 `.../releases/tag/v0.9.23`）提取末尾的版本 tag；
/// 只接受形如 v0.9.23 的 tag，避免把异常页面当版本号。
fn tag_from_location(location: &str) -> Option<String> {
    let tag = location.trim_end_matches('/').rsplit('/').next()?;
    (tag.starts_with('v') && tag.len() > 1).then(|| tag.to_string())
}

/// 比较形如 `v0.9.23` 与 `0.9.22` 的版本号；解析失败按有新版本处理，确保不会跳过更新。
fn is_newer_tag(latest: &str, current: &str) -> bool {
    match (parse_version(latest), parse_version(current)) {
        (Some(l), Some(c)) => l > c,
        _ => true,
    }
}

/// 去掉 `v` 前缀后按 `.` 拆分为数字段，任一段非数字即视为无法解析。
fn parse_version(tag: &str) -> Option<Vec<u64>> {
    let trimmed = tag.trim();
    let body = trimmed.strip_prefix('v').unwrap_or(trimmed);
    body.split('.')
        .map(|part| part.parse::<u64>().ok())
        .collect()
}

/// 尽力删除自身旁边遗留的 `<exe>.old` 备份（Windows 上运行中的 exe 无法立即删除）。
pub(crate) fn cleanup_stale_backup() {
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let Some(name) = exe.file_name().and_then(|n| n.to_str()) else {
        return;
    };
    let _ = std::fs::remove_file(exe.with_file_name(format!("{name}.old")));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_update_command_uses_powershell() {
        let (program, args) = build_update_command(true);
        assert_eq!(program, "powershell");
        assert!(args.contains(&"-Command".to_string()));
        let script = args.last().unwrap();
        assert!(script.contains("irm"));
        assert!(script.contains(&format!("'{INSTALL_PS1_URL}'")));
        assert!(script.contains("iex"));
    }

    #[test]
    fn unix_update_command_uses_bash_curl() {
        let (program, args) = build_update_command(false);
        assert_eq!(program, "bash");
        assert_eq!(args[0], "-c");
        let script = &args[1];
        assert!(script.contains("curl -fsSL"));
        assert!(script.contains(INSTALL_SH_URL));
        assert!(script.contains("| bash"));
    }

    #[test]
    fn newer_tag_compares_numerically() {
        assert!(is_newer_tag("v0.9.23", "0.9.22"));
        assert!(is_newer_tag("0.10.0", "0.9.9"));
        assert!(!is_newer_tag("v0.9.22", "0.9.22"));
        assert!(!is_newer_tag("v0.9.21", "0.9.22"));
    }

    #[test]
    fn unparsable_tag_is_treated_as_newer() {
        assert!(is_newer_tag("latest", "0.9.22"));
    }

    #[test]
    fn tag_from_location_extracts_trailing_tag() {
        assert_eq!(
            tag_from_location("https://github.com/GOLDhjy/GoldBot/releases/tag/v0.10.5"),
            Some("v0.10.5".to_string())
        );
        // 尾部斜杠也能处理
        assert_eq!(
            tag_from_location("https://github.com/GOLDhjy/GoldBot/releases/tag/v0.10.5/"),
            Some("v0.10.5".to_string())
        );
        // 非 tag 形式的地址返回 None
        assert_eq!(
            tag_from_location("https://github.com/GOLDhjy/GoldBot"),
            None
        );
        assert_eq!(tag_from_location("https://github.com/"), None);
    }
}
