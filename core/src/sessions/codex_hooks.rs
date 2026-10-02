// Ported from MIKA (MIT, revision d050bc5): apps/windows/src-tauri/src/agents/codex/hooks.rs
//! Codex hook installation: Buddy's entries in `~/.codex/hooks.json`.
//!
//! Same contract as Claude Code's, through the same code (`hook_file`): read, dated backup, merge without touching
//! anybody else's hooks, show the diff, write only after an explicit click, refuse a file we can't parse, and
//! remove only our own entries.
//!
//! What is Codex's own (https://developers.openai.com/codex/hooks):
//! * User hooks live in `$CODEX_HOME/hooks.json` (default `~/.codex`), or inline in config.toml. Buddy writes the
//!   JSON file, so it never parses or rewrites the user's TOML.
//! * The file is `{ "description"?, "hooks": { "<Event>": [ group… ] } }` and Codex refuses any other top-level
//!   key — so Buddy only ever touches "hooks".
//! * Codex runs a command hook through `$SHELL -lc` on Mac and `cmd.exe /C` on Windows, so the command is the quoted
//!   relay path (native separators), then `--codex <Event>`.
//! * Codex skips a new or changed non-managed hook until the user trusts that exact definition in Codex's `/hooks`.
//!   Buddy never does that for them: trusting a hook is the user's call, made in Codex.
//! * `timeout` is in seconds; SessionEnd and Interrupt allow at most 3.
//! * PermissionRequest takes the same decision JSON as Claude Code's (buddy-hook prints it); no output means Codex
//!   shows its own approval prompt.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use super::hook_file::{self, HookFile};

/// Every Codex event Buddy reacts to, with the timeout written for it. PermissionRequest waits for a human: the
/// relay's 110 s plus a margin. There is no Notification, StopFailure or PostToolUseFailure in Codex.
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

/// Shown by Codex while the PermissionRequest hook runs, i.e. while the card is up in Buddy's notch.
const WAITING_MESSAGE: &str = "Esperando tu respuesta en Buddy";

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

/// Codex's home folder: Codex is around when it exists.
pub fn config_dir() -> Result<PathBuf, String> {
    codex_home_from(std::env::var_os("CODEX_HOME"), || hook_file::home("~/.codex/hooks.json"))
}

pub fn hooks_path() -> Result<PathBuf, String> {
    Ok(config_dir()?.join("hooks.json"))
}

/// `"<relay>" --codex <Event>`, with native separators. On Windows cmd.exe runs it: Codex hands cmd
/// `/C ""<exe>" --codex <Event>"`, and cmd strips the outer pair of quotes.
fn hook_command(relay: &Path, event: &str) -> String {
    format!("{} --codex {event}", hook_file::quoted(&relay.to_string_lossy()))
}

fn handler(relay: &Path, event: &str, timeout: u64) -> Value {
    let mut entry = json!({
        "type": "command",
        "command": hook_command(relay, event),
        "timeout": timeout,
    });
    if event == "PermissionRequest" {
        entry["statusMessage"] = json!(WAITING_MESSAGE);
    }
    entry
}

/// Codex's hooks.json at `path` as a `HookFile` running `relay`.
pub(crate) fn spec(path: PathBuf, relay: PathBuf) -> HookFile {
    HookFile { path, relay, events: HOOK_EVENTS, handler }
}

