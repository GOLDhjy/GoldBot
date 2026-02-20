# GoldBot (TUI MVP)

> [English Version](README_EN.md) | 简体中文

跨平台 TUI Agent 原型：单次进入后按计划循环执行命令，展示过程，并在结束后默认折叠只显示最终结果。

## 特性

### 统一工具接口
- `run_command` 统一工具接口
  - macOS/Linux: `bash -lc`
  - Windows: `powershell -NoProfile -Command`

### 风险控制（三级评估）
- **Safe（安全）** - 直接执行（如 `ls`、`git status`、`sed -n`）
- **Confirm（需确认）** - 弹出菜单确认（如 `rm`、`git commit`、`sed -i`）
- **Block（拦截）** - 直接阻止（如 `sudo`、`format`、fork bomb）
- 高风险命令弹出**可上下选择**菜单（非文本输入）

### 命令分类系统
命令自动识别并分为 5 种类型：
- **Read** - 只读操作（如 `cat`、`ls`、`sed -n`）
- **Write** - 写入操作（如 `cat > file`）
- **Update** - 更新操作（如 `sed -i`、`rm`）
- **Search** - 搜索操作（如 `rg`、`grep`）
- **Bash** - 其他 shell 命令

### 智能命令识别
- `sed -n` 打印模式被识别为只读，无需确认
- `sed -i` 就地编辑需要确认
- 检测未引用的重定向操作 (`>`, `>>`, `<<`)
- Git 子命令细粒度判断（`add`/`commit` 需确认，`status`/`log` 不需要）

### 过程可见
- 事件类型：`Thinking / ToolCall / ToolResult / NeedsConfirmation / Final`
- 完成后默认折叠，仅显示最终总结
- 按 `d` 展开详情

## 运行
```bash
cargo run
```

## 按键
- `Esc` 退出
- `Ctrl+d` 折叠/展开详情（任务完成后）
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

## 记忆机制
- **短期记忆**：每次任务结束写入 `memory/YYYY-MM-DD.md`
- **长期记忆**：关键结论追加写入 `MEMORY.md`
- **上下文压缩**：按轮数阈值触发，保留压缩摘要 + 最近事件，避免上下文膨胀
