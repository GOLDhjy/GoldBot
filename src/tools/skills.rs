use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
};


#[derive(Debug, Clone)]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub content: String,
}

// Project-local skill directories (searched from cwd up to git root).
const LOCAL_SUBDIRS: &[&str] = &[".claude/skills", ".agents/skills", ".opencode/skills"];

// Global skill directories. GoldBot's own dir (~/.goldbot/skills) is checked first.
const GLOBAL_SUBDIRS: &[&str] = &[
    ".config/opencode/skills",
    ".claude/skills",
    ".agents/skills",
];

/// Returns GoldBot's own skills directory: `$GOLDBOT_MEMORY_DIR/skills` or `~/.goldbot/skills`.
pub fn goldbot_skills_dir() -> PathBuf {
    crate::tools::mcp::goldbot_home_dir().join("skills")
}

/// Discover all skills. Priority: project-local → GoldBot own → other global.
/// First occurrence of each name wins.
pub fn discover_skills() -> Vec<Skill> {
    let mut skills = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    // Project-local: walk from cwd up to git root.
    if let Ok(cwd) = std::env::current_dir() {
        for dir in walk_to_git_root(&cwd) {
            for sub in LOCAL_SUBDIRS {
                scan_dir(&dir.join(sub), &mut skills, &mut seen);
            }
        }
    }

    // GoldBot's own skills directory.
    scan_dir(&goldbot_skills_dir(), &mut skills, &mut seen);

    // Other global dirs under $HOME.
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        for sub in GLOBAL_SUBDIRS {
            scan_dir(&home.join(sub), &mut skills, &mut seen);
        }
    }

    skills
}

/// Build the system-prompt section that describes available skills.
pub fn skills_system_prompt(skills: &[Skill]) -> String {
    if skills.is_empty() {
        return String::new();
    }
    let mut out = String::from(
        "\n\n## Available Skills\n\
         If the user's task matches one of the skills below, you MUST load it FIRST \
         before taking any other action. Loading a skill gives you specialized instructions \
         for that task — do not attempt the task without loading the relevant skill first.\n\
         <thought>this task matches skill X</thought>\n\
         <skill>skill-name</skill>\n\n\
         Skills:\n",
    );
    for s in skills {
        out.push_str(&format!("- {}: {}\n", s.name, s.description));
    }
    out
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn walk_to_git_root(start: &Path) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let mut cur = start.to_path_buf();
    loop {
        dirs.push(cur.clone());
        if cur.join(".git").exists() {
            break;
        }
        match cur.parent() {
            Some(p) if p != cur => cur = p.to_path_buf(),
            _ => break,
        }
    }
    dirs
}

fn scan_dir(dir: &Path, skills: &mut Vec<Skill>, seen: &mut HashSet<String>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let skill_file = path.join("SKILL.md");
        if !skill_file.exists() {
            continue;
        }
        if let Some(skill) = parse_skill(&skill_file, &path) {
            if seen.insert(skill.name.clone()) {
                skills.push(skill);
            }
        }
    }
}

fn parse_skill(file: &Path, dir: &Path) -> Option<Skill> {
    let raw = fs::read_to_string(file).ok()?;
    let (meta, body) = parse_frontmatter(&raw)?;

    let name = meta.get("name")?.trim().to_string();
    if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return None;
    }
    // Frontmatter name must match the directory name.
    if dir.file_name().and_then(|n| n.to_str()) != Some(name.as_str()) {
        return None;
    }

    let description = meta
        .get("description")
        .map(|s| s.trim().to_string())
        .unwrap_or_default();

    Some(Skill {
        name,
        description,
        content: body.trim().to_string(),
    })
}

fn parse_frontmatter(content: &str) -> Option<(HashMap<String, String>, String)> {
    let mut lines = content.lines();
    if lines.next()?.trim() != "---" {
        return None;
    }
    let mut meta = HashMap::new();
    let mut body_lines: Vec<&str> = Vec::new();
    let mut in_body = false;
    for line in lines {
        if !in_body && line.trim() == "---" {
            in_body = true;
            continue;
        }
        if in_body {
            body_lines.push(line);
        } else if let Some((k, v)) = line.split_once(':') {
            meta.insert(k.trim().to_string(), v.trim().to_string());
        }
    }
    if !in_body {
        return None;
    }
    Some((meta, body_lines.join("\n")))
}
