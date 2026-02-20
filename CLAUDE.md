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

# Run with Codex provider and custom task
GOLDBOT_USE_CODEX=1 GOLDBOT_TASK="整理当前目录的大文件" cargo run

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

GoldBot is a **cross-platform TUI agent** that executes a pre-planned list of shell commands, displays progress interactively, and persists results to memory files. It is built with `ratatui` + `crossterm` for the terminal UI and runs on the `2024` Rust edition.

### Execution Flow

`main.rs` owns the event loop. At startup it calls `plan_from_codex_or_sample()` (in `src/agent/loop.rs`) to get a `Vec<PlanStep>`, then drives `AppState` step-by-step:

1. Each step emits a `Thinking` event, then the command is assessed by `safety::policy::assess_command`.
2. **Safe** → executed immediately via `tools::runner::run_command`.
3. **Confirm** → pauses and renders a popup menu; user picks Execute / Edit / Skip / Abort with arrow keys + Enter.
4. **Block** → command is rejected outright (e.g. `sudo`, fork-bomb patterns).
5. After every step, `CompactState::tick_and_maybe_compact` fires every 8 rounds to truncate the `events` vec and prepend a summary, preventing unbounded growth.
6. When all steps finish (or user aborts), `finish()` writes to both memory stores and sets `app.running = false`.

### Module Map

| Module | Purpose |
|---|---|
| `src/main.rs` | Entry point, event loop, keybinding handler, step dispatcher |
| `src/types.rs` | `Event` enum (Thinking/ToolCall/ToolResult/NeedsConfirmation/Final), `ConfirmationChoice` |
| `src/app/state.rs` | `AppState` — all runtime state (plan, index, events, UI flags) |
| `src/agent/loop.rs` | `PlanStep` struct, `sample_plan()`, `plan_from_codex_or_sample()` |
| `src/agent/provider.rs` | `CodexProvider` — shells out to `codex exec` CLI to generate plans or decide next actions |
| `src/safety/policy.rs` | `assess_command()` — classifies commands as Safe / Confirm / Block |
| `src/tools/runner.rs` | `run_command()` — executes via `bash -lc` (Unix) or `powershell -NoProfile -Command` (Windows), caps output at 8 KB |
| `src/memory/store.rs` | `MemoryStore` — appends short-term logs to `memory/YYYY-MM-DD.md` and long-term notes to `MEMORY.md` |
| `src/memory/compactor.rs` | `CompactState` — periodic in-memory context compaction |
| `src/ui/mod.rs` | `draw()` — ratatui rendering: header bar, event log / collapsed summary, footer, confirmation popup |

### Plan Sources

- **Default (sample):** `sample_plan()` in `src/agent/loop.rs` returns a hardcoded 3-step plan.
- **Codex provider:** When `GOLDBOT_USE_CODEX=1`, `CodexProvider::build_plan()` calls `codex exec` with a JSON-structured prompt and parses the response. Falls back to sample plan on any error.
- `GOLDBOT_TASK` env var sets the task description (default: `"整理当前目录并汇总文件信息"`).

### Memory Files

- `memory/YYYY-MM-DD.md` — short-term, one file per day, timestamped entries.
- `MEMORY.md` — long-term, append-only, one bullet per completed task.

### Key Keybindings

| Key | Action |
|---|---|
| `q` | Quit |
| `d` | Toggle detail/collapsed view (after task completes) |
| `↑` / `↓` | Navigate confirmation menu |
| `Enter` | Confirm menu selection |

## Current State Note

`src/agent/loop.rs` has uncommitted changes that removed `plan_from_codex_or_sample()`, but `src/main.rs` still imports and calls it. The project will not compile until this function is restored in `loop.rs` or `main.rs` is updated to call `sample_plan()` directly.