pub(crate) fn file(relay: &Path) -> Result<HookFile, String> {
    Ok(spec(hooks_path()?, relay.to_path_buf()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sessions::hook_file::{entry_is_ours, is_our_command, without_ours};

    fn relay() -> PathBuf {
        PathBuf::from(if cfg!(windows) { r"C:\b\bin\buddy-hook.exe" } else { "/b/bin/buddy-hook" })
    }

    #[test]
    fn codex_home_wins_over_the_profile_folder() {
        let home = || Ok(PathBuf::from("/Users/a"));
        assert_eq!(codex_home_from(Some(OsString::from("/opt/codex")), home).unwrap(), PathBuf::from("/opt/codex"));
        assert_eq!(codex_home_from(None, home).unwrap(), PathBuf::from("/Users/a").join(".codex"));
        assert_eq!(
            codex_home_from(Some(OsString::new()), home).unwrap(),
            PathBuf::from("/Users/a").join(".codex"),
            "an empty CODEX_HOME is no CODEX_HOME"
        );
        assert!(codex_home_from(None, || Err("no home".into())).is_err());
    }

    #[test]
    fn every_handler_runs_the_relay_in_codex_mode() {
        for (event, timeout) in HOOK_EVENTS {
            let h = handler(&relay(), event, *timeout);
            let command = h["command"].as_str().unwrap();
            assert!(is_our_command(command), "{command} must be recognised as ours");
            assert!(command.starts_with('"'), "the relay path is quoted for the shell: {command}");
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
        assert_eq!(handler(&relay(), "PermissionRequest", 120)["statusMessage"], WAITING_MESSAGE);
        assert!(handler(&relay(), "PreToolUse", 10).get("statusMessage").is_none());
    }

    #[test]
    fn a_fresh_file_has_nothing_but_hooks() {
        // Codex's HooksFile denies unknown top-level fields: one stray key and the whole file — the user's hooks
        // included — would be ignored.
        let after = spec(PathBuf::from("hooks.json"), relay()).merged(&json!({}));
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
                "PreCompact": [{ "hooks": [{ "type": "command", "command": "keep-me" }] }]
            }
        });
        let after = spec(PathBuf::from("hooks.json"), relay()).merged(&existing);
        assert_eq!(after["description"], "my hooks");
        assert_eq!(after["hooks"]["PreToolUse"][0], existing["hooks"]["PreToolUse"][0]);
        assert!(entry_is_ours(&after["hooks"]["PreToolUse"][1]));
        assert_eq!(after["hooks"]["PreCompact"], existing["hooks"]["PreCompact"]);
        assert_eq!(without_ours(&after), existing);
    }

    /// A real file in a temp folder, reached through `spec` rather than CODEX_HOME / HOME: environment variables
    /// are process-wide and another test moves the home folder.
    #[test]
    fn install_and_uninstall_back_up_and_refuse_a_changed_file() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("hooks.json");
        let codex = spec(path.clone(), relay());

        // No file yet: the preview starts from nothing and the write creates it, with nothing to back up.
        let plan = codex.preview(true).unwrap();
        assert!(plan.diff.contains("--codex"), "the diff must show the relay");
        assert_eq!(codex.write(true, &plan.fingerprint).unwrap(), "");
        assert!(codex.installed());

        // The user adds a hook of their own next to ours.
        let mut now: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        now["hooks"]["Stop"]
            .as_array_mut()
            .unwrap()
            .insert(0, json!({ "hooks": [{ "type": "command", "command": "notify" }] }));
        let written = serde_json::to_vec_pretty(&now).unwrap();
        std::fs::write(&path, &written).unwrap();

        // A file that moved since the preview is refused, and left alone.
        let stale = codex.preview(false).unwrap();
        std::fs::write(&path, br#"{"hooks":{}}"#).unwrap();
        assert!(codex.write(false, &stale.fingerprint).unwrap_err().contains("cambió desde la vista previa"));
        std::fs::write(&path, &written).unwrap();

        // Uninstall keeps their hook, drops ours, and backs up what was there.
        let plan = codex.preview(false).unwrap();
        let backup = codex.write(false, &plan.fingerprint).unwrap();
        assert!(backup.contains("hooks.json.bak-"), "got {backup}");
        assert_eq!(std::fs::read(&backup).unwrap(), written);
        let after: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(after, json!({ "hooks": { "Stop": [{ "hooks": [{ "type": "command", "command": "notify" }] }] } }));
        assert!(!codex.installed());

        // Content we cannot parse is refused before anything is written.
        std::fs::write(&path, b"{ broken").unwrap();
        assert!(codex.preview(true).is_err());
        assert!(codex.write(true, "whatever").is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"{ broken");
    }
}
