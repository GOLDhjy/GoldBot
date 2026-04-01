## ADDED Requirements

### Requirement: Compaction emits a persistent in-panel event
When context compaction fires, the system SHALL emit an `Event::ConversationCompacted` event into `app.events` so that the conversation panel displays a permanent compaction banner for the remainder of the session.

#### Scenario: Banner appears after compaction
- **WHEN** `maybe_flush_and_compact_before_call` determines `should_compact` is true and replaces older messages
- **THEN** an `Event::ConversationCompacted { summary, messages_dropped }` is pushed onto `app.events`
- **THEN** the TUI panel renders a visually distinct separator block (e.g., dimmed/grey) showing "Context compacted — N messages dropped" and the summary excerpt

#### Scenario: Banner persists across redraws
- **WHEN** the TUI is redrawn after compaction
- **THEN** the `ConversationCompacted` banner remains visible in the conversation panel (it is not a transient status-bar message)

#### Scenario: No event emitted when compaction does not trigger
- **WHEN** `maybe_flush_and_compact_before_call` is called but `should_compact` is false
- **THEN** no `ConversationCompacted` event is emitted and the conversation panel is unchanged

### Requirement: Compaction summary includes structured context
The system SHALL extract the last `<final>` output, current phase, and any `<memory>` notes from the messages being compacted to produce the compaction summary injected into `[Context compacted]`.

#### Scenario: Last final output is preserved
- **WHEN** the messages being compacted contain at least one `<final>…</final>` block
- **THEN** the compaction summary includes the content of the most recent final block, truncated to 400 characters

#### Scenario: Phase label is preserved
- **WHEN** the messages being compacted contain a `<phase>…</phase>` tag
- **THEN** the compaction summary includes the most recent phase string

#### Scenario: Memory notes are preserved
- **WHEN** the messages being compacted contain `<memory>…</memory>` tags
- **THEN** the compaction summary includes all unique memory notes from those messages

#### Scenario: Fallback for unstructured content
- **WHEN** the messages being compacted contain no structured tags (`<final>`, `<phase>`, `<memory>`)
- **THEN** the compaction summary falls back to the existing assistant-message text extraction

### Requirement: Compaction event is recorded in the session log
The system SHALL record each compaction event in the current session log file via `ProjectStore::append_compaction_to_session`.

#### Scenario: Session log updated on compaction
- **WHEN** compaction fires successfully
- **THEN** the session log (`~/.goldbot/projects/<workspace>/sessions/<id>.md`) contains a new entry with timestamp, number of messages dropped, and the first 200 characters of the summary

#### Scenario: Session log write failure is non-fatal
- **WHEN** `append_compaction_to_session` returns an error (e.g., disk full, permission denied)
- **THEN** the error is logged to stderr but compaction completes normally and the process does not panic

### Requirement: Dead code removed from compaction path
The system SHALL NOT contain the dead `flushed = 0` variable and associated `"pre-compaction flush"` status branch in `maybe_flush_and_compact_before_call`.

#### Scenario: Dead branch absent
- **WHEN** `maybe_flush_and_compact_before_call` runs
- **THEN** `screen.status` is set to `"🧠 context compacted"` (or equivalent) without branching on a `flushed > 0` condition
