## 1. Type System — Add ConversationCompacted event

- [ ] 1.1 Add `Event::ConversationCompacted { summary: String, messages_dropped: usize }` variant to `src/types.rs`
- [ ] 1.2 Fix all exhaustive `match` sites that the compiler flags after adding the variant (likely in `src/ui/format.rs` and `src/ui/screen.rs`)

## 2. Executor — Improve compaction and emit event

- [ ] 2.1 Extend `summarize_for_compaction` in `src/agent/executor.rs` to extract the last `<final>` block (≤ 400 chars), current `<phase>`, and unique `<memory>` notes from the dropped messages
- [ ] 2.2 Push `Event::ConversationCompacted { summary, messages_dropped }` onto `app.events` at the end of `maybe_flush_and_compact_before_call` (after `app.messages = compacted`)
- [ ] 2.3 Remove the dead `flushed = 0` variable and the `if flushed > 0` branch from `maybe_flush_and_compact_before_call`; simplify `screen.status` assignment to unconditional `"🧠 context compacted"`

## 3. Session Log — Record compaction events

- [ ] 3.1 Add `append_compaction_to_session(&self, summary: &str, messages_dropped: usize) -> Result<()>` to `ProjectStore` in `src/memory/project.rs`; append a markdown block with timestamp, drop count, and summary excerpt (≤ 200 chars)
- [ ] 3.2 Call `ProjectStore::current().append_compaction_to_session(...)` inside `maybe_flush_and_compact_before_call`; handle the `Result` by logging the error without propagating

## 4. UI — Render ConversationCompacted banner

- [ ] 4.1 Add a rendering branch for `Event::ConversationCompacted` in `src/ui/format.rs` that produces a dimmed/grey styled line: `"── Context compacted · N messages dropped ──"` followed by the summary excerpt
- [ ] 4.2 Verify `src/ui/screen.rs` passes the new event through to the format pipeline without filtering it out

## 5. Tests

- [ ] 5.1 Unit-test `summarize_for_compaction` with messages containing `<final>`, `<phase>`, and `<memory>` tags; assert extracted fields appear in the summary string
- [ ] 5.2 Unit-test `append_compaction_to_session` in `project.rs`; assert the session file contains timestamp, drop count, and summary excerpt after a call
- [ ] 5.3 Run `cargo test` and `cargo clippy` to confirm no regressions

## 6. Cleanup

- [ ] 6.1 Audit `src/memory/compactor.rs` — confirm `CompactState` is used by at least one live code path; if it is dead code, remove the struct and the `#![allow(dead_code)]` attr; if live, keep it but remove the attr
