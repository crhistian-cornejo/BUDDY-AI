//! Where Buddy keeps its data: `~/Library/Application Support/Buddy` (Mac), `%LOCALAPPDATA%\Buddy` (Windows).

use std::path::PathBuf;

pub fn default_data_dir() -> PathBuf {
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
