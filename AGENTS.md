# Repository Guidelines

## Project Structure & Module Organization
`GoldBot` is a Rust TUI agent. Core code lives in `src/`:
- `src/main.rs`: app entrypoint, terminal lifecycle, event loop.
- `src/agent/`: LLM response parsing (`react.rs`) and provider/http integration (`provider.rs`).
- `src/tools/`: shell execution and command safety checks.
- `src/tui/`: UI state and rendering logic.
- `src/memory/`: memory persistence and compaction.
- `src/types.rs`: shared enums/types across modules.

Runtime artifacts are stored in `memory/` (daily logs like `memory/2026-02-20.md`) and `MEMORY.md` (long-term notes). Build output is under `target/` and should not be committed.

## Build, Test, and Development Commands
- `cargo run`: start the TUI locally.
- `GOLDBOT_USE_CODEX=1 GOLDBOT_TASK="整理当前目录的大文件" cargo run`: run with Codex provider and a preset task.
- `cargo check`: fast compile validation.
- `cargo test`: run tests (currently minimal; add tests with new logic).
- `cargo clippy --all-targets --all-features`: lint for correctness/style issues.
- `cargo fmt`: format code before commit.

## Coding Style & Naming Conventions
Use Rust 2024 idioms and `rustfmt` defaults (4-space indentation). Naming:
- `snake_case` for modules, files, and functions.
- `PascalCase` for structs/enums/traits.
- `SCREAMING_SNAKE_CASE` for constants.

Prefer small focused modules and keep shared domain types in `src/types.rs`. Use `anyhow::Result` at boundaries and add clear context for I/O/network failures.

## Testing Guidelines
Place unit tests near code with `#[cfg(test)] mod tests`. Name tests by behavior, e.g. `assess_command_blocks_sudo` or `parse_llm_response_requires_tags`. For parser/safety/memory updates, cover both success and failure cases. Run `cargo test` and `cargo clippy --all-targets --all-features` before opening a PR.

## Commit & Pull Request Guidelines
Follow Conventional Commit style used in history (`feat: ...`, `fix: ...`). Keep each commit scoped to one change. PRs should include:
- clear summary and motivation,
- validation output (`cargo test`, `cargo clippy`),
- terminal screenshots/GIFs for TUI rendering changes,
- linked issue/task and any env/config updates (for example `GOLDBOT_USE_CODEX`, `API_TIMEOUT_MS`).
