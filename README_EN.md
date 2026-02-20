# GoldBot (TUI MVP)

> 简体中文 | [English Version](README_EN.md)

A cross-platform TUI Agent prototype: enters once, executes commands in a loop following a plan, displays the process, and defaults to showing only the final summary after completion.

## Features

### Unified Tool Interface
- `run_command` unified tool interface
  - macOS/Linux: `bash -lc`
  - Windows: `powershell -NoProfile -Command`

### Risk Control (Three-Level Assessment)
- **Safe** - Execute directly (e.g., `ls`, `git status`, `sed -n`)
- **Confirm** - Popup menu for confirmation (e.g., `rm`, `git commit`, `sed -i`)
- **Block** - Directly block (e.g., `sudo`, `format`, fork bombs)
- High-risk commands trigger a **selectable menu** (not text input)

### Command Classification
Commands are automatically classified into 5 types:
- **Read** - Read-only operations (e.g., `cat`, `ls`, `sed -n`)
- **Write** - Write operations (e.g., `cat > file`)
- **Update** - Update operations (e.g., `sed -i`, `rm`)
- **Search** - Search operations (e.g., `rg`, `grep`)
- **Bash** - Other shell commands

### Process Visibility
- Event types: `Thinking / ToolCall / ToolResult / NeedsConfirmation / Final`
- Collapsed by default after completion, showing only final summary
- Press `d` to expand/collapse details

## Running

```bash
cargo run
```

## Keyboard Shortcuts

- `Esc` - Exit
- `Ctrl+d` - Collapse/expand details (after task completion)
- `↑/↓` - Navigate confirmation menu
- `Enter` - Confirm menu selection

## Notes

Currently in MVP stage. LLM decision-making uses a deterministic planner (no API Key required), which can be replaced with a real LLM + tools loop in the future.

## Codex Provider Integration (Implemented)

By default, the built-in example planner is used. To generate plans via local Codex at startup:

```bash
GOLDBOT_USE_CODEX=1 GOLDBOT_TASK="Organize large files in current directory" cargo run
```

Notes:
- Generates JSON plan via `codex exec` (provider file: `src/agent/provider.rs`)
- Automatically falls back to example planner if Codex is unavailable or returns an error

## Memory Mechanism

- **Short-term memory** - Written to `memory/YYYY-MM-DD.md` after each task
- **Long-term memory** - Key conclusions appended to `MEMORY.md`
- **Context compression** - Triggered by round threshold, retains compressed summary + recent events to prevent context bloat
