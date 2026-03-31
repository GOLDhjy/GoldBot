## Context

GoldBot compacts conversation history (`maybe_flush_and_compact_before_call` in `executor.rs`) when token budget runs low. The compaction already works correctly — it summarises older messages, replaces them with a `[Context compacted]\n{summary}` user message, and adjusts the message list. However, the only user-visible signal is a fleeting `"🧠 context compacted"` string written to `screen.status`, which disappears on the next render cycle.

The new `ProjectStore` / session-log infrastructure (`src/memory/project.rs`) provides a natural place to persist compaction events alongside task completions. The TUI event model (`src/types.rs` `Event` enum) already supports typed events rendered in the conversation panel, so adding a `ConversationCompacted` variant follows the established pattern.

Dead code remains from an earlier design: `flushed = 0` in `maybe_flush_and_compact_before_call`, and the `CompactState` struct in `compactor.rs` (guarded by `#![allow(dead_code)]`).

## Goals / Non-Goals

**Goals:**
- Emit a `ConversationCompacted` event that persists in the TUI panel when compaction fires.
- Record the compaction event (timestamp, dropped-message count, summary excerpt) in the session log via `ProjectStore`.
- Improve `summarize_for_compaction` to extract the last `<final>` output, current phase, and any `<memory>` tags so the compacted summary is more informative.
- Remove dead code (`flushed`, orphaned `CompactState` usage if confirmed unused).

**Non-Goals:**
- LLM-based summarisation (adds latency and a synchronous API call mid-step; deferred).
- User-initiated compaction (no keybinding for manual compact in this change).
- Changing the compaction trigger heuristic (`dynamic_compact_reserve_tokens` stays as-is).

## Decisions

### D1 — New `Event::ConversationCompacted` variant, not a status string

**Decision:** Add `Event::ConversationCompacted { summary: String, messages_dropped: usize }` to `types.rs` and push it onto `app.events` inside `maybe_flush_and_compact_before_call`.

**Alternatives considered:**
- Continue using `screen.status` — rejected; transient, invisible on the next redraw.
- Inject a fake `Event::Assistant` message — rejected; pollutes the conversation with assistant-role artefact that gets included in future LLM calls.

**Rationale:** Typed events are the established pattern for in-panel notifications (thinking, tool call, tool result). A dedicated variant lets `format.rs` render it distinctively (dimmed border, "Context compacted" label) without adding noise to the LLM message list.

### D2 — Summary extraction: last `<final>`, phase, `<memory>` tags

**Decision:** `summarize_for_compaction` is extended to scan the messages being dropped in reverse order and extract:
1. The most recent `<final>…</final>` block content (truncated to 400 chars).
2. The current phase string (last `<phase>…</phase>` seen).
3. Any `<memory>…</memory>` notes (de-duplicated).

These are assembled into the `[Context compacted]` user message already injected into the retained history.

**Alternatives considered:**
- LLM summarisation call — deferred (latency concern noted above).
- Keep current plain text extraction — retained as fallback when no structured tags are found.

**Rationale:** Structured tags are already parsed throughout the codebase. Extracting them deterministically is zero-latency and produces more actionable context than counting "N thoughts, M tool calls".

### D3 — Session log integration via `ProjectStore::append_compaction_to_session`

**Decision:** Add a new `ProjectStore` method that appends a compact markdown block to the session file: timestamp, messages dropped, and the first 200 chars of the summary.

**Rationale:** Session files already record tasks and diffs; compaction events belong alongside them for post-session audit. The method is a simple append — same pattern as `append_to_session`.

### D4 — Retain `CompactState` for sub-agent use; remove only `flushed` dead code

**Decision:** `compactor.rs` `CompactState` is retained (it may be used by the sub-agent executor path). Only the `flushed = 0` / `pre-compaction flush` dead branch in `executor.rs` is removed.

**Rationale:** Removing `CompactState` entirely risks breaking sub-agent compaction; the `#![allow(dead_code)]` is a signal to investigate, not an instruction to delete.

## Risks / Trade-offs

- **`app.events` grows by one entry per compaction** — negligible; compaction is infrequent and the panel already holds many events.
- **`summarize_for_compaction` regex-style scanning is brittle** — if tag format changes, extraction silently falls back to the existing plain-text path. Acceptable given this is best-effort enrichment.
- **Session log write failure is non-fatal** — `append_compaction_to_session` logs the error and returns; it must not panic or block the compaction path.

## Migration Plan

1. Add `Event::ConversationCompacted` to `types.rs` — existing `match` sites get compiler errors pointing to every exhaustive match arm, making the migration mechanical.
2. Update `executor.rs`: emit event, improve summary, remove dead `flushed` code.
3. Add `ProjectStore::append_compaction_to_session`.
4. Update `format.rs` / `screen.rs` to render the new event.
5. Run `cargo check` then `cargo test` — no data migration needed.

## Open Questions

- Should `ConversationCompacted` be hidden from the LLM's message list? Currently it only goes into `app.events` (the display list), not `app.messages`, so yes — LLM never sees it. Confirm this is the desired separation.
- Is `CompactState` actually exercised by any live code path, or is the `#![allow(dead_code)]` stale? Worth a grep before removing.
