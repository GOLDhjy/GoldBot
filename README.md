# GoldBot - AI 终端自动化助手

[![GitHub Release](https://img.shields.io/github/v/release/GOLDhjy/GoldBot?display_name=tag&style=flat-square)](https://github.com/GOLDhjy/GoldBot/releases) [![GitHub Actions](https://img.shields.io/github/actions/workflow/status/GOLDhjy/GoldBot/release.yml?style=flat-square)](https://github.com/GOLDhjy/GoldBot/actions/workflows/release.yml) [![GitHub Stars](https://img.shields.io/github/stars/GOLDhjy/GoldBot?style=flat-square)](https://github.com/GOLDhjy/GoldBot/stargazers) [![License](https://img.shields.io/github/license/GOLDhjy/GoldBot?style=flat-square)](https://github.com/GOLDhjy/GoldBot/blob/main/LICENSE)

一个基于 Rust 开发的跨平台 TUI Agent，通过 LLM 自动规划和执行 Shell 命令来完成任务。

[English Version](README_EN.md)

## 特性

- ReAct 循环：思考-执行-观察-再思考，支持 shell / plan / question / web_search / MCP 多种动作
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
| `BIGMODEL_API_KEY` | ✅ | — | BigModel API 密钥 |
| `BIGMODEL_BASE_URL` | 否 | `https://open.bigmodel.cn/api/coding/paas/v4` | API 基础 URL |
| `BIGMODEL_MODEL` | 否 | `GLM-4.7` | 模型名称 |
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

- **短期**：`~/.goldbot/memory/YYYY-MM-DD.md`，每日日志
- **长期**：`~/.goldbot/MEMORY.md`，偏好与规则，自动去重提取
- **注入**：启动时读取长期记忆（最近 30 条）+ 最近 2 天短期记忆，嵌入 System Prompt，仅注入一次
- **压缩**：消息超 48 条时自动摘要，保留最近 18 条

### 项目结构

```
src/
├── main.rs              # 入口 + 主事件循环 + App 状态
├── types.rs             # 核心类型定义
├── agent/
│   ├── react.rs         # 系统提示词 + XML 响应解析
│   ├── executor.rs      # start_task → process → execute → finish
│   └── provider.rs      # LLM HTTP + SSE 流式处理
├── tools/
│   ├── shell.rs         # 命令执行、分类、diff 捕获
│   ├── safety.rs        # 风险评估（Safe/Confirm/Block）
│   ├── mcp.rs           # MCP server 发现与调用
│   ├── web_search.rs    # Bocha AI 搜索
│   ├── skills.rs        # Skills 加载
│   └── command.rs       # Slash 命令定义与发现
├── memory/
│   ├── store.rs         # 记忆读写
│   └── compactor.rs     # 上下文压缩
├── ui/
│   ├── screen.rs        # TUI 屏幕管理
│   ├── format.rs        # 事件格式化与 diff 高亮
│   ├── input.rs         # 键盘与粘贴处理
│   └── ge.rs            # GE 模式 UI
└── consensus/
    ├── engine.rs        # GE 引擎：状态管理、流程编排
    ├── evaluate.rs      # 执行验收、done_when 校验
    ├── external.rs      # 外部 LLM 接口（Claude / Codex）
    ├── subagent.rs      # 三问生成、Todo 计划生成
    ├── model.rs         # 数据模型
    └── audit.rs         # 审计日志
```

### 技术栈

- Rust 2024 Edition
- Tokio（异步运行时）
- crossterm（TUI）
- reqwest（HTTP）
- similar（diff 计算）

## TODO
- ~~@ 功能实现~~
- ~~slash command~~
- diff 代码语法高亮
- ~~容易陷入死循环~~
- 减少 Bash 这些标题，合并成一个，如果一直重复
- Bash异步
- 子代理
- 上下文自动压缩
- 固定工具:
  它有专用工具：Read、Write、Edit、Glob、Grep，同时保留了 Bash 工具作为 shell 兜底。                                                                                                             
                                                            
  所以你的想法和 Claude Code 的实际设计是一致的，没什么不对劲。  
- 