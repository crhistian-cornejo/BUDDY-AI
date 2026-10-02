// Codex hook installation: MIKA's entries in `~/.codex/hooks.json`.
//
// Same contract as Claude Code's, through the same code (`agents::hook_file`):
// read, dated backup, merge without touching anybody else's hooks, show the
// diff, write only after an explicit click, refuse a file we can't parse, and
// remove only our own entries.
//
// What is Codex's own (https://developers.openai.com/codex/hooks, checked
// against openai/codex `codex-rs/hooks` and `codex-rs/config/src/hook_config.rs`):
// * User hooks live in `$CODEX_HOME/hooks.json` (default `~/.codex`), or inline
//   in config.toml. MIKA writes the JSON file, so it never parses or rewrites
//   the user's TOML. Both forms load; Codex only warns when one folder has both.
// * The file is `{ "description"?, "hooks": { "<Event>": [ group… ] } }` and
//   Codex refuses any other top-level key — so MIKA only ever touches "hooks".
// * Codex runs a command hook through `cmd.exe /C` on Windows (`$SHELL -lc` on
//   macOS), so the command is the quoted exe path, then `--codex <Event>`.
// * Codex skips a new or changed non-managed hook until the user trusts that
//   exact definition in Codex's `/hooks` (it stores a hash in config.toml).
//   MIKA never writes that hash: trusting a hook is the user's call, made in
//   Codex. The settings window says so.
// * `timeout` is in seconds; SessionEnd and Interrupt allow at most 3.
// * PermissionRequest takes the same decision JSON as Claude Code's (mika-hook
//   prints it); no output means Codex shows its own approval prompt.

use std::ffi::OsString;
use std::path::PathBuf;

use serde_json::{json, Value};

use crate::agents::hook_file::{self, HookFile};
pub use crate::agents::hook_file::{HookPreview, HookStatus};
use crate::services::settings;

/// Every Codex event the island reacts to, with the timeout written for it.
/// PermissionRequest waits for a human: the relay's 110 s plus a margin.
/// There is no Notification, StopFailure or PostToolUseFailure in Codex.
pub const HOOK_EVENTS: &[(&str, u64)] = &[
    ("SessionStart", 10),
    ("SessionEnd", 3),
    ("UserPromptSubmit", 10),
    ("PreToolUse", 10),
    ("PostToolUse", 10),
    ("PermissionRequest", 120),
    ("Stop", 10),
    ("SubagentStart", 10),
    ("SubagentStop", 10),
    ("Interrupt", 3),
];

/// Shown by Codex while the PermissionRequest hook runs, i.e. while the card
/// is up in the island.
const WAITING_MESSAGE: &str = "Waiting for your answer in MIKA";

/// `$CODEX_HOME` when set (as Codex itself resolves it), else `~/.codex`.
fn codex_home_from(
    codex_home: Option<OsString>,
    home: impl FnOnce() -> Result<PathBuf, String>,
) -> Result<PathBuf, String> {
    match codex_home.filter(|v| !v.is_empty()) {
        Some(dir) => Ok(PathBuf::from(dir)),
        None => Ok(home()?.join(".codex")),
    }
}

pub fn hooks_path() -> Result<PathBuf, String> {
    let dir = codex_home_from(std::env::var_os("CODEX_HOME"), || {
        hook_file::home("~/.codex/hooks.json")
    })?;
    Ok(dir.join("hooks.json"))
}

/// `"C:\…\mika-hook.exe" --codex <Event>`. Native backslashes, because cmd.exe
/// runs it: Codex hands cmd `/C ""<exe>" --codex <Event>"`, and cmd strips the
/// outer pair of quotes.
fn hook_command(event: &str) -> String {
    format!("\"{}\" --codex {event}", settings::hook_exe_path().display())
}

fn handler(event: &str, timeout: u64) -> Value {
    let mut entry = json!({
        "type": "command",
        "command": hook_command(event),
        "timeout": timeout,
    });
    if event == "PermissionRequest" {
        entry["statusMessage"] = json!(WAITING_MESSAGE);
    }
    entry
}

/// Codex's hooks.json as a `HookFile` at `path`.
fn spec(path: PathBuf) -> HookFile {
    HookFile { path, events: HOOK_EVENTS, handler }
}

fn file() -> Result<HookFile, String> {
    Ok(spec(hooks_path()?))
}

// ── Public API ────────────────────────────────────────────────────────────────

pub fn status() -> HookStatus {
    file().map(|f| f.status()).unwrap_or_else(|_| hook_file::status_without_file())
}

pub fn preview(install: bool) -> Result<HookPreview, String> {
    file()?.preview(install)
}

