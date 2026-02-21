# GoldBot - AI Terminal Automation Agent

A cross-platform TUI Agent built with Rust that automatically plans and executes shell commands via LLM to complete tasks.

[简体中文版](README.md)

## Features

- **Three-Level Safety Control**: Safe/Confirm/Block
- **Persistent Memory**: Short-term (daily) + Long-term (auto-extracted preferences), injected into every request via System Prompt
- **ReAct Loop**: Think → Act → Observe → Think again
- **Smart Command Classification**: Read/Write/Update/Search/Bash
- **Real-time TUI**: Streamed thinking process, collapsed by default after completion
- **Native LLM Deep Thinking**: Tab key toggles API-level `reasoning_content` stream
- **Auto Context Compaction**: Summarizes old messages when threshold is reached
- **Cross-Platform**: macOS/Linux (bash) / Windows (PowerShell)
- **Optional MCP Tools**: Auto-discover and expose MCP tools as `mcp_<server>_<tool>`

## Installation

### macOS / Linux (Recommended)

**One-Line Install (Recommended)**

```bash
curl -fsSL https://raw.githubusercontent.com/GOLDhjy/GoldBot/master/scripts/install.sh | bash
```

**Install specific version**

```bash
curl -fsSL https://raw.githubusercontent.com/GOLDhjy/GoldBot/master/scripts/install.sh | bash -s -- --version v0.2.0
```

**Install from source**

```bash
curl -fsSL https://raw.githubusercontent.com/GOLDhjy/GoldBot/master/scripts/install.sh | bash -s -- --source
```

### Windows (PowerShell)

```powershell
irm "https://raw.githubusercontent.com/GOLDhjy/GoldBot/master/scripts/install.ps1" | iex
```

### Homebrew (macOS / Linux)

```bash
brew install GOLDhjy/GoldBot/goldbot
```

### Manual Download (3 Platforms)

- macOS Intel: `goldbot-v*-macos-x86_64.tar.gz`
- macOS Apple Silicon: `goldbot-v*-macos-aarch64.tar.gz`
- Linux x86_64: `goldbot-v*-linux-x86_64.tar.gz`
- Windows x86_64: `goldbot-v*-windows-x86_64.zip`

### Build from Source (All Platforms)

```bash
cargo install --git https://github.com/GOLDhjy/GoldBot.git
```

## Project Structure

```
GoldBot/
├── src/
│   ├── main.rs           # Entry point + main event loop
│   ├── agent/
│   │   ├── react.rs      # ReAct framework: system prompt + response parsing
│   │   ├── step.rs       # Core steps: start → process → execute → finish
│   │   └── provider.rs   # LLM interface (HTTP + streaming)
│   ├── tools/
│   │   ├── shell.rs      # Command execution + classification
│   │   ├── mcp.rs        # MCP server config/discovery/invocation (stdio)
│   │   └── safety.rs     # Risk assessment (Safe/Confirm/Block)
│   ├── memory/
│   │   ├── store.rs      # Memory storage (short/long-term)
│   │   └── compactor.rs  # Context compression
│   ├── ui/
│   │   ├── screen.rs     # TUI screen management
│   │   └── format.rs     # Event formatting
│   └── types.rs
├── Cargo.toml
└── README.md
```

## How It Works

### LLM Integration

GoldBot calls GLM-4.7 via the BigModel native API (OpenAI-compatible format):

- **Endpoint**: `BIGMODEL_BASE_URL/chat/completions` (default: `https://open.bigmodel.cn/api/coding/paas/v4`)
- **Auth**: `Authorization: Bearer <BIGMODEL_API_KEY>`
- **Model**: `BIGMODEL_MODEL` (default: `GLM-4.7`)
- **Streaming**: SSE format, two delta types:
  - `content` → answer text, drives TUI scrolling preview
  - `reasoning_content` → deep thinking content, shown in status bar

Deep thinking is controlled by `{"thinking": {"type": "enabled"|"disabled"}}` API parameter, toggled with the Tab key (default: ON). GLM automatically caches repeated system message prefixes, so repeated tokens are not billed again.

### Main Event Loop (main.rs)

```text
loop {
    1. Handle LLM streaming (reasoning_content → status bar, content → accumulated text)
    2. Trigger Agent step (async LLM API call)
    3. Handle keyboard input (Ctrl+C/D, Tab, ↑/↓/Enter)
}
```

### ReAct Loop Flow

```text
User enters task
  → start_task() (reset state, set needs_agent_step=true)
  → maybe_flush_and_compact_before_call() (compact if message count exceeds threshold)
  → LLM call (send System Prompt + history)
  → process_llm_result() (parse <thought> and <tool>/<final>)

  Branches:
  ├─ <tool>shell → execute_command()
  │      ├─ Safe    → Execute directly
  │      ├─ Confirm → Popup confirmation menu
  │      └─ Block   → Reject (return error to LLM)
  │           → Add result to history → needs_agent_step=true (loop)
  │
  └─ <final> → finish()
       → Save memory → Collapse display → running=false (exit)
```

### Safety Assessment

```text
Block:   sudo, format, fork bomb (:(){:|:&};:)
Confirm: rm, mv, cp, git commit, curl, wget, >, >>
Safe:    ls, cat, grep, git status, read-only ops
```

### Memory Mechanism

