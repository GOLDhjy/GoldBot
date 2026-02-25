# GoldBot - AI Terminal Automation Agent

A cross-platform TUI Agent built with Rust that automatically plans and executes shell commands via LLM to complete tasks.

[简体中文版](README.md)

## Features

- **ReAct Loop**: Think → Act → Observe → Think again — supports shell / plan / question / web_search / MCP actions
- **Three-Level Safety**: Safe/Confirm/Block, heredoc content is never misidentified
- **File Diff**: Automatically compares file content before/after command execution, line-numbered red/green highlighting
- **Real-time TUI**: Streamed thinking process, collapsed by default after completion
- **Native Deep Thinking**: Tab key toggles API-level `reasoning_content` stream
- **Auto Context Compaction**: Summarizes old messages when threshold is reached
- **Persistent Memory**: Short-term (daily) + long-term (auto-extracted preferences), injected into every request
- **MCP Tools**: Auto-discover and expose MCP tools as `mcp_<server>_<tool>`
- **Web Search**: Bocha AI integration — LLM can proactively search for up-to-date information
- **Skills System**: Auto-discovers skills from `~/.goldbot/skills/` and injects them into the system prompt
- **@ File Attachment**: Type `@` in the input box to search and attach files — they are sent with the message to the LLM
- **Slash Commands**: Type `/` to open a command picker with 8 built-in commands and user-defined extensions
- **GE Golden Experience**: Multi-model collaboration (Claude executes → Codex optimizes → GoldBot validates), auto git commit
- **Cross-Platform**: macOS/Linux (bash) / Windows (PowerShell)

## Installation

### macOS / Linux

```bash
# One-line install (recommended)
curl -fsSL https://raw.githubusercontent.com/GOLDhjy/GoldBot/master/scripts/install.sh | bash

# Install from source
curl -fsSL https://raw.githubusercontent.com/GOLDhjy/GoldBot/master/scripts/install.sh | bash -s -- --source
```

### Windows (PowerShell)

```powershell
irm "https://raw.githubusercontent.com/GOLDhjy/GoldBot/master/scripts/install.ps1" | iex
```

### Homebrew

```bash
brew install GOLDhjy/GoldBot/goldbot
```

### Manual Download

- macOS Intel: `goldbot-v*-macos-x86_64.tar.gz`
- macOS Apple Silicon: `goldbot-v*-macos-aarch64.tar.gz`
- Linux x86_64: `goldbot-v*-linux-x86_64.tar.gz`
- Windows x86_64: `goldbot-v*-windows-x86_64.zip`

### Build from Source

```bash
cargo install --git https://github.com/GOLDhjy/GoldBot.git
```

## Usage

```bash
goldbot
```

### CLI Arguments

| Flag | Description |
|---|---|
| `-p <message>` / `--prompt <message>` | Send an initial chat message on startup without manual input |
| `-y` / `--yes` | Auto-accept all Confirm-level commands without prompting (Block-level commands are still rejected) |

```bash
# Send a message on startup (Confirm commands still require approval)
goldbot -p "clean up large files in the current directory"

# Interactive mode, but auto-accept all Confirm commands
goldbot -y

# Fully automated: auto-send message + auto-accept commands
goldbot -p "clean up large files in the current directory" -y
```

### Keyboard Shortcuts

| Key | Context | Action |
|---|---|---|
| `Ctrl+C` | Anywhere | Exit |
| `Ctrl+D` | After task completes | Collapse/expand details |
| `Tab` | Outside menu | Toggle deep thinking ON/OFF |
| `Shift+Tab` | Outside menu | Cycle assist mode (agent / accept edits / plan) |
| `@` | Empty input box | Open file attachment picker |
| `/` | Empty input box | Open slash command picker |
| `↑/↓` | Menu / picker mode | Move selection |
| `Enter` / `Tab` | Picker mode | Confirm selection |
| Type any char | Question menu | Enter free-text input mode |
| `Esc` | Input focused | Unfocus / cancel picker / return to menu |

### Confirmation Menu (risky commands)

Shown when a Confirm-level command is about to run:

1. Execute
2. Skip
3. Abort
4. Note — add an instruction before retrying

