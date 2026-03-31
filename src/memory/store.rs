use std::path::PathBuf;

const ENV_MEMORY_DIR: &str = "GOLDBOT_MEMORY_DIR";

/// Base directory for all GoldBot persistent data (`~/.goldbot` by default).
/// Shared by ProjectStore, MCP config, etc.
pub fn default_memory_base_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os(ENV_MEMORY_DIR) {
        let p = PathBuf::from(dir);
        if !p.as_os_str().is_empty() {
            return p;
        }
    }
    if let Some(home) = crate::tools::home_dir() {
        return home.join(".goldbot");
    }
    PathBuf::from(".goldbot")
}
