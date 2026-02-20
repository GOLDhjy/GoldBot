use std::{fs, path::PathBuf};

use anyhow::Result;
use chrono::Local;

pub struct MemoryStore {
    base: PathBuf,
}

impl MemoryStore {
    pub fn new() -> Self {
        Self {
            base: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
        }
    }

    fn short_term_path(&self) -> PathBuf {
        let day = Local::now().format("%Y-%m-%d").to_string();
        self.base.join("memory").join(format!("{day}.md"))
    }

    fn long_term_path(&self) -> PathBuf {
        self.base.join("MEMORY.md")
    }

    pub fn append_short_term(&self, task: &str, final_output: &str) -> Result<()> {
        let path = self.short_term_path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let now = Local::now().format("%H:%M:%S");
        let block = format!("\n## {now}\n- task: {task}\n- final: {final_output}\n");
        append_file(path, &block)
    }

    pub fn append_long_term(&self, note: &str) -> Result<()> {
        let path = self.long_term_path();
        let block = format!("\n- {}\n", note.trim());
        append_file(path, &block)
    }
}

fn append_file(path: PathBuf, content: &str) -> Result<()> {
    use std::io::Write;
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    file.write_all(content.as_bytes())?;
    Ok(())
}
