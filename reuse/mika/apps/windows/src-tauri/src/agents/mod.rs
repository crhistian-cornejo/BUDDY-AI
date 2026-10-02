//! Agent sources: coding agents whose sessions Mika shows on the island.

pub(crate) mod claude_code;
pub(crate) mod codex;
pub(crate) mod hook_file;

use tauri::AppHandle;

/// Starts every agent source. A new source is one module plus one line here.
/// Claude Code's goes first: it stages the relay and opens the pipe that every
/// hook-based source shares.
pub fn start_all(app: &AppHandle) {
    claude_code::start(app);
    codex::start(app);
}
