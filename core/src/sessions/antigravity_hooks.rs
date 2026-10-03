//! Antigravity CLI (`agy`, Gemini) hook installation: Buddy's named hook in `~/.gemini/config/hooks.json`.
//!
//! Same contract as Claude Code's and Codex's, through the same code (`hook_file`): read, dated backup, merge without
//! touching anybody else's hooks, show the diff, write only after an explicit click, refuse a file we can't parse,
//! and remove only our own entries.
//!
//! What is Antigravity's own (the CLI's bundled docs, `agy-customizations/docs/hooks.md`, agy 1.2.15):
//! * The global hooks file is `~/.gemini/config/hooks.json`, shared by the CLI and the desktop app (agy's changelog:
//!   "`/hooks` … the shared `~/.gemini/config/hooks.json`"). A workspace may add `<workspace>/.agents/hooks.json`.
//! * The file is `{ "<hook name>": { "enabled"?, "<Event>": [ … ] } }`: each top-level key is one named hook. Buddy
//!   owns one, `buddy`, and never touches the others.
//! * Tool events (`PreToolUse`, `PostToolUse`) hold `{ "matcher", "hooks": [handler…] }` groups; the others
//!   (`PreInvocation`, `PostInvocation`, `Stop`) hold the handlers directly.
//! * A handler runs through `sh -c` on Unix and `cmd /c` on Windows, in the folder of `hooks.json` (so the relay
//!   takes the session's folder from the payload's `workspacePaths`, never from its own working directory), with
//!   the agent's environment (so `BUDDY_OWN_RUN`, set on Buddy's own Gemini turns, reaches the relay). `timeout` is
//!   in seconds (default 30). Hooks run synchronously and block the agent loop.
//!
//! What Buddy listens to, and why only that: notifications, no approval cards. `PreToolUse` would be the only way
//! to answer for a tool, but it fires before *every* tool and its answer replaces agy's own permission check:
//! there is no "no opinion" (`allow`, `deny`, `ask`, `force_ask`, `deny_unless_prior_grant`), and an empty answer
//! denies the tool (seen with agy 1.2.15). So Buddy stays out of it, and agy keeps asking in its terminal as usual.
//! * `PreInvocation` (before each model call) → working.
//! * `PostToolUse` (after each tool, with its name) → working.
//! * `Stop` → done, or error when it carries one. There is no "needs you" event, so no "waiting" state.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use super::hook_file::{self, HookFile, Layout};

/// The name of Buddy's hook in the file.
pub const HOOK_NAME: &str = "buddy";

/// Every event Buddy reacts to, with the timeout written for it (the relay gives up after 2 s on its own).
pub const HOOK_EVENTS: &[(&str, u64)] = &[("PreInvocation", 5), ("PostToolUse", 5), ("Stop", 5)];

/// The events that wrap their handlers in a matcher group.
const GROUPED: &[&str] = &["PreToolUse", "PostToolUse"];

/// `~/.gemini/config`: Antigravity's global customization folder. Antigravity is around when it exists.
pub fn config_dir() -> Result<PathBuf, String> {
    Ok(hook_file::home("~/.gemini/config/hooks.json")?.join(".gemini").join("config"))
}

pub fn hooks_path() -> Result<PathBuf, String> {
    Ok(config_dir()?.join("hooks.json"))
}

/// `"<relay>" --antigravity <Event>`, with native separators (cmd strips the outer quotes on Windows).
fn hook_command(relay: &Path, event: &str) -> String {
    format!("{} --antigravity {event}", hook_file::quoted(&relay.to_string_lossy()))
}

fn handler(relay: &Path, event: &str, timeout: u64) -> Value {
    json!({ "type": "command", "command": hook_command(relay, event), "timeout": timeout })
}

/// Antigravity's hooks.json at `path` as a `HookFile` running `relay`.
pub(crate) fn spec(path: PathBuf, relay: PathBuf) -> HookFile {
    HookFile { path, relay, events: HOOK_EVENTS, handler, layout: Layout::Named { name: HOOK_NAME, grouped: GROUPED } }
}

