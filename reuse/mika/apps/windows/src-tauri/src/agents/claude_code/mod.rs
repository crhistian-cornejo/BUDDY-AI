//! Claude Code as an agent source.
//!
//! Claude Code runs `mika-hook.exe` on every hook event (see `hooks`, which
//! installs those entries in `~/.claude/settings.json` only after the user
//! confirmed a diff); the relay forwards each event over a per-user named pipe
//! (see `pipe`), and `pipe` turns it into a `hook` event for the island.

pub(crate) mod hooks;
pub(crate) mod pipe;

use tauri::AppHandle;

/// Stages the relay executable and starts listening on the pipe.
pub fn start(app: &AppHandle) {
    hooks::ensure_hook_exe(app);
    pipe::start(app.clone());
}
