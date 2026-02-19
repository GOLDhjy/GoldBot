use std::process::Command;

use anyhow::{Context, Result, anyhow};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct CodexPlan {
    pub steps: Vec<CodexStep>,
    pub final_summary: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CodexStep {
    pub thought: String,
    pub command: String,
}

pub struct CodexProvider;

impl CodexProvider {
    pub fn build_plan(task: &str, max_steps: usize) -> Result<CodexPlan> {
        let prompt = format!(
            "你是一个本地自动化助手规划器。\n请根据用户任务生成命令执行计划。\n\
             输出必须是 JSON 且只能是 JSON，结构:\n\
             {{\"steps\":[{{\"thought\":\"...\",\"command\":\"...\"}}],\"final_summary\":\"...\"}}\n\
             约束:\n\
             1) steps 最多 {max_steps} 条\n\
             2) 命令必须跨平台可替换，优先只给当前平台可运行的一套\n\
             3) 不要包含 markdown 代码块\n\
             用户任务: {task}"
        );

        let output = Command::new("codex")
            .args(["exec", &prompt])
            .output()
            .context("failed to run codex CLI")?;

        if !output.status.success() {
            return Err(anyhow!(
                "codex exec failed: {}",
                String::from_utf8_lossy(&output.stderr)
            ));
        }

        let raw = String::from_utf8_lossy(&output.stdout).to_string();
        let json = extract_json_object(&raw)
            .ok_or_else(|| anyhow!("codex output did not contain JSON object"))?;
        let plan: CodexPlan = serde_json::from_str(json).context("invalid JSON from codex")?;

        if plan.steps.is_empty() {
            return Err(anyhow!("codex returned empty steps"));
        }

        Ok(plan)
    }
}

fn extract_json_object(text: &str) -> Option<&str> {
    let start = text.find('{')?;
    let end = text.rfind('}')?;
    (end > start).then_some(&text[start..=end])
}