pub(crate) fn file(relay: &Path) -> Result<HookFile, String> {
    Ok(spec(hooks_path()?, relay.to_path_buf()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sessions::hook_file::is_our_command;

    fn relay() -> PathBuf {
        PathBuf::from(if cfg!(windows) { r"C:\b\bin\buddy-hook.exe" } else { "/b/bin/buddy-hook" })
    }

    fn agy() -> HookFile {
        spec(PathBuf::from("hooks.json"), relay())
    }

    #[test]
    fn every_handler_runs_the_relay_in_antigravity_mode() {
        for (event, timeout) in HOOK_EVENTS {
            let h = handler(&relay(), event, *timeout);
            let command = h["command"].as_str().unwrap();
            assert!(is_our_command(command), "{command} must be recognised as ours");
            assert!(command.ends_with(&format!("\" --antigravity {event}")), "got {command}");
            assert_eq!(h["type"], "command");
        }
    }

    #[test]
    fn buddy_never_answers_for_a_tool() {
        // PreToolUse's answer replaces agy's own permission check (an empty one denies): Buddy must not be there.
        assert!(HOOK_EVENTS.iter().all(|(e, _)| *e != "PreToolUse"));
    }

    #[test]
    fn a_fresh_file_has_one_named_hook_in_agys_shape() {
        let after = agy().merged(&json!({}));
        let keys: Vec<&String> = after.as_object().unwrap().keys().collect();
        assert_eq!(keys, vec![HOOK_NAME]);
        let ours = &after[HOOK_NAME];
        assert_eq!(ours.as_object().unwrap().len(), HOOK_EVENTS.len());
        // Flat events hold the handler itself; tool events a matcher group.
        assert!(ours["Stop"][0]["command"].as_str().unwrap().ends_with("--antigravity Stop"));
        assert!(ours["PreInvocation"][0]["command"].as_str().unwrap().ends_with("--antigravity PreInvocation"));
        assert_eq!(ours["PostToolUse"][0]["matcher"], "*");
        assert!(ours["PostToolUse"][0]["hooks"][0]["command"].as_str().unwrap().ends_with("--antigravity PostToolUse"));
    }

    #[test]
    fn install_is_idempotent_and_uninstall_restores_the_users_hooks() {
        let existing = json!({
            "lint-checker": { "PostToolUse": [{ "matcher": "run_command", "hooks": [{ "command": "./lint.sh", "timeout": 10 }] }] },
            "reminder": { "enabled": false, "PreInvocation": [{ "type": "command", "command": "./remind.sh" }] }
        });
        let once = agy().merged(&existing);
        assert_eq!(agy().merged(&once), once, "installing twice changes nothing");
        assert_eq!(once["lint-checker"], existing["lint-checker"]);
        assert_eq!(once["reminder"], existing["reminder"]);
        assert_eq!(agy().cleaned(&once), existing, "uninstall leaves exactly what the user had");
        assert_eq!(agy().cleaned(&existing), existing, "nothing of ours, nothing removed");
    }

    #[test]
    fn our_handler_is_found_and_removed_under_any_name_but_foreign_ones_stay() {
        let moved = json!({
            "mine": {
                "Stop": [
                    { "type": "command", "command": "\"/old/bin/buddy-hook\" --antigravity Stop" },
                    { "type": "command", "command": "say done" }
                ],
                "PostToolUse": [{ "matcher": "*", "hooks": [{ "command": "\"/old/bin/buddy-hook\" --antigravity PostToolUse" }] }]
            },
            "wrapper": { "Stop": [{ "command": "node buddy-hook-wrapper.js" }] }
        });
        let cleaned = agy().cleaned(&moved);
        assert_eq!(cleaned["mine"], json!({ "Stop": [{ "type": "command", "command": "say done" }] }));
        assert_eq!(cleaned["wrapper"], moved["wrapper"], "a hook that only mentions our name is not ours");
        // Installing replaces the stray copy instead of running the relay twice.
        let installed = agy().merged(&moved);
        let count = installed.to_string().matches("--antigravity Stop").count();
        assert_eq!(count, 1);
    }

    /// A real file in a temp folder, reached through `spec` rather than HOME.
    #[test]
    fn install_and_uninstall_back_up_and_refuse_a_changed_file() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("config").join("hooks.json");
        let agy = spec(path.clone(), relay());

        let plan = agy.preview(true).unwrap();
        assert!(plan.diff.contains("--antigravity"), "the diff must show the relay");
        assert_eq!(agy.write(true, &plan.fingerprint).unwrap(), "", "no file yet: nothing to back up");
        assert!(agy.installed());
        let first = std::fs::read(&path).unwrap();

        // Installing again: same file, and a backup of the previous one.
        let plan = agy.preview(true).unwrap();
        assert_eq!(plan.diff, "Sin cambios.");
        let backup = agy.write(true, &plan.fingerprint).unwrap();
        assert!(backup.contains("hooks.json.bak-"), "got {backup}");
        assert_eq!(std::fs::read(&path).unwrap(), first);

        // The user adds a hook of their own.
        let mut now: Value = serde_json::from_slice(&first).unwrap();
        now["safety-gate"] = json!({ "PreToolUse": [{ "matcher": "run_command", "hooks": [{ "command": "./check.sh" }] }] });
        let written = serde_json::to_vec_pretty(&now).unwrap();
        std::fs::write(&path, &written).unwrap();

        // A file that moved since the preview is refused, and left alone.
        let stale = agy.preview(false).unwrap();
        std::fs::write(&path, b"{}").unwrap();
        assert!(agy.write(false, &stale.fingerprint).unwrap_err().contains("cambió desde la vista previa"));
        std::fs::write(&path, &written).unwrap();

        // Uninstall keeps theirs, drops ours, backs up what was there.
        let plan = agy.preview(false).unwrap();
        let backup = agy.write(false, &plan.fingerprint).unwrap();
        assert_eq!(std::fs::read(&backup).unwrap(), written);
        let after: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(after, json!({ "safety-gate": now["safety-gate"].clone() }));
        assert!(!agy.installed());
        // Uninstalling again changes nothing.
        assert_eq!(agy.preview(false).unwrap().diff, "Sin cambios.");

        std::fs::write(&path, b"{ broken").unwrap();
        assert!(agy.preview(true).is_err());
        assert!(agy.write(true, "whatever").is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"{ broken");
    }
}
