## Why

When GoldBot compacts context to free token budget, users receive only a fleeting status-bar flash (`"🧠 context compacted"`). There is no persistent, in-conversation record that compaction happened, what messages were discarded, or what summary was preserved — leaving users confused about gaps in conversation history. With the new session-log infrastructure (`ProjectStore`) now in place, this is the right moment to surface compaction as a first-class event.

## What Changes

- Add a `ConversationCompacted` variant to the `Event` enum in `types.rs` carrying the compaction summary and message count.
- Emit this event into the TUI event stream whenever `maybe_flush_and_compact_before_call` runs, so it renders as a permanent banner in the conversation panel rather than a transient status-bar message.
- Improve `summarize_for_compaction` to extract higher-quality context: preserve the last `<final>` output, phase label, and any `<memory>` tags emitted before compaction.
- Record compaction events in the session log (`ProjectStore::append_compaction_to_session`) so post-session review shows what was dropped.
- Remove the dead `flushed = 0` / `CompactState` dead-code artefacts left from earlier iterations.

## Capabilities

### New Capabilities

- `conversation-compacted`: Persistent in-conversation compaction event with summary banner rendered in the TUI panel; replaces the transient status-bar flash.

### Modified Capabilities

*(none — no existing spec-level behaviour changes)*

## Impact

- **`src/types.rs`** — new `Event::ConversationCompacted { summary: String, messages_dropped: usize }` variant.
- **`src/agent/executor.rs`** — `maybe_flush_and_compact_before_call` emits the new event; `summarize_for_compaction` improved; dead `flushed`/`CompactState` references removed.
- **`src/memory/project.rs`** — new `append_compaction_to_session(summary, dropped)` method on `ProjectStore`.
- **`src/ui/format.rs`** / **`src/ui/screen.rs`** — render `ConversationCompacted` as a styled separator block.
- **`src/memory/compactor.rs`** — `CompactState` retained for round-based sub-agent compaction but `summarize_events` updated to align with executor's richer format.
- No API changes, no breaking changes to environment variables or config.
