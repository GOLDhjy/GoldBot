# GoldBot - AI 终端自动化助手

[![GitHub Release](https://img.shields.io/github/v/release/GOLDhjy/GoldBot?display_name=tag&style=flat-square)](https://github.com/GOLDhjy/GoldBot/releases) [![GitHub Actions](https://img.shields.io/github/actions/workflow/status/GOLDhjy/GoldBot/release.yml?style=flat-square)](https://github.com/GOLDhjy/GoldBot/actions/workflows/release.yml) [![GitHub Stars](https://img.shields.io/github/stars/GOLDhjy/GoldBot?style=flat-square)](https://github.com/GOLDhjy/GoldBot/stargazers) [![License](https://img.shields.io/github/license/GOLDhjy/GoldBot?style=flat-square)](https://github.com/GOLDhjy/GoldBot/blob/main/LICENSE)

一个基于 Rust 开发的跨平台 TUI Agent，通过 LLM 自动规划和执行 Shell 命令来完成任务。

[English Version](README_EN.md)

## 特性

- ReAct 循环：思考-执行-观察-再思考，支持 shell / plan / question / web_search / MCP / SubAgent 多种动作
- SubAgent 子代理：DAG 任务图调度，拓扑排序自动并行/串行，依赖节点输出自动合并，支持 role 角色预设（search/coding/analysis/writer/reviewer）
- 三级安全控制：Safe/Confirm/Block，heredoc 内容不误判
- 文件变更 diff：命令执行后自动对比前后内容，行号级红绿高亮显示
- 实时 TUI：流式显示思考过程，完成后默认折叠
- 原生 LLM 深度思考：Tab 键切换，控制 API 层 `reasoning_content` 流
- 上下文自动压缩：消息超阈值时自动摘要并截断，防止 token 膨胀
- 持久化记忆：短期按日期存储，长期自动提取偏好，每次请求随 System Prompt 发送
- MCP 工具接入：启动时自动发现并映射为 `mcp_<server>_<tool>`
- 联网搜索：接入 Bocha AI，LLM 可主动检索实时信息
- Skills 系统：启动时自动发现 `~/.goldbot/skills/` 下的技能，注入系统提示词
- **@ 文件附加**：输入框中键入 `@` 实时搜索文件，选中后自动附加到消息发送给 LLM
- **Slash 命令**：`/` 打开命令选择器，内置 8 个实用命令，支持用户自定义扩展
- GE 监督模式：多模型协作（Claude 执行 → Codex 优化 → GoldBot 验收），自动 git commit
- 跨平台：macOS/Linux (bash) / Windows (PowerShell)

## 安装

### macOS / Linux

```bash
# 一行安装（推荐）
curl -fsSL https://raw.githubusercontent.com/GOLDhjy/GoldBot/master/scripts/install.sh | bash

# 从源码安装
curl -fsSL https://raw.githubusercontent.com/GOLDhjy/GoldBot/master/scripts/install.sh | bash -s -- --source
```

### Windows（PowerShell）

```powershell
irm "https://raw.githubusercontent.com/GOLDhjy/GoldBot/master/scripts/install.ps1" | iex
```

### 手动下载

- macOS Intel: `goldbot-v*-macos-x86_64.tar.gz`
- macOS Apple Silicon: `goldbot-v*-macos-aarch64.tar.gz`
- Linux x86_64: `goldbot-v*-linux-x86_64.tar.gz`
- Windows x86_64: `goldbot-v*-windows-x86_64.zip`

### 从源码编译

```bash
cargo install --git https://github.com/GOLDhjy/GoldBot.git
```

## 使用方法

```bash
goldbot
```

### 命令行参数

| 参数 | 说明 |
|---|---|
| `-p <消息>` / `--prompt <消息>` | 启动时直接发送一条聊天消息，无需手动输入 |
| `-y` / `--yes` | 自动接受所有 Confirm 级命令，无需手动确认（Block 级命令仍会被拦截） |

```bash
# 启动后自动发送消息（仍需手动确认 risky 命令）
goldbot -p "整理当前目录的大文件"

# 交互模式，但自动接受所有 Confirm 命令
goldbot -y

# 完全自动化：自动发消息 + 自动接受命令
goldbot -p "整理当前目录的大文件" -y
```

### 按键

| 按键 | 场景 | 说明 |
|---|---|---|
| `Ctrl+C` | 任意 | 退出 |
| `Ctrl+D` | 任务完成后 | 折叠/展开详情 |
| `Tab` | 非菜单模式 | 切换深度思考 ON/OFF |
| `Shift+Tab` | 非菜单模式 | 循环切换协助模式（agent / accept edits / plan） |
| `@` | 输入框为空时 | 打开文件搜索选择器 |
| `/` | 输入框为空时 | 打开 slash 命令选择器 |
| `↑/↓` | 菜单/选择器模式 | 移动选项 |
| `Enter` / `Tab` | 选择器模式 | 确认选中项 |
| 直接输入字符 | question 菜单 | 进入自定义输入模式 |
| `Esc` | 输入中 | 失焦 / 取消选择器 / 返回菜单 |

### 确认菜单（risky 命令）

执行 Confirm 级命令时弹出：

1. Execute - 执行
2. Skip - 跳过
3. Abort - 终止
4. Note - 添加补充指令

### Question 菜单（LLM 提问）

LLM 使用 `question` 工具时弹出，选项由 LLM 决定。最后一项通常为 `✏ 我来说...`，选中或直接输入字符可进入自由文本模式。

## @ 文件附加

在输入框为空时键入 `@`，弹出文件搜索面板：

- 继续输入字符实时过滤文件名（大小写不敏感）
- `↑/↓` 选择候选，`Enter` 或 `Tab` 确认附加
- `Esc` 或退格删除 `@` 取消选择器
- 可附加多个文件，选中后以 `@path/to/file` 形式嵌入输入框
- 提交时自动将文件绝对路径追加到消息，LLM 可据此读取文件内容

搜索范围为当前 workspace，自动跳过 `.git`、`target`、`node_modules` 等目录。

## Slash 命令

在输入框为空时键入 `/`，弹出命令选择器，输入字符实时过滤，`↑/↓` 导航，`Enter` 执行。

### 内置命令

| 命令 | 说明 |
|---|---|
| `/help` | 显示键位绑定和可用命令列表 |
| `/clear` | 清除会话历史，重新开始对话 |
| `/compact` | 立即截断上下文，保留最近 18 条消息 |
| `/memory` | 查看当前长期和短期记忆内容 |
| `/thinking` | 切换原生 Thinking 模式（同 Tab 键） |
| `/skills` | 列出所有已发现的 Skill |
| `/mcp` | 列出所有已注册的 MCP 工具及状态 |
| `/status` | 显示 workspace、模型、Thinking 状态等配置摘要 |

### 用户自定义命令

在 `~/.goldbot/command/<name>/COMMAND.md` 创建命令文件：

```markdown
---
name: my-cmd
description: 命令说明
---

模板内容，选中后填入输入框供用户编辑后提交
```

目录名必须与 `name` 字段一致，`name` 只允许字母、数字和连字符。

## GE 黄金体验

GE（黄金体验）是面向开发任务的持续监督模式，执行链路固定为：**Claude 执行 → Codex 检查优化 → GoldBot 只读验收**，每个 Todo 验收通过后自动创建 git commit。

### 触发方式

| 命令 | 说明 |
|---|---|
| `GE <任务描述>` | 进入 GE 模式，无 `CONSENSUS.md` 时触发三问 |
| `GE` | 进入 GE 模式，已有 `CONSENSUS.md` 时直接加载 |
| `GE replan` | 基于当前共识重新生成 Todo 计划 |
| `GE exit` | 退出 GE 模式 |

### Interview 阶段

首次进入或无 `CONSENSUS.md` 时，依次询问三个问题：

1. **Purpose** - 本次开发的核心目标
2. **Rules** - 开发规范、技术栈、测试要求等
3. **Scope** - 任务边界，明确做什么/不做什么

完成后自动生成项目根目录的 `CONSENSUS.md`。

### Todo 计划

共识确立后，LLM 生成 8-12 个细粒度步骤，每步状态为 Pending → Running → Done，在 TUI 侧边栏实时显示。

### 审计日志

所有 GE 操作追加记录到项目根目录 `GE_LOG.jsonl`，自动 commit 时排除该文件。

## MCP 接入

配置文件默认路径：`~/.goldbot/mcp_servers.json`，格式与 OpenCode 兼容，可直接复用已有配置。

```json
{
  "mcp": {
    "context7": {
      "type": "local",
      "command": ["npx", "-y", "@upstash/context7-mcp"],
      "enabled": true
    }
  }
}
```

也可通过环境变量 `GOLDBOT_MCP_SERVERS` 临时覆盖（JSON 字符串，格式同上）。

启动时自动执行 `tools/list` 发现工具，并在系统提示词中暴露为 `mcp_<server>_<tool>`。

### 配置字段

| 字段 | 必填 | 默认值 | 说明 |
|---|---|---|---|
| `type` | 否 | `local` | 目前仅支持 `local`（stdio） |
| `command` | 是 | — | 启动命令及参数数组 |
| `env` | 否 | `{}` | 传给 server 的环境变量 |
| `cwd` | 否 | 当前目录 | server 工作目录 |
| `enabled` | 否 | `true` | `false` 时跳过 |

### 常见问题

- `Failed to load MCP tools...`：确认 `command` 数组中的命令本地可执行，且 server 支持 stdio MCP
- `unknown MCP tool`：重启并检查 `enabled` 及依赖安装
- 传参报错：`<arguments>` 必须是 JSON 对象，不能是数组或纯文本

## Skills

在 `~/.goldbot/skills/<name>/SKILL.md` 创建技能文件，启动时自动发现并注入系统提示词。

```markdown
---
name: pdf
description: 整理和处理 PDF 文件
---

# 技能内容（自由格式 Markdown）
```

也可以直接让 GoldBot 帮你创建，或自动从 Claude / Codex 获取：

```
帮我创建一个 skill，用于整理 PDF 文件
```

## 环境变量

配置文件统一放在 `~/.goldbot/.env`，首次启动若不存在会自动从模板创建。

| 变量 | 必填 | 默认值 | 说明 |
|---|---|---|---|
| `BIGMODEL_API_KEY` | 否 | — | BigModel API 密钥 |
| `KIMI_API_KEY` | 否 | — | Kimi API 密钥 |
| `MIMO_API_KEY` | 否 | — | Xiaomi MiMo API 密钥 |
| `MINIMAX_API_KEY` | 否 | — | MiniMax API 密钥 |
| `LLM_PROVIDER` | 否 | 自动检测 | 显式指定 `glm`、`kimi`、`mimo`、`minimax` |
| `BIGMODEL_BASE_URL` | 否 | `https://open.bigmodel.cn/api/coding/paas/v4` | API 基础 URL |
| `BIGMODEL_MODEL` | 否 | `glm-5` | 模型名称，支持 `GLM-4.7`、`glm-5`、`glm-5.1` |
| `KIMI_BASE_URL` | 否 | `https://api.kimi.com/coding/v1` 或 `https://api.moonshot.cn/v1` | Kimi API 基础 URL |
| `KIMI_MODEL` | 否 | `kimi-for-coding` 或 `kimi-k2.5` | Kimi 模型名称 |
| `MIMO_BASE_URL` | 否 | `https://api.xiaomimimo.com/v1` | Xiaomi MiMo 普通 chat API 基础 URL |
| `MIMO_MODEL` | 否 | `mimo-v2-pro` | Xiaomi MiMo 模型名称，支持 `mimo-v2-pro`、`mimo-v2-flash`、`mimo-v2-omni` |
| `MIMO_CONTEXT_WINDOW_TOKENS` | 否 | `256000` | MiMo 上下文预算估算值 |
| `MINIMAX_BASE_URL` | 否 | `https://api.minimaxi.com/v1` | MiniMax API 基础 URL |
| `MINIMAX_MODEL` | 否 | `MiniMax-M2.5` | MiniMax 模型名称 |
| `BOCHA_API_KEY` | 否 | — | Bocha AI 搜索密钥 |
| `GOLDBOT_TASK` | 否 | — | 启动时直接执行的任务 |
| `GOLDBOT_MCP_SERVERS` | 否 | — | MCP 配置 JSON（覆盖文件） |
| `GOLDBOT_MCP_SERVERS_FILE` | 否 | `~/.goldbot/mcp_servers.json` | MCP 配置文件路径 |
| `HTTP_PROXY` | 否 | — | HTTP 代理 |
| `API_TIMEOUT_MS` | 否 | — | 请求超时（毫秒） |

## 技术细节

### ReAct 流程

```text
用户输入 → start_task() → LLM 调用 → process_llm_result()

├─ shell    → execute_command()
  │     ├─ Safe    → 直接执行，捕获前后文件内容生成 diff
  │     ├─ Confirm → 弹出确认菜单
  │     └─ Block   → 显示被拦截命令，返回错误给 LLM
  ├─ SubAgent  → DAG 调度器
  │     ├─ 拓扑排序 → 自动并行/串行
  │     ├─ 依赖合并 → InputMerge (Concat/Structured)
  │     └─ 输出汇总 → OutputMerge (All/First/Concat)
  ├─ web_search → Bocha AI → 返回摘要继续循环
  ├─ plan       → 渲染 markdown 计划
  ├─ question   → 显示选项菜单，等待用户回答
  ├─ mcp_*      → 调用对应 MCP server
  └─ final      → 保存记忆 → 折叠显示 → 结束
```

### 安全评估

```text
Block:   sudo, format, diskpart, fork bomb (:(){:|:&};:)
Confirm: rm, mv, cp, git commit/push/reset, curl, wget, sed -i, > file
Safe:    ls, cat, grep, git status/log/diff, heredoc 只读, 其他只读操作
```

heredoc 内容不参与评估，仅外层命令生效。

### 记忆机制

**短期记忆**
- 路径：`~/.goldbot/memory/YYYY-MM-DD.md`
- 格式：每日 Markdown 日志，记录任务与输出摘要
- 清理：启动时自动删除 **15 天前**的文件

**长期记忆**
- 路径：`~/.goldbot/MEMORY.md`
- 格式：偏好与规则列表，单条不超过 120 字符
- 提取：任务完成后检测"记忆意图"关键词（记住/默认/以后/always/prefer...）自动提取
- 晋升：14 天内重复出现 **3 次以上**的短期任务自动晋升为长期记忆

**注入机制**
- 时机：启动时注入一次
- 内容：**仅长期记忆**（全量注入，无条数限制）
- 位置：Assistant Message（非 System Prompt）

**上下文压缩**
- 触发：剩余 token 低于动态阈值时自动触发
- 保留：首选最近 **18 条**消息，最低保留 6 条
- 摘要：历史对话压缩为单条 User Message

### TUI 渲染分区

终端屏幕分为两个独立区域：

```
┌─────────────────────────────────────┐
│  滚动区（Scrollback Zone）           │ ← emit_live_event() 在此追加输出
│  已打印内容不可修改                   │   保存在 screen.task_rendered 供重绘
│                                     │
│  ⏺ shell  ls -la                   │
│  ⏺ read   src/main.rs              │
│  ...                                │
├─────────────────────────────────────┤ ← clear_managed() 从此行向下清空
│  管理区（Managed Area）              │ ← 每次 refresh() 完整重绘
│                                     │
│  [Todo 进度面板]         (可选)      │
│  [@ / /model 选择器]    (可选)      │
│  [Live DAG 树形]         (可选)     │ ← screen.dag_tree: Option<String>
│  ⣟ Thinking... (3s • esc to stop)  │ ← screen.status / screen.status_right
│  ❯ 用户输入                          │ ← screen.input
│    → mode: agent (shift+tab)        │ ← hint 行
└─────────────────────────────────────┘
```

**核心规则**：需要原地实时更新的内容必须放在管理区；滚动区的内容一旦打印就无法修改。

| API | 说明 |
|---|---|
| `emit_live_event(screen, &ev)` | 把事件打印到滚动区，追加到 `task_rendered`，然后重绘管理区 |
| `screen.refresh()` | `clear_managed()` + `draw_managed()`，重绘整个管理区 |
| `screen.refresh_status_only()` | 仅原地覆写状态行（spinner 跳帧专用，避免闪烁） |
| `screen.dag_tree: Option<String>` | SubAgent DAG 执行期间的实时树形；节点完成时更新，DAG 结束后清空 |
### 技术栈

- Rust 2024 Edition
- Tokio（异步运行时）
- crossterm（TUI）
- reqwest（HTTP）
- similar（diff 计算）
- serde / serde_json（序列化）
- toml（配置解析）
- regex（正则匹配）
- anyhow（错误处理）
- chrono（日期时间）
- dotenvy（环境变量）
- arboard（剪贴板）
- unicode-width（Unicode 宽度计算）
 - ignore（文件忽略规则）

## SubAgent 子代理

SubAgent 是独立的 ReAct 循环，由 MainAgent 编排，负责实际执行工作。

### DAG 调度机制

- **拓扑排序自动分层**：根据 `depends_on` 依赖关系自动构建执行层级
- **同层节点并行执行**：无依赖的节点同时启动，提升效率
- **跨层节点串行执行**：有依赖的节点按顺序执行
- **依赖输出自动传递**：上游节点的输出自动合并传递给下游

### 角色预设（role）

| 角色 | 别名 | 职责 | 可用工具 |
|---|---|---|---|
| search | researcher | 信息检索与研究 | shell, read, search, web_search, final |
| coding | code, dev | 代码编写与调试 | shell, read, write, update, search, final |
| analysis | analyst | 数据分析与报告 | shell, read, search, final |
| writer | writing | 文案与文档 | shell, read, write, search, final |
| reviewer | review, critic | 质量审查 | shell, read, search, final |
| docs | doc, readme | 项目文档维护 | read, write, update, search, final |

### 输入合并（input_merge）

- `concat`（默认）：纯文本拼接
- `structured`：JSON 结构化，保留来源信息

### 输出汇总（output_merge）

- `all`（默认）：返回所有输出节点的完整 JSON
- `first`：竞争模式，仅返回最先完成的节点
- `concat`：文本拼接

### 支持的工具

`shell`, `read`, `write`, `update`, `search`, `web_search`, `final`

> 注意：SubAgent 不支持 MCP、skill、嵌套 sub_agent

### 配置参数

| 参数 | 默认值 | 说明 |
|---|---|---|
| max_steps | 30 | 最大步数 |
| timeout | 600s | 超时时间 |

### JSON 配置示例

```json
{
  "nodes": [
    {"id": "search", "task": "搜索 Rust 异步最佳实践", "role": "search"},
    {"id": "analyze", "task": "分析结果", "role": "analysis", "depends_on": ["search"]},
    {"id": "code", "task": "编写示例代码", "role": "coding", "depends_on": ["analyze"]},
    {"id": "review", "task": "审查代码", "role": "reviewer", "depends_on": ["code"]}
  ],
  "output_nodes": ["review"],
  "output_merge": "all"
}
```

### 架构图

```
MainAgent (编排：意图理解 → 任务拆解 → DAG 分发 → 结果审查)
    │
    ▼
DAG Scheduler (拓扑排序 → 分层并行 → 输入合并 → 输出汇总)
    │
    ▼
SubAgent Workers (独立 ReAct 循环：LLM → 工具执行 → ... → Final)
```
