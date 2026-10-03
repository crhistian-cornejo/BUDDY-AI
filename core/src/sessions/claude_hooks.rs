// Ported from MIKA (MIT, revision d050bc5): apps/windows/src-tauri/src/agents/claude_code/hooks.rs
//! Claude Code hook installation: Buddy's entries in `~/.claude/settings.json`.
//!
//! Read, dated backup, merge without touching anybody else's hooks, show the diff, write only after an explicit
//! click; uninstall removes Buddy's entries and nothing else. The machinery lives in `hook_file`, shared with Codex;
//! this file says what is Claude Code's own: where settings.json is, which events, and the command line.
//!
//! The command is only the quoted relay path plus the event name. On Windows Claude Code runs hook commands through
//! Git Bash, so the path is written with forward slashes (anything with PowerShell or cmd in it breaks).

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use super::hook_file::{self, HookFile};

/// Every event Buddy reacts to, with the hook timeout written to settings.json. PermissionRequest waits for a human,
/// so it gets the relay's decision budget (110 s) plus a margin.
pub const HOOK_EVENTS: &[(&str, u64)] = &[
    ("SessionStart", 10),
    ("SessionEnd", 10),
    ("UserPromptSubmit", 10),
    ("PreToolUse", 10),
    ("PostToolUse", 10),
    ("PostToolUseFailure", 10),
    ("PermissionRequest", 120),
    ("Notification", 10),
    ("Stop", 10),
    ("StopFailure", 10),
    ("SubagentStart", 10),
    ("SubagentStop", 10),
];

/// `~/.claude`: Claude Code is around when this folder exists.
pub fn config_dir() -> Result<PathBuf, String> {
    Ok(hook_file::home("~/.claude/settings.json")?.join(".claude"))
}

pub fn settings_path() -> Result<PathBuf, String> {
    Ok(config_dir()?.join("settings.json"))
}

fn hook_command(relay: &Path, event: &str) -> String {
    let relay = relay.to_string_lossy();
    let relay = if cfg!(windows) { relay.replace('\\', "/") } else { relay.into_owned() };
    format!("{} {event}", hook_file::quoted(&relay))
}

fn handler(relay: &Path, event: &str, timeout: u64) -> Value {
    json!({
        "type": "command",
        "command": hook_command(relay, event),
        "timeout": timeout,
    })
}

/// Claude Code's settings.json at `path` as a `HookFile` running `relay`.
pub(crate) fn spec(path: PathBuf, relay: PathBuf) -> HookFile {
    HookFile { path, relay, events: HOOK_EVENTS, handler, layout: hook_file::Layout::Hooks }
}

