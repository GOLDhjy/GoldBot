# GoldBot (TUI MVP)

跨平台 TUI Agent 原型：单次进入后按计划循环执行命令，展示过程，并在结束后默认折叠只显示最终结果。

## 特性
- `run_command` 统一工具接口
  - macOS/Linux: `bash -lc`
  - Windows: `powershell -NoProfile -Command`
- 风险控制：高风险命令弹出**可上下选择**菜单（非文本输入）
- 过程可见：`Thinking / ToolCall / ToolResult / NeedsConfirmation / Final`
- 完成后默认折叠，仅显示最终总结；按 `d` 展开详情

## 运行
```bash
cargo run
```

## 按键
- `q` 退出
- `d` 折叠/展开详情（任务完成后）
- `↑/↓` 移动确认菜单
- `Enter` 确认菜单选项

## 说明
当前是 MVP，LLM 决策为 deterministic planner（无需 API Key），后续可替换为真实 LLM + tools 循环。


## 接入 Codex Provider（已实现）
默认仍走内置示例 planner。若要让程序启动时通过本地 Codex 生成计划：

```bash
GOLDBOT_USE_CODEX=1 GOLDBOT_TASK="整理当前目录的大文件" cargo run
```

说明：
- 通过 `codex exec` 生成 JSON 计划（provider 文件：`src/agent/provider.rs`）
- 若 Codex 不可用或返回异常，自动回退到示例 planner
