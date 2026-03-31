# GoldBot — Copilot Instructions

GoldBot is a cross-platform Rust TUI agent that uses LLMs to automate shell command execution via a ReAct (Reasoning-Acting) loop.

## Build, Test & Lint

```bash
cargo build                          # debug build
cargo build --release                # release build
cargo check                          # fast compile check, no binary
cargo run                            # launch TUI
GOLDBOT_TASK="describe task" cargo run  # start with a pre-filled task

cargo test                           # all tests
cargo test rm_requires_confirmation  # single test by name pattern
cargo test tools::safety::tests::rm_requires_confirmation  # exact module path
cargo test -- --nocapture            # show println! in tests

cargo clippy --all-targets --all-features   # lint
cargo fmt                            # format (run before committing)
```

## Architecture

### ReAct Loop

`main.rs` owns the `App` struct and event loop. When a task starts (`agent/step.rs: start_task`):

1. `maybe_flush_and_compact_before_call()` — at 48+ messages, compacts older context into a summary block, keeping the 18 most recent.
2. LLM is called via `provider.rs: chat_stream_with()` — streaming over SSE, two channels: `content` (text) and `reasoning_content` (thinking).
3. The raw response is parsed by `agent/react.rs: parse_llm_response()` into `(thought, Vec<LlmAction>)`.
4. `process_llm_result()` dispatches each `LlmAction` in document order, stopping at the first blocking action (shell, question, final, etc.).

### LLM Tool Protocol

The LLM responds with XML-style tags. `parse_llm_response()` extracts them in this priority order:

1. `<final>` / `<skill>` / `<create_mcp>` — handled directly, not wrapped in `<tool>`.
2. `<memory>` — non-blocking, collected alongside any other action.
3. `<tool>toolname</tool>` + companion tags — all tool calls in document order.
4. Bare `<command>` — backward-compat fallback.

Key blocking tools: `shell`, `question`, `final`, `sub_agent`, `task`.  
Non-blocking tools (may combine with blocking): `phase`, `memory`, `set_mode`.  
File tools: `read`, `write`, `update`, `search`, `glob`.

### Provider System

`agent/provider.rs` defines `Message`, `Role`, `Usage`, and `build_http_client()`.  
Provider implementations: `provider_glm.rs`, `provider_kimi.rs`, `provider_minimax.rs`.  
`BACKEND_PRESETS` in `provider.rs` lists all selectable backends and their model names (used by the `/model` switcher).  
The active provider is selected at runtime via environment variables or the `/model` command.

### Safety Assessment

Every shell command passes through `tools/safety.rs: assess_command()` → `RiskLevel`:
- `Block` — hard stops (e.g., `sudo`, `format`, `:(){` fork bombs).
- `Confirm` — user must approve (e.g., `rm`, `mv`, `curl`, output redirection `>`/`>>`).
- `Safe` — read-only, executes immediately.

### Memory System

- **Short-term**: `~/.goldbot/memory/YYYY-MM-DD.md` — auto-updated daily logs including file change diffs.
- **Long-term**: project-scoped `MEMORY.md` at the workspace root — written by `<memory>` tags.
- **Injection**: Both are injected into the system prompt on each LLM call (last 30 long-term entries + 2 days short-term).
- **Compaction**: `memory/compactor.rs` handles context compression at the 48-message threshold.

### GE (Golden Experience) Mode

Triggered by prefixing input with `GE`. A structured multi-model supervision workflow:

1. If no `CONSENSUS.md` exists, runs a 3-question interview (Purpose / Rules / Scope) via `consensus/engine.rs`.
2. LLM generates 8–12 granular todo steps.
3. Per todo: Claude executes → Codex optimizes → GoldBot self-reviews (`consensus/evaluate.rs`).
4. Auto-commits after each completed todo (excluding `GE_LOG.jsonl`).
5. All events are appended to `GE_LOG.jsonl` via `consensus/audit.rs`.

`Mode` enum (in `types.rs`): `Normal` | `GeInterview` | `GeRun` | `GeIdle`.

### Sub-Agent DAG

The `<tool>sub_agent</tool>` action dispatches a dependency graph of typed sub-agents (`agent/sub_agent.rs`). Nodes have roles (`search`, `coding`, `analysis`, `writer`, `reviewer`, `docs`), optional `depends_on` lists, and `input_merge` strategies (`concat` / `structured`). The graph executes with topological ordering; independent nodes run in parallel.

### MCP Integration

MCP servers discovered from `~/.goldbot/mcp_servers.json` (or `GOLDBOT_MCP_SERVERS` env var) at startup. Each stdio server is registered, its tools extracted, and the system prompt augmented with the available tool list (`tools/mcp.rs: augment_system_prompt()`). The LLM calls MCP tools via `<tool>mcp_<server>_<tool></tool>`.

## Key Conventions

### Shared Types

All cross-module domain types live in `src/types.rs` — import from there, not from intermediate modules. When adding a new action variant, add it to both `LlmAction` (in `types.rs`) and the dispatch match in `agent/step.rs`.

### Error Handling

```rust
// Use anyhow::Result<T> for fallible functions
use anyhow::{anyhow, Context, Result};

let content = fs::read_to_string(&path).context("failed to read config file")?;
// Use anyhow! for actionable error messages — never include secrets/tokens
```

### Constants

Module-level constants at the top of the file in `SCREAMING_SNAKE_CASE`:

```rust
const MAX_OUTPUT_CHARS: usize = 10_000;
const DEFAULT_CMD_TIMEOUT_SECS: u64 = 120;
```

### Testing Pattern

Tests live in `#[cfg(test)] mod tests` at the bottom of the source file. Names describe behavior:

```rust
#[test]
fn rm_requires_confirmation() {
    let (risk, _) = assess_command("rm README.md");
    assert_eq!(risk, RiskLevel::Confirm);
}
```

Use `memory/store.rs: unique_base()` to create isolated temp directories in tests; clean up after.

### Imports

```rust
use std::{collections::HashMap, path::PathBuf};
use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use crate::types::LlmAction;
```

### Commits

Follow Conventional Commits: `feat: ...`, `fix: ...`, `refactor: ...`. Include `cargo test` + `cargo clippy` results in PR descriptions.

## Environment Variables

| Variable | Default | Purpose |
|---|---|---|
| `BIGMODEL_API_KEY` | — | GLM API key (required) |
| `BIGMODEL_BASE_URL` | `https://open.bigmodel.cn/api/coding/paas/v4` | API endpoint |
| `BIGMODEL_MODEL` | `GLM-4.7` | Model name |
| `BOCHA_API_KEY` | — | Web search via Bocha AI |
| `GOLDBOT_TASK` | — | Pre-filled task on startup |
| `GOLDBOT_MCP_SERVERS` | — | MCP config JSON (inline) |
| `GOLDBOT_MCP_SERVERS_FILE` | `~/.goldbot/mcp_servers.json` | MCP config path |
| `HTTP_PROXY` | — | HTTP proxy |
| `API_TIMEOUT_MS` | — | Request timeout in ms |

Config auto-created at `~/.goldbot/.env` on first run.

## Runtime Artifacts (Do Not Commit)

- `target/` — build output
- `GE_LOG.jsonl` — GE mode audit log
- `memory/` — short-term memory logs
- `MEMORY.md` — long-term project memory (workspace-scoped)