pub(crate) fn file(relay: &Path) -> Result<HookFile, String> {
    Ok(spec(settings_path()?, relay.to_path_buf()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sessions::hook_file::{ENV_LOCK, entry_is_ours, is_our_command, without_ours};

    fn relay() -> PathBuf {
        if cfg!(windows) {
            PathBuf::from(r"C:\Users\a\AppData\Local\Buddy\bin\buddy-hook.exe")
        } else {
            PathBuf::from("/Users/a/Library/Application Support/Buddy/bin/buddy-hook")
        }
    }

    #[test]
    fn merging_keeps_every_other_setting_and_every_foreign_hook() {
        let existing = json!({
            "model": "claude-opus-5",
            "theme": "dark",
            "enabledPlugins": ["a", "b"],
            "hooks": {
                "PreToolUse": [
                    { "hooks": [{ "type": "command", "command": "someone-elses-tool" }] }
                ],
                "SomeEventWeDoNotTouch": [
                    { "hooks": [{ "type": "command", "command": "keep-me" }] }
                ]
            }
        });

        let after = spec(PathBuf::from("settings.json"), relay()).merged(&existing);
        assert_eq!(after["model"], "claude-opus-5");
        assert_eq!(after["theme"], "dark");
        assert_eq!(after["enabledPlugins"], json!(["a", "b"]));

        let pre = after["hooks"]["PreToolUse"].as_array().unwrap();
        assert!(
            pre.iter().any(|e| serde_json::to_string(e).unwrap().contains("someone-elses-tool")),
            "another tool's hook was dropped"
        );
        assert!(pre.iter().any(entry_is_ours), "our own hook was not added");
        assert!(after["hooks"]["SomeEventWeDoNotTouch"].is_array());

        // And removing ours puts it back exactly as it was.
        assert_eq!(without_ours(&after), existing);
    }

    #[test]
    fn the_command_is_the_quoted_relay_and_the_event() {
        let command = hook_command(&relay(), "Stop");
        assert!(command.starts_with('"') && command.ends_with("\" Stop"), "got {command}");
        assert!(is_our_command(&command), "got {command}");
        assert!(!command.contains("--codex"));
        if cfg!(windows) {
            // Claude Code runs hooks through Git Bash on Windows: no backslashes.
            assert!(!command.contains('\\'), "got {command}");
        } else {
            assert_eq!(command, "\"/Users/a/Library/Application Support/Buddy/bin/buddy-hook\" Stop");
        }
    }

    /// Everything filesystem-shaped lives in one test on purpose: it points the home folder at a temp directory,
    /// and that is process-wide (hence `ENV_LOCK`, and the old value is put back).
    #[test]
    fn writing_backs_up_preserves_and_refuses_a_changed_file() {
        let _env = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join(".claude")).unwrap();
        let var = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
        let old = std::env::var_os(var);
        // SAFETY: every test that touches the home variables holds ENV_LOCK.
        unsafe { std::env::set_var(var, tmp.path()) };

        let result = std::panic::catch_unwind(|| {
            let path = settings_path().unwrap();
            assert!(path.starts_with(tmp.path()), "the test must not touch the real home");
            let claude = file(&relay()).unwrap();

            // A real-shaped file, written the way PowerShell 5 would: UTF-8 with BOM.
            let original = r#"{"model":"claude-opus-5","theme":"dark","tui":{"x":1},"hooks":{"PreToolUse":[{"hooks":[{"type":"command","command":"other-tool"}]}]}}"#;
            let mut bytes = vec![0xEF, 0xBB, 0xBF];
            bytes.extend_from_slice(original.as_bytes());
            std::fs::write(&path, &bytes).unwrap();

            // Install.
            let plan = claude.preview(true).expect("a BOM must not stop the preview");
            assert!(plan.diff.contains("buddy-hook"), "the diff must show what changes");
            assert_eq!(plan.path, path.to_string_lossy());
            let backup = claude.write(true, &plan.fingerprint).expect("install should succeed");
            assert!(backup.contains("settings.json.bak-"), "got {backup}");

            // The backup holds the original bytes, BOM and all.
            assert_eq!(std::fs::read(&backup).unwrap(), bytes);

            // Everything else survived, and so did the other tool's hook.
            let after: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
            assert_eq!(after["model"], "claude-opus-5");
            assert_eq!(after["theme"], "dark");
            assert_eq!(after["tui"]["x"], 1);
            let pre = after["hooks"]["PreToolUse"].as_array().unwrap();
            assert!(pre.iter().any(|e| serde_json::to_string(e).unwrap().contains("other-tool")));
            assert!(claude.installed());

            // A file that moved since the preview is refused, and left alone.
            let stale = claude.preview(false).unwrap();
            std::fs::write(&path, br#"{"model":"someone-else-edited-this"}"#).unwrap();
            let err = claude.write(false, &stale.fingerprint).unwrap_err();
            assert!(err.contains("cambió desde la vista previa"), "got: {err}");
            let untouched: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
            assert_eq!(untouched["model"], "someone-else-edited-this");
            assert!(!claude.installed());

            // Content we cannot parse is refused before anything is written.
            std::fs::write(&path, b"{ broken").unwrap();
            assert!(claude.preview(true).is_err());
            assert!(claude.write(true, "whatever").is_err());
            assert_eq!(std::fs::read(&path).unwrap(), b"{ broken");
        });

        // SAFETY: still under ENV_LOCK.
        unsafe {
            match old {
                Some(v) => std::env::set_var(var, v),
                None => std::env::remove_var(var),
            }
        }
        if let Err(panic) = result {
            std::panic::resume_unwind(panic);
        }
    }
}
