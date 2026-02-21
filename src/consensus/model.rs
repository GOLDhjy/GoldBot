use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};

pub const CONSENSUS_FILE_NAME: &str = "CONSENSUS.md";

const PURPOSE_SECTION: &str = "Purpose";
const RULES_SECTION: &str = "Rules";
const TODO_SECTION: &str = "Todo";
const STATUS_SECTION: &str = "Bot Status";
const JOURNAL_SECTION: &str = "Bot Journal";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TodoItem {
    pub id: String,
    pub text: String,
    pub checked: bool,
    pub done_when: Vec<String>,
    pub assist: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsensusDoc {
    pub purpose_lines: Vec<String>,
    pub rules_lines: Vec<String>,
    pub todos: Vec<TodoItem>,
    pub bot_status_lines: Vec<String>,
    pub bot_journal_lines: Vec<String>,
}

impl ConsensusDoc {
    pub fn parse(text: &str) -> Self {
        let sections = split_sections(text);
        let purpose_lines = sections
            .get(PURPOSE_SECTION)
            .cloned()
            .unwrap_or_else(|| vec!["- Define the shared goal.".to_string()]);
        let rules_lines = sections
            .get(RULES_SECTION)
            .cloned()
            .unwrap_or_else(|| vec!["- Keep edits scoped and test changes.".to_string()]);
        let todos = sections
            .get(TODO_SECTION)
            .map(|lines| parse_todos(lines))
            .unwrap_or_default();
        let bot_status_lines = sections
            .get(STATUS_SECTION)
            .cloned()
            .unwrap_or_else(|| vec!["- Waiting for first run.".to_string()]);
        let bot_journal_lines = sections.get(JOURNAL_SECTION).cloned().unwrap_or_default();

        Self {
            purpose_lines,
            rules_lines,
            todos,
            bot_status_lines,
            bot_journal_lines,
        }
    }

    pub fn render(&self) -> String {
        let mut out = String::new();
        out.push_str("# Consensus\n\n");
        out.push_str("## ");
        out.push_str(PURPOSE_SECTION);
        out.push('\n');
        out.push_str(&render_lines(
            &self.purpose_lines,
            "- Define the shared goal.",
        ));
        out.push('\n');
        out.push_str("## ");
        out.push_str(RULES_SECTION);
        out.push('\n');
        out.push_str(&render_lines(
            &self.rules_lines,
            "- Keep edits scoped and test changes.",
        ));
        out.push('\n');
        out.push_str("## ");
        out.push_str(TODO_SECTION);
        out.push('\n');
        if self.todos.is_empty() {
            out.push_str("- [ ] T001 Define initial todos\n");
            out.push_str("  - done_when: Consensus Todo contains at least 5 clear tasks\n");
        } else {
            for todo in &self.todos {
                let status = if todo.checked { "x" } else { " " };
                if todo.text.trim().is_empty() {
                    out.push_str(&format!("- [{status}] {}\n", todo.id.trim()));
                } else {
                    out.push_str(&format!(
                        "- [{status}] {} {}\n",
                        todo.id.trim(),
                        todo.text.trim()
                    ));
                }
                if todo.done_when.is_empty() {
                    out.push_str("  - done_when: Completed and verified\n");
                } else {
                    for cond in &todo.done_when {
                        out.push_str(&format!("  - done_when: {}\n", cond.trim()));
                    }
                }
                if let Some(assist) = &todo.assist {
                    out.push_str(&format!("  - assist: {}\n", assist.trim()));
                }
            }
        }
        out.push('\n');
        out.push_str("## ");
        out.push_str(STATUS_SECTION);
        out.push('\n');
        out.push_str(&render_lines(
            &self.bot_status_lines,
            "- Waiting for first GE run.",
        ));
        out.push('\n');
        out.push_str("## ");
        out.push_str(JOURNAL_SECTION);
        out.push('\n');
        out.push_str(&render_lines(&self.bot_journal_lines, "- (empty)"));
        out
    }

    pub fn first_open_todo_index(&self) -> Option<usize> {
        self.todos.iter().position(|t| !t.checked)
    }

    pub fn all_done(&self) -> bool {
        !self.todos.is_empty() && self.todos.iter().all(|t| t.checked)
    }

    pub fn mark_checked(&mut self, id: &str) -> bool {
        if let Some(todo) = self.todos.iter_mut().find(|t| t.id == id) {
            todo.checked = true;
            return true;
        }
        false
    }

    pub fn append_status(&mut self, line: impl Into<String>) {
        self.bot_status_lines.push(line.into());
        trim_lines(&mut self.bot_status_lines, 80);
    }

    pub fn append_journal(&mut self, line: impl Into<String>) {
        self.bot_journal_lines.push(line.into());
        trim_lines(&mut self.bot_journal_lines, 200);
    }
}

pub fn consensus_file_path(cwd: &Path) -> PathBuf {
    cwd.join(CONSENSUS_FILE_NAME)
}

pub fn load(path: &Path) -> Result<ConsensusDoc> {
    let raw =
        fs::read_to_string(path).with_context(|| format!("failed to read `{}`", path.display()))?;
    Ok(ConsensusDoc::parse(&raw))
}

pub fn save(path: &Path, doc: &ConsensusDoc) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, doc.render()).with_context(|| format!("failed to write `{}`", path.display()))
}

