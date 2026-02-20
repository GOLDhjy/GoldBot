# GoldBot - AI 终端自动化助手

一个基于 Rust 开发的跨平台 TUI Agent，通过 LLM 自动规划和执行 Shell 命令来完成任务。

## 特性

- 三级安全控制：Safe/Confirm/Block
- 持久化记忆：短期按日期存储，长期自动提取偏好
- ReAct 循环：思考-执行-观察-再思考
- 智能命令分类：Read/Write/Update/Search/Bash
- 实时 TUI：流式显示思考过程，完成后默认折叠
- 跨平台支持：macOS/Linux (bash) / Windows (PowerShell)

## 安装

### 一行命令安装（推荐）

**macOS / Linux**

```bash
curl -fsSL https://raw.githubusercontent.com/GOLDhjy/GoldBot/main/scripts/install.sh | bash
```

或使用 Homebrew：

```bash
brew install GOLDhjy/GoldBot/goldbot
```

### 从源码编译

```bash
cargo install --git https://github.com/GOLDhjy/GoldBot.git
```

## 项目构成

```
GoldBot/
├── src/
│   ├── main.rs           # 程序入口 + 主事件循环
│   ├── agent/
│   │   ├── react.rs      # ReAct 框架：系统提示词 + 响应解析
│   │   ├── step.rs       # 核心步骤：start → process → execute → finish
│   │   └── provider.rs   # LLM 接口（HTTP + 流式处理）
│   ├── tools/
│   │   ├── shell.rs      # 命令执行 + 分类
│   │   └── safety.rs     # 风险评估（Safe/Confirm/Block）
│   ├── memory/
│   │   ├── store.rs      # 记忆存储（短期/长期）
│   │   └── compactor.rs  # 上下文压缩
│   ├── ui/
│   │   ├── screen.rs     # TUI 屏幕管理
│   │   └── format.rs     # 事件格式化
│   └── types.rs
├── Cargo.toml
└── README.md
```

## 运行机制

### 主事件循环 (main.rs)

```text
loop {
    1. 处理 LLM 流式响应（实时更新思考预览）
    2. 触发 Agent 步骤（异步调用 LLM API）
    3. 处理键盘输入（Ctrl+C/D, ↑/↓/Enter）
}
```

### ReAct 循环流程

```text
用户输入任务
  → start_task() (重置状态, 设置 needs_agent_step=true)
  → LLM 调用 (发送 System Prompt + 历史消息)
  → process_llm_result() (解析 <thought> 和 <tool>/<final>)
  
  分支:
  ├─ <tool>shell → execute_command()
  │      ├─ Safe → 直接执行
  │      ├─ Confirm → 弹出菜单
  │      └─ Block → 拒绝
  │           → 将结果加入历史 → needs_agent_step=true (循环)
  │
  └─ <final> → finish()
       → 保存记忆 → 折叠显示 → running=false (退出)
```

### 安全评估

```text
Block: sudo, format, fork bomb
Confirm: rm, mv, cp, git commit, curl, wget, >, >>
Safe: ls, cat, grep, git status
```

### 记忆机制

- **短期记忆**: ~/.goldbot/memory/YYYY-MM-DD.md（完整对话）
- **长期记忆**: ~/.goldbot/MEMORY.md（仅偏好/规则，去重）
- **启动注入**: 自动加载长期记忆 + 最近 2 天短期记忆

## 使用方法

```bash
goldbot
```

### 按键

- Ctrl+C: 退出
- Ctrl+D: 折叠/展开详情
- ↑/↓/Enter: 确认菜单选项

### 确认菜单

1. Execute - 执行
2. Skip - 跳过
3. Abort - 终止
4. Note - 添加指令

## 技术栈

- Rust 2024 Edition
- Tokio (异步运行时)
- crossterm (TUI)
- reqwest (HTTP)
- serde_json
