# GoldBot - AI 终端自动化助手

一个基于 Rust 开发的跨平台 TUI Agent，通过 LLM 自动规划和执行 Shell 命令来完成任务。

## 特性

- 三级安全控制：Safe/Confirm/Block
- 持久化记忆：短期按日期存储，长期自动提取偏好，每次请求随 System Prompt 发送
- ReAct 循环：思考-执行-观察-再思考
- 智能命令分类：Read/Write/Update/Search/Bash
- 实时 TUI：流式显示思考过程，完成后默认折叠
- 原生 LLM 深度思考：Tab 键切换，控制 API 层 `reasoning_content` 流
- 上下文自动压缩：消息超阈值时自动摘要并截断，防止 token 膨胀
- 跨平台支持：macOS/Linux (bash) / Windows (PowerShell)
- 可选 MCP 工具接入：启动时自动发现并映射为 `mcp_<server>_<tool>`

## 安装

### macOS / Linux（推荐）

**一行命令安装（推荐）**

```bash
curl -fsSL https://raw.githubusercontent.com/GOLDhjy/GoldBot/master/scripts/install.sh | bash
```

**指定版本安装**

```bash
curl -fsSL https://raw.githubusercontent.com/GOLDhjy/GoldBot/master/scripts/install.sh | bash -s -- --version v0.2.0
```

**从源码安装**

```bash
curl -fsSL https://raw.githubusercontent.com/GOLDhjy/GoldBot/master/scripts/install.sh | bash -s -- --source
```

### Windows（PowerShell）

```powershell
irm "https://raw.githubusercontent.com/GOLDhjy/GoldBot/master/scripts/install.ps1" | iex
```

### Homebrew（macOS / Linux）

```bash
brew install GOLDhjy/GoldBot/goldbot
```

### 手动下载（3 平台）

- macOS Intel: `goldbot-v*-macos-x86_64.tar.gz`
- macOS Apple Silicon: `goldbot-v*-macos-aarch64.tar.gz`
- Linux x86_64: `goldbot-v*-linux-x86_64.tar.gz`
- Windows x86_64: `goldbot-v*-windows-x86_64.zip`

### 从源码编译（所有平台）

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
│   │   ├── mcp.rs        # MCP server 配置/发现/调用（stdio）
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

### LLM 接入

GoldBot 通过 BigModel 原生 API（OpenAI 兼容格式）调用 GLM-4.7：

- **端点**: `BIGMODEL_BASE_URL/chat/completions`（默认 `https://open.bigmodel.cn/api/coding/paas/v4`）
- **鉴权**: `Authorization: Bearer <BIGMODEL_API_KEY>`
- **模型**: `BIGMODEL_MODEL`（默认 `GLM-4.7`）
- **流式响应**: SSE 格式，两类 delta：
  - `content` → 正文回答，驱动 TUI 滚动预览
  - `reasoning_content` → 深度思考内容，状态栏实时显示

深度思考（Thinking）由 API 参数 `{"thinking": {"type": "enabled"|"disabled"}}` 控制，Tab 键切换，默认开启。GLM 平台对系统消息自动做前缀缓存，重复 token 不额外计费。

### 主事件循环 (main.rs)

```text
loop {
    1. 处理 LLM 流式响应（reasoning_content → 状态栏预览，content → 累积正文）
    2. 触发 Agent 步骤（异步调用 LLM API）
    3. 处理键盘输入（Ctrl+C/D, Tab, ↑/↓/Enter）
}
```

### ReAct 循环流程

```text
用户输入任务
  → start_task() (重置状态, 设置 needs_agent_step=true)
  → maybe_flush_and_compact_before_call() (消息数超阈值时压缩)
  → LLM 调用 (发送 System Prompt + 历史消息)
  → process_llm_result() (解析 <thought> 和 <tool>/<final>)

  分支:
  ├─ <tool>shell → execute_command()
  │      ├─ Safe   → 直接执行
  │      ├─ Confirm → 弹出确认菜单
  │      └─ Block  → 拒绝（返回错误给 LLM）
  │           → 将结果加入历史 → needs_agent_step=true (循环)
  │
  └─ <final> → finish()
       → 保存记忆 → 折叠显示 → running=false (退出)
```

### 安全评估

```text
Block:   sudo, format, fork bomb (:(){:|:&};:)
Confirm: rm, mv, cp, git commit, curl, wget, >, >>
Safe:    ls, cat, grep, git status, 只读操作
```

### 记忆机制

- **短期记忆**: `~/.goldbot/memory/YYYY-MM-DD.md`（每日日志，任务完成后追加）
- **长期记忆**: `~/.goldbot/MEMORY.md`（偏好/规则，自动去重提取）
- **启动注入**: `App::new()` 时读取长期记忆（最近 30 条）+ 最近 2 天短期记忆，嵌入 System Prompt，**每次请求都随 messages[0] 一并发送**（GLM 自动缓存重复前缀）
- **上下文压缩**: 消息数超过 48 条时，将旧消息摘要为 `[Context compacted]` 块，保留最近 18 条，防止 token 无限增长

### 环境变量

| 变量 | 必填 | 默认值 | 说明 |
|---|---|---|---|
| `BIGMODEL_API_KEY` | ✅ | — | BigModel API 密钥 |
| `BIGMODEL_BASE_URL` | 否 | `https://open.bigmodel.cn/api/coding/paas/v4` | API 基础 URL |
| `BIGMODEL_MODEL` | 否 | `GLM-4.7` | 模型名称 |
| `HTTP_PROXY` | 否 | — | HTTP 代理 |
| `API_TIMEOUT_MS` | 否 | — | 请求超时（毫秒） |
| `GOLDBOT_TASK` | 否 | — | 启动时直接执行的任务 |
| `GOLDBOT_MCP_SERVERS` | 否 | — | MCP 服务器配置 JSON |
| `GOLDBOT_MCP_SERVERS_FILE` | 否 | `~/.goldbot/mcp_servers.json` | MCP 配置文件路径（仅在未设置 `GOLDBOT_MCP_SERVERS` 时生效） |

