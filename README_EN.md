# GoldBot - AI Terminal Automation Agent

A cross-platform TUI Agent built with Rust that automatically plans and executes shell commands via LLM to complete tasks.

[简体中文版](README.md)

## Features

- **Three-Level Safety Control**: Safe/Confirm/Block
- **Persistent Memory**: Short-term (daily) + Long-term (auto-extracted preferences)
- **ReAct Loop**: Think → Act → Observe → Think again
- **Smart Command Classification**: Read/Write/Update/Search/Bash
- **Real-time TUI**: Streamed thinking process, collapsed by default after completion
- **Cross-Platform**: macOS/Linux (bash) / Windows (PowerShell)

## Installation

### One-Line Install (Recommended)

**macOS / Linux**

```bash
curl -fsSL https://raw.githubusercontent.com/GOLDhjy/GoldBot/main/scripts/install.sh | bash
```

Or via Homebrew:

```bash
brew install GOLDhjy/GoldBot/goldbot
```

### Build from Source

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

### Main Event Loop (main.rs)

```text
loop {
    1. Handle LLM streaming response (update thinking preview)
    2. Trigger Agent step (async LLM API call)
    3. Handle keyboard input (Ctrl+C/D, ↑/↓/Enter)
}
```

### ReAct Loop Flow

```text
User enters task
  → start_task() (reset state, set needs_agent_step=true)
  → LLM call (send System Prompt + history)
  → process_llm_result() (parse <thought> and <tool>/<final>)
  
  Branches:
  ├─ <tool>shell → execute_command()
  │      ├─ Safe → Execute directly
  │      ├─ Confirm → Popup menu
  │      └─ Block → Reject
  │           → Add result to history → needs_agent_step=true (loop)
  │
  └─ <final> → finish()
       → Save memory → Collapse display → running=false (exit)
```

### Safety Assessment

```text
Block: sudo, format, fork bomb
Confirm: rm, mv, cp, git commit, curl, wget, >, >>
Safe: ls, cat, grep, git status
```

### Memory Mechanism

- **Short-term**: ~/.goldbot/memory/YYYY-MM-DD.md (full conversations)
- **Long-term**: ~/.goldbot/MEMORY.md (preferences/rules only, deduplicated)
- **Startup injection**: Auto-load long-term memory + recent 2-day short-term memory

## Usage

```bash
goldbot
```

### Keyboard Shortcuts

- `Ctrl+C` - Exit
- `Ctrl+D` - Collapse/expand details
- `↑/↓/Enter` - Navigate confirmation menu

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