pub fn build_from_interview(purpose: &str, rules: &str, scope: &str) -> ConsensusDoc {
    let mut purpose_lines = vec![];
    for line in purpose.lines() {
        let trimmed = line.trim();
        if !trimmed.is_empty() {
            purpose_lines.push(format!("- {trimmed}"));
        }
    }
    if !scope.trim().is_empty() {
        purpose_lines.push(format!("- Scope: {}", scope.trim()));
    }
    if purpose_lines.is_empty() {
        purpose_lines.push("- Execute the shared plan continuously.".to_string());
    }

    let mut rules_lines = vec![];
    for line in rules.lines() {
        let trimmed = line.trim();
        if !trimmed.is_empty() {
            rules_lines.push(format!("- {trimmed}"));
        }
    }
    if rules_lines.is_empty() {
        rules_lines.push("- Prefer small verifiable changes.".to_string());
        rules_lines.push("- Keep user-visible behavior stable unless asked.".to_string());
    }

    let todos = vec![
        TodoItem {
            id: "T001".to_string(),
            text: "Create project folder and initialize repository scaffolding.".to_string(),
            checked: false,
            done_when: vec!["cmd: ls".to_string()],
            assist: Some("claude".to_string()),
        },
        TodoItem {
            id: "T002".to_string(),
            text: "Set up core build configuration and dependencies.".to_string(),
            checked: false,
            done_when: vec!["cmd: git status --short".to_string()],
            assist: Some("auto".to_string()),
        },
        TodoItem {
            id: "T003".to_string(),
            text: "Implement first minimal functional slice for the product.".to_string(),
            checked: false,
            done_when: vec!["Completed and verified by Codex review".to_string()],
            assist: Some("auto".to_string()),
        },
        TodoItem {
            id: "T004".to_string(),
            text: "Add user-facing interaction flow for the first slice.".to_string(),
            checked: false,
            done_when: vec!["Completed and verified by Codex review".to_string()],
            assist: Some("auto".to_string()),
        },
        TodoItem {
            id: "T005".to_string(),
            text: "Implement second functional slice and integrate with first.".to_string(),
            checked: false,
            done_when: vec!["Completed and verified by Codex review".to_string()],
            assist: Some("auto".to_string()),
        },
        TodoItem {
            id: "T006".to_string(),
            text: "Run project tests and fix failing checks.".to_string(),
            checked: false,
            done_when: vec!["cmd: cargo check".to_string()],
            assist: Some("codex".to_string()),
        },
        TodoItem {
            id: "T007".to_string(),
            text: "Perform cross-platform smoke verification path.".to_string(),
            checked: false,
            done_when: vec!["Completed and verified by Codex review".to_string()],
            assist: Some("codex".to_string()),
        },
        TodoItem {
            id: "T008".to_string(),
            text: "Document final outcome and next follow-up actions.".to_string(),
            checked: false,
            done_when: vec!["Consensus status and journal updated".to_string()],
            assist: Some("auto".to_string()),
        },
    ];

    ConsensusDoc {
        purpose_lines,
        rules_lines,
        todos,
        bot_status_lines: vec!["- GE initialized and waiting for first execution.".to_string()],
        bot_journal_lines: vec![],
    }
}

