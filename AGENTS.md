# 仓库指南

## 项目结构与模块组织
`GoldBot` 是一个 Rust TUI Agent。核心代码位于 `src/`：
- `src/main.rs`: 应用入口、终端生命周期、事件循环。
- `src/agent/`: ReAct 主流程、子代理 DAG、计划、角色与各类 provider 集成。
- `src/tools/`: shell 执行、命令分类、安全检查、搜索、skills 与 MCP 支持。
- `src/ui/`: TUI 渲染、输入处理、格式化与 GE 模式。
- `src/memory/`: 记忆持久化、压缩与检索。
- `src/consensus/`: 多代理编排、评估、审计与外部接口。
- `src/types.rs`: 跨模块共享的枚举和类型。

辅助目录与文件：
- `scripts/`: 安装或辅助脚本。
- `builtin-commands/`、`builtin-skills/`、`myskills/`: 命令和技能资源。
- `.github/`: CI、发布与自动化配置。
- `README.md`、`README_EN.md`、`CLAUDE.md`、`CONSENSUS.md`: 文档与协作说明。

运行时产物通常写入 `memory/` 和 `MEMORY.md`。构建输出位于 `target/`，不应提交。

## 构建、测试和开发命令
- `cargo run`: 本地启动 TUI。
- `GOLDBOT_USE_CODEX=1 GOLDBOT_TASK="整理当前目录的大文件" cargo run`: 使用 Codex provider 和预设任务运行。
- `cargo check`: 快速编译验证，不生成最终二进制。
- `cargo test`: 运行全部测试。
- `cargo test <test_name>`: 运行匹配名称模式的测试，例如 `cargo test rm_requires_confirmation`。
- `cargo test -- <test_name>`: 运行精确名称测试。
- `cargo test <module>::tests::<test_name>`: 运行模块内特定测试。
- `cargo test -- --nocapture`: 显示测试中的 `println!` 输出。
- `cargo clippy --all-targets --all-features`: 检查正确性与风格问题。
- `cargo fmt`: 提交前格式化代码。

## 代码风格与格式化
使用 Rust 2024 版本和 `rustfmt` 默认格式（4 空格缩进）。提交前运行 `cargo fmt`。保持合理行宽，优先拆分长行，而不是调整 `rustfmt` 配置。

## 导入
按逻辑分组导入：标准库优先，其次外部 crate，最后是当前 crate 模块。同一模块的多个导入使用 `{}`：

```rust
use std::{collections::HashMap, path::PathBuf};
use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use crate::types::LlmAction;
```

共享类型优先从 `crate::types` 引入，而不是在中间模块重复导出。

## 命名约定
- 模块、文件、函数和局部变量使用 `snake_case`。
- 结构体、枚举和 trait 使用 `PascalCase`。
- 常量使用 `SCREAMING_SNAKE_CASE`，例如 `MAX_OUTPUT_CHARS`。
- 私有辅助函数使用描述性的 `snake_case` 名称，例如 `extract_last_tag`、`normalize_note`。

## 类型
跨多个模块共享的领域类型放在 `src/types.rs`。需要复制的类型优先使用 `#[derive(Debug, Clone)]`；体量小、语义明确的枚举可进一步派生 `Copy`。仅在需要暴露接口时使用 `pub`，尽量保持实现细节私有。

多变体枚举的 `match` 应尽量覆盖完整分支。涉及 JSON 序列化时，优先使用 `serde` 派生：

```rust
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Message {
    pub role: Role,
    pub content: String,
}
```

## 错误处理
使用 `anyhow::Result<T>` 作为可失败函数的返回类型，并在错误边界通过 `.context()` 补充上下文：

```rust
let content = fs::read_to_string(&path)
    .context("failed to read config file")?;
```

对预期错误使用 `anyhow!()` 构造清晰、可操作的错误信息。不要在错误消息中输出机密、token 或原始敏感输入。优先使用 `?` 提前返回；对于正常的缺省分支，可考虑使用 `Option` 而不是过度包装为 `Result`。

## 异步与并发
异步逻辑基于 `tokio`。异步函数显式标记为 `async fn`。涉及多任务协作时，优先使用结构化并发，避免难以追踪的后台任务泄漏。

## 测试指南
单元测试优先放在源文件底部的 `#[cfg(test)] mod tests` 中，测试名应描述行为：

```rust
#[test]
fn rm_requires_confirmation() {
    let (risk, _) = assess_command("rm README.md");
    assert_eq!(risk, RiskLevel::Confirm);
}
```

对解析器、安全策略、记忆存储和 provider 逻辑同时覆盖成功路径和失败路径。需要临时目录时，优先复用现有辅助函数（例如 `memory/store.rs` 中的 `unique_base()`）来隔离测试环境，并在测试后清理资源。

当前仓库没有固定的 `tests/` 集成测试目录；新增跨模块行为测试时再创建 `tests/`，并保持场景聚焦、命名清晰。

## 常量与配置
模块级常量放在文件顶部，使用 `SCREAMING_SNAKE_CASE`：

```rust
const MAX_OUTPUT_CHARS: usize = 10_000;
const DEFAULT_CMD_TIMEOUT_SECS: u64 = 120;
```

环境变量使用 `std::env::var()` 读取，并通过 `.unwrap_or_else()` 或等价方式提供合理默认值。

## 提交与拉取请求指南
提交信息遵循 Conventional Commit 风格，例如 `feat: ...`、`fix: ...`。单个提交尽量只做一件事。PR 应包含：
- 清晰的摘要和动机。
- 验证结果，例如 `cargo test`、`cargo clippy`。
- 涉及 TUI 渲染变更时的终端截图或 GIF。
- 关联的问题或任务，以及必要的环境/配置更新说明，例如 `GOLDBOT_USE_CODEX`、`API_TIMEOUT_MS`。

## 代码排版规则

- 不要把函数竖着写，比如。除非名字特别长
  pub fn build_sub_agent_prompt(
    custom_prompt: Option<&str>,
    role: Option<&BuiltinRole>,
    base_prompt: &str,)
- 注释要写中文