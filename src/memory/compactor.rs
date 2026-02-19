use crate::types::Event;

#[derive(Debug, Clone)]
pub struct CompactState {
    pub rounds: usize,
    pub max_rounds_before_compact: usize,
    pub memory_summary: String,
}

impl CompactState {
    pub fn new(max_rounds_before_compact: usize) -> Self {
        Self {
            rounds: 0,
            max_rounds_before_compact,
            memory_summary: String::new(),
        }
    }

    pub fn tick_and_maybe_compact(&mut self, events: &mut Vec<Event>) -> Option<String> {
        self.rounds += 1;
        if self.rounds < self.max_rounds_before_compact {
            return None;
        }

        self.rounds = 0;
        let summary = summarize_events(events);
        self.memory_summary = summary.clone();

        let tail: Vec<Event> = events.iter().rev().take(4).cloned().collect();
        events.clear();
        events.push(Event::Thinking {
            text: format!("上下文已压缩：{}", self.memory_summary),
        });
        for e in tail.into_iter().rev() {
            events.push(e);
        }

        Some(summary)
    }
}

fn summarize_events(events: &[Event]) -> String {
    let mut thoughts = 0;
    let mut calls = 0;
    let mut last_result = String::new();
    for e in events {
        match e {
            Event::Thinking { .. } => thoughts += 1,
            Event::ToolCall { .. } => calls += 1,
            Event::ToolResult { output, .. } => last_result = output.clone(),
            _ => {}
        }
    }
    format!("{} thoughts, {} tool calls, last_result: {}", thoughts, calls, trim(&last_result, 120))
}

fn trim(s: &str, n: usize) -> String {
    if s.len() <= n {
        s.to_string()
    } else {
        format!("{}...", &s[..n])
    }
}