/// Writes the merged (or cleaned) hooks.json after taking a dated backup;
/// refuses when the file no longer matches the preview's `fingerprint`.
pub fn write(install: bool, fingerprint: &str) -> Result<String, String> {
    file()?.write(install, fingerprint)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::hook_file::{entry_is_ours, is_our_command, without_ours};

    #[test]
    fn codex_home_wins_over_the_profile_folder() {
        let home = || Ok(PathBuf::from(r"C:\Users\a"));
        assert_eq!(
            codex_home_from(Some(OsString::from(r"D:\codex")), home).unwrap(),
            PathBuf::from(r"D:\codex")
        );
        assert_eq!(codex_home_from(None, home).unwrap(), PathBuf::from(r"C:\Users\a\.codex"));
        assert_eq!(
            codex_home_from(Some(OsString::new()), home).unwrap(),
            PathBuf::from(r"C:\Users\a\.codex"),
            "an empty CODEX_HOME is no CODEX_HOME"
        );
        assert!(codex_home_from(None, || Err("no home".into())).is_err());
    }

    #[test]
    fn every_handler_runs_the_relay_in_codex_mode() {
        for (event, timeout) in HOOK_EVENTS {
            let h = handler(event, *timeout);
            let command = h["command"].as_str().unwrap();
            assert!(is_our_command(command), "{command} must be recognised as ours");
            assert!(command.starts_with('"'), "the exe path is quoted for cmd.exe: {command}");
            assert!(command.ends_with(&format!("\" --codex {event}")), "got {command}");
            assert_eq!(h["type"], "command");
            assert_eq!(h["timeout"], *timeout);
        }
    }

    #[test]
    fn timeouts_respect_codex_limits() {
        for (event, timeout) in HOOK_EVENTS {
            if *event == "SessionEnd" || *event == "Interrupt" {
                assert!(*timeout <= 3, "Codex caps {event} at 3 s");
            }
        }
        let permission = HOOK_EVENTS.iter().find(|(e, _)| *e == "PermissionRequest").unwrap();
        assert!(permission.1 > 110, "Codex must outwait the relay's 110 s decision budget");
    }

    #[test]
    fn only_the_permission_hook_has_a_status_message() {
        assert_eq!(handler("PermissionRequest", 120)["statusMessage"], WAITING_MESSAGE);
        assert!(handler("PreToolUse", 10).get("statusMessage").is_none());
    }

    #[test]
    fn a_fresh_file_has_nothing_but_hooks() {
        // Codex's HooksFile denies unknown top-level fields: one stray key and
        // the whole file — the user's hooks included — would be ignored.
        let after = spec(PathBuf::from("hooks.json")).merged(&json!({}));
        let keys: Vec<&String> = after.as_object().unwrap().keys().collect();
        assert_eq!(keys, vec!["hooks"]);
        let events = after["hooks"].as_object().unwrap();
        assert_eq!(events.len(), HOOK_EVENTS.len());
        for (event, _) in HOOK_EVENTS {
            let groups = events[*event].as_array().unwrap();
            assert_eq!(groups.len(), 1);
            assert!(groups[0].get("matcher").is_none(), "no matcher: every {event} counts");
            assert!(entry_is_ours(&groups[0]));
        }
    }

    #[test]
    fn the_users_own_hooks_and_description_survive() {
        let existing = json!({
            "description": "my hooks",
            "hooks": {
                "PreToolUse": [
                    { "matcher": "^Bash$", "hooks": [{ "type": "command", "command": "python3 scan.py", "timeout": 30 }] }
                ],
                "PreCompact": [{ "hooks": [{ "type": "command", "command": "keep-me.exe" }] }]
            }
        });
        let after = spec(PathBuf::from("hooks.json")).merged(&existing);
        assert_eq!(after["description"], "my hooks");
        assert_eq!(after["hooks"]["PreToolUse"][0], existing["hooks"]["PreToolUse"][0]);
        assert!(entry_is_ours(&after["hooks"]["PreToolUse"][1]));
        assert_eq!(after["hooks"]["PreCompact"], existing["hooks"]["PreCompact"]);
        assert_eq!(without_ours(&after), existing);
    }

    /// A real file in a temp folder, reached through `spec` rather than
    /// CODEX_HOME / USERPROFILE: environment variables are process-wide and
    /// another test moves USERPROFILE.
    #[test]
    fn install_and_uninstall_back_up_and_refuse_a_changed_file() {
        let tmp = std::env::temp_dir().join(format!("mika-codex-hooks-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let path = tmp.join("hooks.json");
        let codex = spec(path.clone());

        // No file yet: the preview starts from nothing and the write creates it.
        let plan = codex.preview(true).unwrap();
        assert!(plan.diff.contains("--codex"), "the diff must show the relay");
        let backup = codex.write(true, &plan.fingerprint).unwrap();
        assert!(!std::path::Path::new(&backup).exists(), "nothing to back up the first time");
        assert!(codex.status().installed);

        // The user adds a hook of their own next to ours.
        let mut now: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        now["hooks"]["Stop"]
            .as_array_mut()
            .unwrap()
            .insert(0, json!({ "hooks": [{ "type": "command", "command": "notify.exe" }] }));
        let written = serde_json::to_vec_pretty(&now).unwrap();
        std::fs::write(&path, &written).unwrap();

        // A file that moved since the preview is refused, and left alone.
        let stale = codex.preview(false).unwrap();
        std::fs::write(&path, br#"{"hooks":{}}"#).unwrap();
        assert!(codex.write(false, &stale.fingerprint).unwrap_err().contains("changed since the preview"));
        std::fs::write(&path, &written).unwrap();

        // Uninstall keeps their hook, drops ours, and backs up what was there.
        let plan = codex.preview(false).unwrap();
        let backup = codex.write(false, &plan.fingerprint).unwrap();
        assert!(backup.contains("hooks.json.bak-"), "got {backup}");
        assert_eq!(std::fs::read(&backup).unwrap(), written);
        let after: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(after, json!({ "hooks": { "Stop": [{ "hooks": [{ "type": "command", "command": "notify.exe" }] }] } }));
        assert!(!codex.status().installed);

        // Content we cannot parse is refused before anything is written.
        std::fs::write(&path, b"{ broken").unwrap();
        assert!(codex.preview(true).is_err());
        assert!(codex.write(true, "whatever").is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"{ broken");

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
