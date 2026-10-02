//! Where Buddy keeps its data: `$BUDDY_DATA_DIR` when set, else `~/Library/Application Support/Buddy` (Mac),
//! `%LOCALAPPDATA%\Buddy` (Windows), `~/.local/share/buddy` (other Unix). The hook relay (`buddy-hook`) resolves the
//! same folder to find the socket, so both must agree.

use std::path::{Path, PathBuf};

pub fn default_data_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("BUDDY_DATA_DIR").filter(|v| !v.is_empty()) {
        return PathBuf::from(dir);
    }
    #[cfg(target_os = "windows")]
    {
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            return PathBuf::from(local).join("Buddy");
        }
    }
    #[cfg(target_os = "macos")]
    {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join("Library/Application Support/Buddy");
        }
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join(".local/share/buddy");
        }
    }
    std::env::temp_dir().join("Buddy")
}

/// The stable copy of the hook relay that the agents' config files point at: `<data_dir>/bin/buddy-hook[.exe]`.
pub fn relay_path(data_dir: &Path) -> PathBuf {
    data_dir.join("bin").join(if cfg!(windows) { "buddy-hook.exe" } else { "buddy-hook" })
}

/// The Unix socket the relay talks to (Mac and other Unix; Windows uses a named pipe).
pub fn hooks_socket_path(data_dir: &Path) -> PathBuf {
    data_dir.join("hooks.sock")
}