- **Short-term**: `~/.goldbot/memory/YYYY-MM-DD.md` (daily log, appended after each task)
- **Long-term**: `~/.goldbot/MEMORY.md` (preferences/rules, auto-extracted and deduplicated)
- **Startup injection**: `App::new()` reads long-term memory (last 30 entries) + recent 2 days short-term, embeds into System Prompt — **sent with every request in messages[0]** (GLM auto-caches repeated prefix)
- **Context compaction**: When message count exceeds 48, older messages are summarized as `[Context compacted]` and trimmed to the last 18 messages

### Environment Variables

| Variable | Required | Default | Description |
|---|---|---|---|
| `BIGMODEL_API_KEY` | ✅ | — | BigModel API key |
| `BIGMODEL_BASE_URL` | No | `https://open.bigmodel.cn/api/coding/paas/v4` | API base URL |
| `BIGMODEL_MODEL` | No | `GLM-4.7` | Model name |
| `HTTP_PROXY` | No | — | HTTP proxy URL |
| `API_TIMEOUT_MS` | No | — | Request timeout in milliseconds |
| `GOLDBOT_TASK` | No | — | Task to run immediately on startup |
| `GOLDBOT_MCP_SERVERS` | No | — | MCP server config JSON |
| `GOLDBOT_MCP_SERVERS_FILE` | No | `~/.goldbot/mcp_servers.json` | MCP config file path (used only when `GOLDBOT_MCP_SERVERS` is not set) |

Create a `.env` file in the project root (auto-loaded on startup):

```env
BIGMODEL_API_KEY=your_api_key_here
BIGMODEL_BASE_URL=https://open.bigmodel.cn/api/coding/paas/v4
BIGMODEL_MODEL=GLM-4.7
```

## Usage

```bash
goldbot
```

### GE Supervisor Mode (Chat Trigger)

Use input lines that start with `GE` to enter continuous supervisor mode:

- `GE <goal>`: enter GE mode (if `CONSENSUS.md` is missing, GoldBot runs a fixed 3-question bootstrap and generates it)
- `GE`: enter GE mode (attach to existing `CONSENSUS.md`)
- `GE exit` / `GE 退出`: leave GE mode
- `GE replan` / `GE 细化todo`: regenerate a finer todo plan from current Purpose/Rules/Scope

In GE mode:

- Execution pipeline is fixed: Claude executes -> Codex checks/optimizes -> GoldBot performs read-only validation
- Todos are generated by LLM after the 3-question bootstrap (target: 8-12 fine-grained steps)
- After each todo passes validation, GoldBot runs self-review and creates a local git commit
- Auto commit excludes `GE_LOG.jsonl` to avoid log noise in code history
- Consensus file path: project root `CONSENSUS.md`
- Audit log path: project root `GE_LOG.jsonl` (JSONL, single append-only file)
- Triggers: immediate reload after each todo completion + periodic reload every 30 minutes

## MCP Integration (OpenCode-style)

By default, GoldBot loads MCP config from the memory directory:

- macOS / Linux: `~/.goldbot/mcp_servers.json`
- If `GOLDBOT_MEMORY_DIR` is set: `$GOLDBOT_MEMORY_DIR/mcp_servers.json`

File content is JSON (`server_name -> config`):

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

Then run:

```bash
goldbot
```

You can still override with `GOLDBOT_MCP_SERVERS`:

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

Notes:
- Current implementation supports `local` (stdio) servers and OpenCode-like fields: `type/command/args/env/cwd/enabled`.
- On startup, GoldBot runs MCP `tools/list`, then maps discovered tools to `mcp_<server>_<tool>`.
- LLM call format:
  - `<tool>mcp_...</tool>`
  - `<arguments>{"key":"value"}</arguments>`

### MCP Config Fields

| Field | Required | Default | Description |
|---|---|---|---|
| `type` | No | `local` | Currently only `local` (stdio) is supported |
| `command` | Yes | — | Command to launch the MCP server |
| `args` | No | `[]` | Command arguments |
| `env` | No | `{}` | Environment variables for the MCP server |
| `cwd` | No | current directory | Working directory for the MCP server |
| `enabled` | No | `true` | Skip server when set to `false` |

### Multi-server Example

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

### Migration from OpenCode / OpenClawd

- If your config is already `server_name -> config`, you can copy it directly into `GOLDBOT_MCP_SERVERS`.
- If your config is wrapped like this:

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

Use the inner `mcpServers` object as `GOLDBOT_MCP_SERVERS`:

```bash
export GOLDBOT_MCP_SERVERS='{
  "context7": {
    "command": "npx",
    "args": ["-y", "@upstash/context7-mcp"]
  }
}'
```

### Troubleshooting

- `Failed to load MCP tools...` on startup:
  - Verify `command/args` can run locally and the server supports stdio MCP.
- `unknown MCP tool` at runtime:
  - The tool was not discovered at startup; restart and check `enabled`, dependencies, and `tools/list`.
- Argument errors:
  - `<arguments>` must be a JSON object (`{}`), not an array or plain text.

### Keyboard Shortcuts

| Key | Context | Action |
|---|---|---|
| `Ctrl+C` | Anywhere | Exit |
| `Ctrl+D` | After task completes | Collapse/expand details |
| `Tab` | Outside confirmation menu | Toggle deep thinking ON/OFF |
| `↑/↓` | Confirmation menu | Move selection |
| `Enter` | Confirmation menu | Confirm selection |
| `Esc` | Input focused | Unfocus / return to menu |

### Confirmation Menu

1. Execute - Execute command
2. Skip - Skip command
3. Abort - Abort task
4. Note - Add instruction

## Tech Stack

- Rust 2024 Edition
- Tokio (async runtime)
- crossterm (TUI)
- reqwest (HTTP)
- serde_json
