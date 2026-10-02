//! Child processes for the provider CLIs: where they are installed, a clean environment, and stopping them.

use std::ffi::OsString;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};

/// Keys that would silently switch a CLI from the user's subscription to API billing.
const BLOCKED_ENV: &[&str] = &[
    "ANTHROPIC_API_KEY",
    "ANTHROPIC_AUTH_TOKEN",
    "ANTHROPIC_BASE_URL",
    "CLAUDE_CODE_OAUTH_TOKEN",
    "OPENAI_API_KEY",
    "CODEX_API_KEY",
    "OPENAI_BASE_URL",
];

/// The environment a CLI runs with: the app's own, minus the API keys.
pub fn scrubbed_env() -> Vec<(OsString, OsString)> {
    std::env::vars_os().filter(|(k, _)| !BLOCKED_ENV.iter().any(|b| k == *b)).collect()
}

/// Finds a CLI by name. An app opened from Finder or the Start menu has a short PATH, so the usual install
/// folders are searched too.
pub fn locate(name: &str) -> Option<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
    let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from);
    if let Some(home) = &home {
        dirs.push(home.join(".local/bin"));
        dirs.push(home.join(".claude/local"));
    }
    if cfg!(windows) {
        if let Some(appdata) = std::env::var_os("APPDATA") {
            dirs.push(PathBuf::from(appdata).join("npm"));
        }
    } else {
        dirs.push("/opt/homebrew/bin".into());
        dirs.push("/usr/local/bin".into());
    }
    let names: Vec<String> =
        if cfg!(windows) { ["exe", "cmd"].iter().map(|ext| format!("{name}.{ext}")).collect() } else { vec![name.into()] };
    dirs.iter().flat_map(|d| names.iter().map(move |n| d.join(n))).find(|p| p.is_file())
}

/// A command for a CLI with piped stdio, the scrubbed environment, and no console window on Windows.
pub fn command(exe: &std::path::Path) -> Command {
    let mut cmd = Command::new(exe);
    cmd.env_clear().envs(scrubbed_env()).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

/// Asks the child to stop (SIGINT, like Ctrl-C) where that exists; kills it elsewhere.
pub fn interrupt(child: &mut Child) {
    #[cfg(unix)]
    {
        // SAFETY: plain kill(2) on the pid of a child we own.
        unsafe {
            libc::kill(child.id() as libc::pid_t, libc::SIGINT);
        }
    }
    #[cfg(not(unix))]
    {
        let _ = child.kill();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_keys_never_reach_a_cli() {
        // SAFETY: tests in this module do not read these variables concurrently.
        unsafe { std::env::set_var("ANTHROPIC_API_KEY", "sk-test") };
        assert!(!scrubbed_env().iter().any(|(k, _)| k == "ANTHROPIC_API_KEY"));
        assert!(scrubbed_env().iter().any(|(k, _)| k == "PATH"));
    }
}