### Question Menu (LLM asks user)

Shown when the LLM uses the `question` tool. Options are decided by the LLM. The last option is typically `✏ Custom input` — select it or just start typing to enter free-text mode.

## @ File Attachment

Type `@` in an empty input box to open the file picker:

- Continue typing to filter by filename (case-insensitive)
- `↑/↓` to navigate, `Enter` or `Tab` to attach the selected file
- `Esc` or backspace over `@` to cancel
- Multiple files can be attached; each appears as `@path/to/file` in the input
- On submit, absolute paths are appended to the message so the LLM can reference them

Search covers the current workspace, automatically skipping `.git`, `target`, `node_modules`, etc.

## Slash Commands

Type `/` in an empty input box to open the command picker. Type to filter, `↑/↓` to navigate, `Enter` to run.

### Built-in Commands

| Command | Description |
|---|---|
| `/help` | Show keyboard shortcuts and available commands |
| `/clear` | Clear conversation history and start fresh |
| `/compact` | Immediately truncate context, keeping the last 18 messages |
| `/memory` | View current long-term and short-term memory |
| `/thinking` | Toggle native Thinking mode (same as Tab) |
| `/skills` | List all discovered Skills |
| `/mcp` | List all registered MCP tools and their status |
| `/status` | Show workspace, model, Thinking state, and other config |

### User-Defined Commands

Create a command file at `~/.goldbot/command/<name>/COMMAND.md`:

```markdown
---
name: my-cmd
description: What this command does
---

Template content — placed into the input box for editing before submit
```

The directory name must match the `name` field. Only letters, digits, and hyphens are allowed in `name`.

## GE Golden Experience

GE (Golden Experience) is a continuous supervisor mode for development tasks. The execution pipeline is fixed: **Claude executes → Codex checks/optimizes → GoldBot read-only validates**. A git commit is created automatically after each todo passes validation.

### Commands

| Command | Description |
|---|---|
| `GE <goal>` | Enter GE mode; triggers 3-question bootstrap if no `CONSENSUS.md` |
| `GE` | Enter GE mode; loads existing `CONSENSUS.md` directly |
| `GE replan` | Regenerate todo plan from current consensus |
| `GE exit` | Leave GE mode |

### Interview Phase

When entering GE mode for the first time (no `CONSENSUS.md`), GoldBot runs a structured interview:

1. **Purpose** — the core goal of this development session
2. **Rules** — coding standards, tech stack, testing requirements, etc.
3. **Scope** — task boundaries: what's in and what's out

`CONSENSUS.md` is generated automatically in the project root upon completion.

### Todo Plan

After consensus is established, the LLM generates 8–12 fine-grained steps. Each step progresses Pending → Running → Done, shown in real-time in the TUI sidebar.

### Audit Log

All GE operations are appended to `GE_LOG.jsonl` in the project root. This file is automatically excluded from git commits.

## MCP Integration

Default config path: `~/.goldbot/mcp_servers.json` — compatible with OpenCode config format.

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

You can also override with the `GOLDBOT_MCP_SERVERS` environment variable (same JSON format).

On startup, GoldBot runs `tools/list` to discover tools and exposes them as `mcp_<server>_<tool>`.

### Config Fields

| Field | Required | Default | Description |
|---|---|---|---|
| `type` | No | `local` | Only `local` (stdio) is supported |
| `command` | Yes | — | Command and arguments array |
| `env` | No | `{}` | Environment variables for the server |
| `cwd` | No | current dir | Working directory for the server |
| `enabled` | No | `true` | Set to `false` to skip |

### Troubleshooting

- `Failed to load MCP tools...`: verify the command array runs locally and the server supports stdio MCP
- `unknown MCP tool`: restart and check `enabled` and dependency installation
- Argument errors: `<arguments>` must be a JSON object, not an array or plain text

## Skills

Create skill files at `~/.goldbot/skills/<name>/SKILL.md`. They are auto-discovered at startup and injected into the system prompt. You can also ask GoldBot to create one, or have it fetched automatically from Claude / Codex:

```
Help me create a skill for organizing PDF files
```

