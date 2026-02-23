# 仓库指南

## 项目结构与模块组织
`GoldBot` 是一个 Rust TUI 代理。核心代码位于 `src/`：
- `src/main.rs`: 应用入口、终端生命周期、事件循环。
- `src/agent/`: LLM 响应解析 (`react.rs`) 和提供程序/HTTP 集成 (`provider.rs`)。
- `src/tools/`: shell 执行、命令分类和安全检查。
- `src/ui/`: UI 渲染、格式化和 GE (GoldBot Enhancer) 模式。
- `src/memory/`: 内存持久化、压缩和检索。
- `src/consensus/`: 多代理编排、评估和外部 API。
- `src/types.rs`: 跨模块共享的枚举/类型。

运行时产物存储在 `memory/`（如 `memory/2026-02-20.md` 的每日日志）和 `MEMORY.md`（长期笔记）中。构建输出位于 `target/`，不应提交。

## 构建、测试和开发命令
- `cargo run`: 本地启动 TUI。
- `GOLDBOT_USE_CODEX=1 GOLDBOT_TASK="整理当前目录的大文件" cargo run`: 使用 Codex 提供程序和预设任务运行。
- `cargo check`: 快速编译验证，不构建二进制文件。
- `cargo test`: 运行所有测试。
- `cargo test <test_name>`: 运行匹配名称模式的测试（如 `cargo test rm_requires_confirmation`）。
- `cargo test -- <test_name>`: 运行指定名称的精确测试。
- `cargo test <module>::tests::<test_name>`: 运行模块中的特定测试。
- `cargo test -- --nocapture`: 在测试中显示 `println!` 的输出。
- `cargo clippy --all-targets --all-features`: 检查正确性/风格问题。
- `cargo fmt`: 提交前格式化代码。

## 代码风格与格式化
使用 Rust 2024 版本和 `rustfmt` 默认值（4 空格缩进）。提交前运行 `cargo fmt`。保持合理的行长度；优先拆分长行而不是修改 `cargo fmt` 配置。

## 导入
逻辑分组导入：标准库优先，然后是外部 crate，最后是 crate 模块。对同一模块的多个导入使用 `{}`：
```rust
use std::{collections::HashMap, path::PathBuf};
use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};
use crate::types::LlmAction;
```
对于共享类型，优先使用 `use crate::types::` 而不是重新导出。

## 命名约定
- 模块、文件、函数和局部变量使用 `snake_case`。
- 结构体、枚举和 trait 使用 `PascalCase`。
- 常量使用 `SCREAMING_SNAKE_CASE`（如 `MAX_OUTPUT_CHARS: usize = 10_000`）。
- 私有辅助函数使用 `snake_case` 和描述性名称（如 `extract_last_tag`、`normalize_note`）。

## 类型
当类型在多个模块中使用时，在 `src/types.rs` 中定义领域类型。对于需要复制的类型，使用 `#[derive(Debug, Clone)]`。小枚举优先使用 `Copy`。选择性使用 `pub`；保持实现细节私有。

多变体枚举应使用 match 表达式全面覆盖。对于 JSON 序列化，使用 `serde` 派生宏：
```rust
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Message {
    pub role: Role,
    pub content: String,
}
```

## 错误处理
使用 `anyhow::Result<T>` 作为可能失败的函数的返回类型。在错误边界使用 `.context()` 添加上下文：
```rust
let content = fs::read_to_string(&path)
    .context("failed to read config file")?;
```
对于预期错误（如解析失败），使用 `anyhow!()` 宏和描述性消息。永远不要在错误消息中记录机密或原始输入。

使用 `?` 操作符提前返回。对于可恢复错误，考虑使用 `Option` 和提前返回而不是 Result。

## 异步与并发
使用 `tokio` 运行时进行异步操作。异步函数应显式标记为 `async fn`。生成任务时，优先使用结构化并发模式。

## 测试指南
在文件底部使用 `#[cfg(test)] mod tests` 块内联放置单元测试。使用 `assert_eq!` 和 `assert!` 宏按行为描述性地命名测试：
```rust
#[test]
fn rm_requires_confirmation() {
    let (risk, _) = assess_command("rm README.md");
    assert_eq!(risk, RiskLevel::Confirm);
}
```
对于解析器/安全/内存代码，测试成功和失败情况。使用如 `memory/store.rs` 中的 `unique_base()` 等辅助函数创建隔离的测试目录。使用后清理测试资源。

多模块行为的集成测试放在 `tests/` 目录中（目前最少；需要时添加）。

## 常量与配置
在顶部使用 `SCREAMING_SNAKE_CASE` 定义模块级常量：
```rust
const MAX_OUTPUT_CHARS: usize = 10_000;
const DEFAULT_CMD_TIMEOUT_SECS: u64 = 120;
```
环境变量应使用 `std::env::var()` 读取，并通过 `.unwrap_or_else()` 提供合理的默认值。

## 提交与拉取请求指南
遵循历史记录中使用的 Conventional Commit 风格（`feat: ...`、`fix: ...`）。保持每个提交只涉及一个更改。PR 应包括：
- 清晰的摘要和动机，
- 验证输出（`cargo test`、`cargo clippy`），
- TUI 渲染更改的终端截图/GIF，
- 链接的问题/任务以及任何环境/配置更新（例如 `GOLDBOT_USE_CODEX`、`API_TIMEOUT_MS`）。
