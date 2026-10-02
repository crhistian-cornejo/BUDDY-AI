//! Codex as an agent source.
//!
//! Codex runs the same relay as Claude Code — `mika-hook.exe --codex <Event>`,
//! installed in `~/.codex/hooks.json` by `hooks` only after the user confirmed a
//! diff — and the relay talks to the same named pipe (`claude_code::pipe`),
//! tagging every event `_agent: "codex"`. The island routes on that tag, so the
//! Codex pill gets the same ticker, sounds and approval card as Claude Code's.
//! The relay and the pipe are started once, by `claude_code::start`.

pub(crate) mod hooks;

use tauri::AppHandle;

use crate::services::log;

/// Nothing to start of its own (see above); says in the log whether Codex
/// will be sending anything, which is the first question when it doesn't.
pub fn start(_app: &AppHandle) {
    let status = hooks::status();
    log::line(format!(
        "codex hooks {} ({})",
        if status.installed { "installed" } else { "not installed" },
        status.settings_path
    ));
}