fn split_sections(text: &str) -> BTreeMap<String, Vec<String>> {
    let mut sections: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut current: Option<String> = None;

    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("## ") {
            let key = rest.trim().to_string();
            current = Some(key.clone());
            sections.entry(key).or_default();
            continue;
        }
        if let Some(section) = &current {
            sections
                .entry(section.clone())
                .or_default()
                .push(line.to_string());
        }
    }
    sections
}

fn parse_todos(lines: &[String]) -> Vec<TodoItem> {
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < lines.len() {
        let line = lines[i].trim_start();
        let (checked, rest) = if let Some(r) = line.strip_prefix("- [ ] ") {
            (false, r.trim())
        } else if let Some(r) = line.strip_prefix("- [x] ") {
            (true, r.trim())
        } else if let Some(r) = line.strip_prefix("- [X] ") {
            (true, r.trim())
        } else {
            i += 1;
            continue;
        };

        let mut parts = rest.splitn(2, char::is_whitespace);
        let mut id = parts.next().unwrap_or("").trim().to_string();
        if !looks_like_todo_id(&id) {
            id = format!("T{:03}", out.len() + 1);
        }
        let text = parts.next().unwrap_or("").trim().to_string();
        let mut done_when = Vec::new();
        let mut assist = None;
        i += 1;

        while i < lines.len() {
            let sub = lines[i].trim_start();
            if sub.starts_with("- [ ] ")
                || sub.starts_with("- [x] ")
                || sub.starts_with("- [X] ")
                || sub.starts_with("## ")
            {
                break;
            }
            if let Some(v) = sub.strip_prefix("- done_when:") {
                let v = v.trim();
                if !v.is_empty() {
                    done_when.push(v.to_string());
                }
            } else if let Some(v) = sub.strip_prefix("- assist:") {
                let v = v.trim();
                if !v.is_empty() {
                    assist = Some(v.to_string());
                }
            } else if let Some(v) = sub.strip_prefix("  - done_when:") {
                let v = v.trim();
                if !v.is_empty() {
                    done_when.push(v.to_string());
                }
            } else if let Some(v) = sub.strip_prefix("  - assist:") {
                let v = v.trim();
                if !v.is_empty() {
                    assist = Some(v.to_string());
                }
            }
            i += 1;
        }

        out.push(TodoItem {
            id,
            text,
            checked,
            done_when,
            assist,
        });
    }
    out
}

fn looks_like_todo_id(s: &str) -> bool {
    let Some(rest) = s.strip_prefix('T') else {
        return false;
    };
    !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit())
}

fn render_lines(lines: &[String], fallback: &str) -> String {
    let mut out = String::new();
    if lines.iter().all(|l| l.trim().is_empty()) {
        out.push_str(fallback);
        out.push('\n');
        return out;
    }
    for line in lines {
        out.push_str(line);
        out.push('\n');
    }
    out
}

fn trim_lines(lines: &mut Vec<String>, max: usize) {
    if lines.len() <= max {
        return;
    }
    let excess = lines.len() - max;
    lines.drain(..excess);
}

#[cfg(test)]
mod tests {
    use super::{ConsensusDoc, build_from_interview};

    #[test]
    fn parse_and_render_roundtrip_has_todos() {
        let raw = "# Consensus\n\n## Purpose\n- Ship feature\n\n## Rules\n- Keep tests green\n\n## Todo\n- [ ] T001 Do one\n  - done_when: cmd: cargo check\n- [x] T002 Done\n  - assist: codex\n\n## Bot Status\n- idle\n\n## Bot Journal\n- none\n";
        let parsed = ConsensusDoc::parse(raw);
        assert_eq!(parsed.todos.len(), 2);
        assert_eq!(parsed.todos[0].id, "T001");
        assert!(!parsed.todos[0].checked);
        assert!(parsed.todos[1].checked);
        let rendered = parsed.render();
        assert!(rendered.contains("## Todo"));
        assert!(rendered.contains("- [x] T002 Done"));
    }

    #[test]
    fn build_from_interview_creates_eight_todos() {
        let doc = build_from_interview("build x", "rule y", "scope z");
        assert_eq!(doc.todos.len(), 8);
        assert_eq!(doc.todos[0].id, "T001");
        assert!(!doc.todos[0].checked);
    }
}
