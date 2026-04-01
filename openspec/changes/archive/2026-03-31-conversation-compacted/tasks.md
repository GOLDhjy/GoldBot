## 1. Type System — Add ConversationCompacted event

- [x] 1.1 Add `Event::ConversationCompacted { summary: String, messages_dropped: usize }` variant to `src/types.rs`
- [x] 1.2 Fix all exhaustive `match` sites that the compiler flags after adding the variant (likely in `src/ui/format.rs` and `src/ui/screen.rs`)

## 2. Executor — LLM-driven compaction and emit event

- [x] 2.1 Add `compact_system_prompt() -> &'static str` in `src/agent/executor.rs` returning the dedicated compaction prompt (plain-text summary, no tool tags, ≤ 300 words)
- [x] 2.2 Add `async fn llm_summarize(messages: &[Message], client: &reqwest::Client) -> Result<String>` that calls `chat_stream_with` (or a non-streaming variant) on the current GLM backend with the compaction prompt; this call is **synchronous / `.await`-ed** inside `maybe_flush_and_compact_before_call`, blocking the step until the summary returns or an error is received
- [x] 2.3 Add `fn summarize_fallback(messages: &[Message]) -> String` that scans dropped messages in reverse for the last `<final>` block (≤ 400 chars), last `<phase>` string, and unique `<memory>` notes; returns a plain-text string prefixed with `[context compaction failed, messages truncated]\n`
- [x] 2.4 In `maybe_flush_and_compact_before_call`: call `llm_summarize`; on `Ok` use the result as the summary; on `Err` log with `eprintln!` and call `summarize_fallback`; then push `Event::ConversationCompacted { summary, messages_dropped }` onto `app.events`
- [x] 2.5 Remove the dead `flushed = 0` variable and the `if flushed > 0` branch; simplify `screen.status` assignment to unconditional `"🧠 context compacted"`

## 3. Session Log — Record compaction events

- [x] 3.1 Add `append_compaction_to_session(&self, summary: &str, messages_dropped: usize) -> Result<()>` to `ProjectStore` in `src/memory/project.rs`; append a markdown block with timestamp, drop count, and summary excerpt (≤ 200 chars)
- [x] 3.2 Call `ProjectStore::current().append_compaction_to_session(...)` inside `maybe_flush_and_compact_before_call`; handle the `Result` by logging the error without propagating

## 4. UI — Render ConversationCompacted banner

- [x] 4.1 Add a rendering branch for `Event::ConversationCompacted` in `src/ui/format.rs` that produces a dimmed/grey styled line: `"── Context compacted · N messages dropped ──"` followed by the summary excerpt
- [x] 4.2 Verify `src/ui/screen.rs` passes the new event through to the format pipeline without filtering it out

## 5. Tests

- [x] 5.1 Unit-test `summarize_for_compaction` with messages containing `<final>`, `<phase>`, and `<memory>` tags; assert extracted fields appear in the summary string
- [x] 5.2 Unit-test `append_compaction_to_session` in `project.rs`; assert the session file contains timestamp, drop count, and summary excerpt after a call
- [x] 5.3 Run `cargo test` and `cargo clippy` to confirm no regressions

## 6. Cleanup

- [x] 6.1 Audit `src/memory/compactor.rs` — confirm `CompactState` is used by at least one live code path; if it is dead code, remove the struct and the `#![allow(dead_code)]` attr; if live, keep it but remove the attr
