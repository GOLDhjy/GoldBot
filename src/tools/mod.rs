pub mod mcp;
pub mod safety;
pub mod shell;
pub mod skills;
pub mod web_search;

use std::path::PathBuf;

/// Returns the current user's home directory in a cross-platform way.
/// - Unix/macOS: `$HOME`
/// - Windows: `$USERPROFILE`, then `$HOMEDRIVE$HOMEPATH`
pub fn home_dir() -> Option<PathBuf> {
    if let Some(home) = std::env::var_os("HOME") {
        return Some(PathBuf::from(home));
    }
    if cfg!(target_os = "windows") {
        if let Some(profile) = std::env::var_os("USERPROFILE") {
            return Some(PathBuf::from(profile));
        }
        if let (Some(drive), Some(path)) =
            (std::env::var_os("HOMEDRIVE"), std::env::var_os("HOMEPATH"))
        {
            let mut p = PathBuf::from(drive);
            p.push(path);
            return Some(p);
        }
    }
    None
}
