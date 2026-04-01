## Context

GoldBot compacts conversation history (`maybe_flush_and_compact_before_call` in `executor.rs`) when token budget runs low. The compaction already works correctly — it summarises older messages, replaces them with a `[Context compacted]\n{summary}` user message, and adjusts the message list. However, the only user-visible signal is a fleeting `"🧠 context compacted"` string written to `screen.status`, which disappears on the next render cycle.

The new `ProjectStore` / session-log infrastructure (`src/memory/project.rs`) provides a natural place to persist compaction events alongside task completions. The TUI event model (`src/types.rs` `Event` enum) already supports typed events rendered in the conversation panel, so adding a `ConversationCompacted` variant follows the established pattern.

Dead code remains from an earlier design: `flushed = 0` in `maybe_flush_and_compact_before_call`, and the `CompactState` struct in `compactor.rs` (guarded by `#![allow(dead_code)]`).

## Goals / Non-Goals

**Goals:**
- Emit a `ConversationCompacted` event that persists in the TUI panel when compaction fires.
- Record the compaction event (timestamp, dropped-message count, summary excerpt) in the session log via `ProjectStore`.
- **LLM-driven summarisation**: call the same GLM backend with a dedicated compaction system prompt to generate a structured natural-language summary; use it as the `[Context compacted]` block injected into the retained history. The call is synchronous and blocks the current step until the summary returns.
- On LLM failure, fall back to deterministic tag extraction (last `<final>`, `<phase>`, `<memory>` notes) and surface `[context compaction failed, messages truncated]` in the UI event; never propagate the error to the main task flow.
- Remove dead code (`flushed`, orphaned `CompactState` usage if confirmed unused).

**Non-Goals:**
- User-initiated compaction (no keybinding for manual compact in this change).
- Changing the compaction trigger heuristic (`dynamic_compact_reserve_tokens` stays as-is).
- Introducing a separate lighter model for compaction (add `GOLDBOT_COMPACT_MODEL` env var later if needed).

## Decisions

### D1 — New `Event::ConversationCompacted` variant, not a status string

**Decision:** Add `Event::ConversationCompacted { summary: String, messages_dropped: usize }` to `types.rs` and push it onto `app.events` inside `maybe_flush_and_compact_before_call`.

**Alternatives considered:**
- Continue using `screen.status` — rejected; transient, invisible on the next redraw.
- Inject a fake `Event::Assistant` message — rejected; pollutes the conversation with assistant-role artefact that gets included in future LLM calls.

**Rationale:** Typed events are the established pattern for in-panel notifications (thinking, tool call, tool result). A dedicated variant lets `format.rs` render it distinctively (dimmed border, "Context compacted" label) without adding noise to the LLM message list.

### D2 — LLM-driven summarisation with tag-extraction fallback

**Decision:** Replace `summarize_for_compaction` with an async-blocking LLM call using a dedicated compaction system prompt (separate from the GoldBot agent prompt). The prompt instructs the model to output plain text only (no tool tags). The call reuses the same GLM backend and HTTP client already in use.

**Compaction system prompt template:**
```
You are a conversation summarizer. The following messages are about to be discarded from an AI agent's context window. Produce a concise structured summary (≤ 300 words) covering:
1. Main task goal
2. Key decisions and outcomes
3. Important tool results or file changes
4. Current progress state and next intended step

Output plain text only. No XML tags, no markdown headings, no tool calls.
```

**Failure handling:** If the LLM call fails (network error, timeout, non-200 response), fall back to deterministic extraction: scan dropped messages in reverse for the last `<final>` block (≤ 400 chars), the last `<phase>` string, and unique `<memory>` notes. Prefix the injected block with `[context compaction failed, messages truncated]` so the LLM knows summarisation was degraded. The error is logged (`eprintln!`) but not propagated — it must not crash or stall the main task.

**Alternatives considered:**
- Reuse GoldBot system prompt — rejected; contains tool definitions that could cause the model to emit `<tool>` tags instead of a plain summary, wasting tokens.
- Introduce a lighter model via `GOLDBOT_COMPACT_MODEL` — deferred; adds config complexity now, easy to add later.
- Keep current plain text extraction only — rejected; produces low-quality context ("N thoughts, M tool calls") that doesn't help the LLM resume work.

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
