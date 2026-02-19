use std::env;

use super::provider::CodexProvider;

#[derive(Debug, Clone)]
pub struct PlanStep {
    pub thought: String,
    pub command: String,
}

pub fn plan_from_codex_or_sample() -> Vec<PlanStep> {
    let use_codex = env::var("GOLDBOT_USE_CODEX").ok().as_deref() == Some("1");
    if !use_codex {
        return sample_plan();
    }

    let task = env::var("GOLDBOT_TASK").unwrap_or_else(|_| "整理当前目录并汇总文件信息".to_string());
    match CodexProvider::build_plan(&task, 5) {
        Ok(plan) => plan
            .steps
            .into_iter()
            .map(|s| PlanStep {
                thought: s.thought,
                command: s.command,
            })
            .collect(),
        Err(_) => sample_plan(),
    }
}

pub fn sample_plan() -> Vec<PlanStep> {
    if cfg!(target_os = "windows") {
        vec![
            PlanStep { thought: "先看看当前目录".into(), command: "Get-ChildItem".into() },
            PlanStep { thought: "统计文件数量".into(), command: "(Get-ChildItem -Recurse -File | Measure-Object).Count".into() },
            PlanStep { thought: "创建工作目录（风险示例）".into(), command: "New-Item -ItemType Directory -Path .\\goldbot_temp -Force".into() },
        ]
    } else {
        vec![
            PlanStep { thought: "先看看当前目录".into(), command: "ls -la".into() },
            PlanStep { thought: "统计文件数量".into(), command: "find . -type f | wc -l".into() },
            PlanStep { thought: "创建工作目录（风险示例）".into(), command: "mkdir -p ./goldbot_temp".into() },
        ]
    }
}