推荐在项目根目录创建 `.env` 文件（启动时自动加载）：

```env
BIGMODEL_API_KEY=your_api_key_here
BIGMODEL_BASE_URL=https://open.bigmodel.cn/api/coding/paas/v4
BIGMODEL_MODEL=GLM-4.7
```

## 使用方法

```bash
goldbot
```

### GE 监督模式（聊天触发）

在输入框里用 `GE` 开头可进入持续监督模式：

- `GE <目标描述>`：进入 GE 模式（若无 `CONSENSUS.md`，会进入固定三问并自动生成）
- `GE`：进入 GE 模式（已有 `CONSENSUS.md` 时直接接管）
- `GE 退出` / `GE exit`：退出 GE 模式
- `GE 细化todo` / `GE replan`：基于当前 Purpose/Rules/Scope 重新生成更细粒度 Todo

GE 模式下：

- 执行链路固定为：Claude 执行 -> Codex 检查优化 -> GoldBot 只读验收
- 三问完成后 Todo 由 LLM 生成（目标 8-12 个细粒度步骤）
- 每个 Todo 验收通过后，GoldBot 会执行自审并在本地创建 git commit
- 自动 commit 会排除 `GE_LOG.jsonl`，避免日志污染代码提交
- 共识文件路径：项目根 `CONSENSUS.md`
- 审计日志路径：项目根 `GE_LOG.jsonl`（JSONL，单文件持续追加）
- 任务触发：每个 Todo 完成后立即重读 + 每 30 分钟周期重读

## MCP 接入（对齐 OpenCode 风格）

默认从记忆目录读取配置文件：

- macOS / Linux: `~/.goldbot/mcp_servers.json`
- 若设置 `GOLDBOT_MEMORY_DIR`，则读取 `$GOLDBOT_MEMORY_DIR/mcp_servers.json`

文件内容为 JSON（`server_name -> config`）：

```json
{
  "context7": {
    "type": "local",
    "command": "npx",
    "args": ["-y", "@upstash/context7-mcp"],
    "enabled": true
  }
}
```

然后直接启动：

```bash
goldbot
```

也可以用环境变量 `GOLDBOT_MCP_SERVERS` 临时覆盖：

```bash
export GOLDBOT_MCP_SERVERS='{
  "context7": {
    "type": "local",
    "command": "npx",
    "args": ["-y", "@upstash/context7-mcp"],
    "enabled": true
  }
}'
goldbot
```

说明：
- 当前支持 `local`（stdio）类型，字段与 OpenCode 常用配置对齐：`type/command/args/env/cwd/enabled`。
- 启动时会执行 MCP `tools/list` 自动发现工具，并在系统提示词中暴露为 `mcp_<server>_<tool>`。
- LLM 调用格式：
  - `<tool>mcp_...</tool>`
  - `<arguments>{"key":"value"}</arguments>`

### MCP 配置字段

| 字段 | 必填 | 默认值 | 说明 |
|---|---|---|---|
| `type` | 否 | `local` | 目前仅支持 `local`（stdio） |
| `command` | 是 | — | 启动 MCP server 的命令 |
| `args` | 否 | `[]` | 命令参数 |
| `env` | 否 | `{}` | 传给 MCP server 的环境变量 |
| `cwd` | 否 | 当前目录 | MCP server 工作目录 |
| `enabled` | 否 | `true` | `false` 时跳过该 server |

### 多服务示例

```bash
export GOLDBOT_MCP_SERVERS='{
  "context7": {
    "type": "local",
    "command": "npx",
    "args": ["-y", "@upstash/context7-mcp"],
    "enabled": true
  },
  "filesystem": {
    "type": "local",
    "command": "npx",
    "args": ["-y", "@modelcontextprotocol/server-filesystem", "/path/to/your/project"],
    "enabled": true
  }
}'
```

### 从 OpenCode / OpenClawd 迁移

- 如果你已有同结构 `server_name -> config`，可直接赋值给 `GOLDBOT_MCP_SERVERS`。
- 如果你的配置是下面这种包装结构：

```json
{
  "mcpServers": {
    "context7": {
      "command": "npx",
      "args": ["-y", "@upstash/context7-mcp"]
    }
  }
}
```

只需要把 `mcpServers` 里的对象拿出来赋给 `GOLDBOT_MCP_SERVERS`：

```bash
export GOLDBOT_MCP_SERVERS='{
  "context7": {
    "command": "npx",
    "args": ["-y", "@upstash/context7-mcp"]
  }
}'
```

### 常见问题

- 启动时出现 `Failed to load MCP tools...`：
  - 先确认 `command/args` 本地可执行，再确认该 server 支持 stdio MCP。
- 调用时报 `unknown MCP tool`：
  - 说明本次启动没有发现该工具，重启并检查 `enabled`、依赖安装和 `tools/list` 能否返回。
- 传参报错：
  - `<arguments>` 必须是 JSON 对象（`{}`），不能是数组或纯文本。

### 按键

| 按键 | 场景 | 说明 |
|---|---|---|
| `Ctrl+C` | 任意 | 退出 |
| `Ctrl+D` | 任务完成后 | 折叠/展开详情 |
| `Tab` | 非确认菜单 | 切换深度思考 ON/OFF |
| `↑/↓` | 确认菜单 | 移动选项 |
| `Enter` | 确认菜单 | 确认选项 |
| `Esc` | 输入中 | 失焦 / 返回菜单 |

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
