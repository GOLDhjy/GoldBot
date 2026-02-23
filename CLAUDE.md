# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Commands

```bash
# Build (debug)
cargo build

# Build (release)
cargo build --release

# Run
cargo run

# Run with predefined task
GOLDBOT_TASK="整理当前目录的大文件" cargo run

# Run tests
cargo test

# Run a single test
cargo test <test_name>

# Check without building
cargo check

# Lint
cargo clippy
```

## Architecture

GoldBot is a **cross-platform TUI agent** built with Rust that uses LLMs to automate shell command execution. It features a ReAct (Reasoning-Acting) loop, streaming responses with native thinking support, MCP (Model Context Protocol) integration, and a GE (GoldBot Enhanced) supervision mode for structured development workflows.

### Execution Flow

`main.rs` owns the main event loop. The `App` struct holds all runtime state including message history, MCP registry, skills, and todo items. The loop processes:

1. **LLM streaming responses** via `LlmWorkerEvent` channels (separate `Delta` and `ThinkingDelta` streams)
2. **Keyboard/paste input** via `handle_key` and `handle_paste`
3. **MCP discovery** completes asynchronously, augments system prompt when done
4. **GE mode events** via `drain_ge_events`

When a task starts (`start_task` in `agent/step.rs`):
1. `maybe_flush_and_compact_before_call()` - if messages exceed 48, compacts context with summary
2. LLM is called with streaming (two streams: `content` and `reasoning_content`)
3. `process_llm_result()` parses response via `parse_llm_response()` from `agent/react.rs`
4. Actions are dispatched: `shell`, `web_search`, `plan`, `question`, `mcp_*`, `skill`, `create_mcp`, `todo`, or `final`
5. For shell commands: `assess_command()` classifies as Safe/Confirm/Block

### Module Map

| Module | Purpose |
|---|---|
| `src/main.rs` | Entry point, event loop, `App` struct, async LLM worker spawning |
| `src/types.rs` | `Event`, `LlmAction`, `TodoItem`, `Mode`, `TodoStatus`, GE-related enums |
| `src/agent/react.rs` | System prompt template, `parse_llm_response()` for XML-style tool tags |
| `src/agent/step.rs` | Core step lifecycle: `start_task`, `process_llm_result`, command execution, context compaction |
| `src/agent/provider.rs` | `Message` type, HTTP client with proxy support, `chat_stream_with()` for streaming API calls |
| `src/tools/shell.rs` | Command execution (bash on Unix, PowerShell on Windows), command classification (Read/Write/Update/Search/Bash) |
| `src/tools/safety.rs` | `assess_command()` - three-tier risk: Block (dangerous), Confirm (destructive), Safe (read-only) |
| `src/tools/mcp.rs` | MCP server discovery (stdio), tool registration, execution, `augment_system_prompt()` |
| `src/tools/web_search.rs` | Bocha AI integration for internet search |
| `src/tools/skills.rs` | Skill discovery from `~/.goldbot/skills/*/SKILL.md` |
| `src/memory/store.rs` | Dual-layer memory: short-term daily logs (`memory/YYYY-MM-DD.md`), long-term (`MEMORY.md`) |
| `src/memory/compactor.rs` | Context compression logic |
| `src/ui/screen.rs` | TUI rendering, event emission, status bar |
| `src/ui/format.rs` | Markdown rendering, event formatting |
| `src/ui/input.rs` | Keyboard and paste handling |
| `src/ui/ge.rs` | GE mode-specific UI rendering |
| `src/consensus/engine.rs` | GE mode state management, interview flow (Purpose/Rules/Scope) |
| `src/consensus/evaluate.rs` | Multi-model evaluation (Claude → Codex → GoldBot review) |
| `src/consensus/external.rs` | External LLM API calls for GE workflow |
| `src/consensus/model.rs` | `Consensus`, `AuditEvent` data structures |
| `src/consensus/audit.rs` | JSONL audit logging to `GE_LOG.jsonl` |

### LLM Response Format

The LLM responds with XML-style tags:

```xml
<thought>reasoning here</thought>
<tool>shell</tool><command>ls -la</command>
```

Supported tools:
- `shell` - Execute command (risk-assessed)
- `web_search` - Internet search via Bocha AI
- `plan` - Render markdown plan to TUI
- `question` - Ask user with numbered options
- `mcp_<server>_<tool>` - Call MCP tool
- `skill` - Load skill content
- `create_mcp` - Create new MCP server config
- `todo` - Update todo progress panel
- `final` - Complete task

### Memory System

- **Short-term**: `~/.goldbot/memory/YYYY-MM-DD.md` - daily logs
- **Long-term**: `~/.goldbot/MEMORY.md` - persistent preferences/rules
- **Injection**: System prompt rebuilt with memory (last 30 long-term + 2 days short-term) on each LLM call
- **Compaction**: At 48+ messages, old context summarized to `[Context compacted]` block, keeping 18 recent

### GE (GoldBot Enhanced) Mode

Supervision mode for structured development:

1. **Interview** (if no `CONSENSUS.md`): Three questions (Purpose/Rules/Scope)
2. **Todo Planning**: LLM generates 8-12 granular steps
3. **Execution**: Claude executes → Codex optimizes → GoldBot reviews
4. **Auto-commit**: Git commits after each Todo (excludes `GE_LOG.jsonl`)
5. **Audit log**: All operations logged to `GE_LOG.jsonl`

Trigger with `GE` prefix in input.

### Keybindings

| Key | Action |
|---|---|
| `Ctrl+C` | Quit |
| `Ctrl+D` | Toggle detail/collapsed view |
| `Tab` | Toggle native thinking ON/OFF |
| `↑` / `↓` | Navigate menus |
| `Enter` | Confirm selection |
| Type characters | Enter custom input in question menu |
| `Esc` | Exit input mode |

### Environment Variables

| Variable | Required | Default | Purpose |
|---|---|---|
| `BIGMODEL_API_KEY` | Yes | — | GLM API key |
| `BIGMODEL_BASE_URL` | No | `https://open.bigmodel.cn/api/coding/paas/v4` | API endpoint |
| `BIGMODEL_MODEL` | No | `GLM-4.7` | Model name |
| `BOCHA_API_KEY` | No | — | Bocha AI search key |
| `GOLDBOT_TASK` | No | — | Predefined task on startup |
| `GOLDBOT_MCP_SERVERS` | No | — | MCP config JSON |
| `GOLDBOT_MCP_SERVERS_FILE` | No | `~/.goldbot/mcp_servers.json` | MCP config path |
| `HTTP_PROXY` | No | — | HTTP proxy |
| `API_TIMEOUT_MS` | No | — | Request timeout |

Config stored at `~/.goldbot/.env` (auto-created from template on first run).