```markdown
---
name: pdf
description: Organize and process PDF files
---

# Skill content (free-form Markdown)
```

## Environment Variables

Config is stored at `~/.goldbot/.env` — auto-created from template on first run.

| Variable | Required | Default | Description |
|---|---|---|---|
| `BIGMODEL_API_KEY` | ✅ | — | BigModel API key |
| `BIGMODEL_BASE_URL` | No | `https://open.bigmodel.cn/api/coding/paas/v4` | API base URL |
| `BIGMODEL_MODEL` | No | `GLM-4.7` | Model name |
| `BOCHA_API_KEY` | No | — | Bocha AI search key |
| `GOLDBOT_TASK` | No | — | Task to run immediately on startup |
| `GOLDBOT_MCP_SERVERS` | No | — | MCP config JSON (overrides file) |
| `GOLDBOT_MCP_SERVERS_FILE` | No | `~/.goldbot/mcp_servers.json` | MCP config file path |
| `HTTP_PROXY` | No | — | HTTP proxy |
| `API_TIMEOUT_MS` | No | — | Request timeout in milliseconds |

## Technical Details

### ReAct Flow

```text
User input → start_task() → LLM call → process_llm_result()

  ├─ shell      → execute_command()
  │     ├─ Safe    → Execute directly, capture before/after diff
  │     ├─ Confirm → Show confirmation menu
  │     └─ Block   → Show blocked command, return error to LLM
  ├─ web_search → Bocha AI → return summary, continue loop
  ├─ plan       → Render markdown plan
  ├─ question   → Show option menu, wait for user answer
  ├─ mcp_*      → Call MCP server
  └─ final      → Save memory → collapse display → done
```

### Safety Assessment

```text
Block:   sudo, format, diskpart, fork bomb (:(){:|:&};:)
Confirm: rm, mv, cp, git commit/push/reset, curl, wget, sed -i, > file
Safe:    ls, cat, grep, git status/log/diff, read-only heredoc, other read-only ops
```

Heredoc body content is never evaluated — only the outer command is assessed.

### Memory

- **Short-term**: `~/.goldbot/memory/YYYY-MM-DD.md` — daily log
- **Long-term**: `~/.goldbot/MEMORY.md` — preferences and rules, auto-deduplicated
- **Injection**: loaded once at startup — last 30 long-term entries + 2 days of short-term memory, embedded into the System Prompt
- **Compaction**: when messages exceed 48, older ones are summarized, keeping the last 18

### Project Structure

```
src/
├── main.rs              # Entry point + event loop + App state
├── types.rs             # Core type definitions
├── agent/
│   ├── react.rs         # System prompt + XML response parsing
│   ├── executor.rs      # start_task → process → execute → finish
│   └── provider.rs      # LLM HTTP + SSE streaming
├── tools/
│   ├── shell.rs         # Command execution, classification, diff capture
│   ├── safety.rs        # Risk assessment (Safe/Confirm/Block)
│   ├── mcp.rs           # MCP server discovery and invocation
│   ├── web_search.rs    # Bocha AI search
│   ├── skills.rs        # Skills loading
│   └── command.rs       # Slash command definitions and discovery
├── memory/
│   ├── store.rs         # Memory read/write
│   └── compactor.rs     # Context compaction
├── ui/
│   ├── screen.rs        # TUI screen management
│   ├── format.rs        # Event formatting and diff highlighting
│   ├── input.rs         # Keyboard and paste handling
│   └── ge.rs            # GE mode UI
└── consensus/
    ├── engine.rs        # GE engine: state machine, pipeline orchestration
    ├── evaluate.rs      # Validation: execution acceptance, done_when checks
    ├── external.rs      # External LLM interfaces (Claude / Codex)
    ├── subagent.rs      # 3-question generation, todo plan generation
    ├── model.rs         # Data models
    └── audit.rs         # Audit log
```

### Tech Stack

- Rust 2024 Edition
- Tokio (async runtime)
- crossterm (TUI)
- reqwest (HTTP)
- similar (diff computation)

## TODO

- ~~@ file attachment~~
- ~~Slash commands~~
- Syntax highlighting for code in diffs
