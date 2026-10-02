// The JSON hooks file a coding agent reads — Claude Code's
// `~/.claude/settings.json`, Codex's `~/.codex/hooks.json` — and the one way MIKA
// ever changes it.
//
// The rule from CLAUDE.md is strict and is followed to the letter: read the file,
// take a dated backup, merge without touching anybody else's hooks, show the
// diff, and write only after an explicit click. Uninstall removes MIKA's entries
// and nothing else.
//
// Both agents use the same shape —
//   { "hooks": { "<Event>": [ { "matcher"?, "hooks": [ { "type": "command", … } ] } ] } }
// — so one implementation serves both. Each agent source only says where its
// file is, which events it wants and what one handler looks like (`HookFile`).
// MIKA's handlers are recognised by the program they run (`mika-hook.exe`).

use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::{json, Map, Value};
use windows::Win32::System::SystemInformation::GetLocalTime;

use crate::services::settings;

/// File name of the relay; identifies MIKA's hooks inside any agent's file.
const MARKER: &str = "mika-hook.exe";

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HookStatus {
    pub installed: bool,
    /// The agent's hooks file (named after Claude Code's, the first one).
    pub settings_path: String,
    pub hook_path: String,
    pub hook_ready: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HookPreview {
    pub diff: String,
    pub backup: String,
    pub settings_path: String,
    /// Identifies the bytes this diff was computed from; handed back to `write`
    /// so we only ever apply what the user actually looked at.
    pub fingerprint: String,
}

/// One agent's hooks file and what MIKA puts in it.
pub(crate) struct HookFile {
    pub path: PathBuf,
    /// Every event MIKA listens to, with the timeout (seconds) written for it.
    pub events: &'static [(&'static str, u64)],
    /// MIKA's handler object for one event: `{"type":"command","command":…,…}`.
    pub handler: fn(event: &str, timeout: u64) -> Value,
}

/// The user's profile folder. There is deliberately no fallback: guessing "."
/// would put an agent's config wherever the process happens to run. `what`
/// names the file we were looking for, for the error.
pub(crate) fn home(what: &str) -> Result<PathBuf, String> {
    std::env::var_os("USERPROFILE")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| format!("USERPROFILE isn't set, so MIKA can't tell where {what} is."))
}

/// The status to report when the file's location can't even be worked out.
pub(crate) fn status_without_file() -> HookStatus {
    let hook_path = settings::hook_exe_path();
    HookStatus {
        installed: false,
        settings_path: String::new(),
        hook_ready: hook_path.exists(),
        hook_path: hook_path.to_string_lossy().to_string(),
    }
}

impl HookFile {
    /// Reads the file.
    ///
    /// The only error that means "start from nothing" is the file not being
    /// there. Everything else — a lock held by another process, a permission
    /// problem, JSON we cannot parse — is reported, because the alternative is
    /// treating somebody's unreadable config as an empty object and then writing
    /// that back over it.
    fn read(&self) -> Result<Value, String> {
        match std::fs::read(&self.path) {
            Ok(bytes) => parse_settings(&bytes, &self.path.display().to_string()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(json!({})),
            // A lock, a permission problem, a bad drive: all of them mean we do not
            // know what is in there, and not knowing is not the same as empty.
            Err(err) => Err(format!("Can't read {}: {err}", self.path.display())),
        }
    }

    /// The file as it is, or an empty object when we cannot tell. Only for
    /// read-only paths like `status()`, which must never fail loudly; anything
    /// that writes uses `read()` and surfaces the error instead.
    fn read_lossy(&self) -> Value {
        self.read().unwrap_or_else(|_| json!({}))
    }

    /// The file with MIKA's hooks added; everything else is left untouched.
    /// Our entries go last in each event's list, so the position of every other
    /// tool's entry stays what it was.
    pub(crate) fn merged(&self, existing: &Value) -> Value {
        let mut root = existing.as_object().cloned().unwrap_or_default();
        let mut hooks = root
            .get("hooks")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_else(Map::new);

        for (event, timeout) in self.events {
            let mut list = hooks
                .get(*event)
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            list = list.iter().filter_map(without_our_hooks).collect();
            list.push(json!({ "hooks": [(self.handler)(event, *timeout)] }));
            hooks.insert((*event).to_string(), Value::Array(list));
        }

        root.insert("hooks".into(), Value::Object(hooks));
        Value::Object(root)
    }

    pub fn status(&self) -> HookStatus {
        let current = self.read_lossy();
        let installed = current
            .get("hooks")
            .and_then(Value::as_object)
            .map(|hooks| {
                hooks
                    .values()
                    .filter_map(Value::as_array)
                    .flatten()
                    .any(entry_is_ours)
            })
            .unwrap_or(false);
        let hook_path = settings::hook_exe_path();
        HookStatus {
            installed,
            settings_path: self.path.to_string_lossy().to_string(),
            hook_ready: hook_path.exists(),
            hook_path: hook_path.to_string_lossy().to_string(),
        }
    }

    pub fn preview(&self, install: bool) -> Result<HookPreview, String> {
        let current = self.read()?;
        let next = if install { self.merged(&current) } else { without_ours(&current) };
        Ok(HookPreview {
            diff: unified_diff(&pretty(&current), &pretty(&next)),
            backup: backup_path(&self.path).to_string_lossy().to_string(),
            settings_path: self.path.to_string_lossy().to_string(),
            fingerprint: current_fingerprint(&self.path),
        })
    }

    /// Writes the merged (or cleaned) file after taking a dated backup.
    ///
    /// `fingerprint` is the one the preview was computed from. If the file changed
    /// in between — another tool, another window, the user's own editor — we stop
    /// and make them look at a fresh diff, because the only thing worse than not
    /// installing the hooks is silently reverting somebody else's edit.
    pub fn write(&self, install: bool, fingerprint: &str) -> Result<String, String> {
        let path = &self.path;
        let dir = path.parent().unwrap_or(Path::new("."));
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;

        // Read before the backup: an unreadable file must abort before we touch
        // anything at all.
        let current = self.read()?;
        if current_fingerprint(path) != fingerprint {
            return Err(format!(
                "{} changed since the preview. Nothing was written — review the new diff.",
                path.display()
            ));
        }

        let backup = backup_path(path);
        if path.exists() {
            std::fs::copy(path, &backup).map_err(|e| format!("backup failed: {e}"))?;
        }

        let next = if install { self.merged(&current) } else { without_ours(&current) };
        let mut text = pretty(&next);
        text.push('\n');

        // Write beside the target and rename over it: a crash or a full disk leaves
        // the original file intact rather than half a file.
        let temp = path.with_extension(format!("json.mika-{}", std::process::id()));
        std::fs::write(&temp, text.as_bytes()).map_err(|e| format!("write failed: {e}"))?;
        if let Err(err) = std::fs::rename(&temp, path) {
            let _ = std::fs::remove_file(&temp);
            return Err(format!("write failed: {err}"));
        }
        Ok(backup.to_string_lossy().to_string())
    }
}

/// The parsing half of `HookFile::read`, split out so it can be tested without
/// a file.
pub(crate) fn parse_settings(bytes: &[u8], path: &str) -> Result<Value, String> {
    // PowerShell writes a UTF-8 BOM with `Set-Content -Encoding utf8`, and
    // serde_json refuses it. Stripping it is safe and well defined; guessing at
    // anything else is not.
    let text = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    if text.iter().all(u8::is_ascii_whitespace) {
        return Ok(json!({}));
    }
    match serde_json::from_slice::<Value>(text) {
        Ok(v) if v.is_object() => Ok(v),
        Ok(_) => Err(format!("{path} isn't a JSON object — MIKA won't touch it.")),
        Err(err) => Err(format!(
            "{path} isn't valid JSON ({err}). Fix or move it, then try again — MIKA won't overwrite it."
        )),
    }
}

/// The program a hook command runs: its first token, quoted or not.
fn program_of(command: &str) -> &str {
    let command = command.trim_start();
    match command.strip_prefix('"') {
        Some(rest) => rest.split('"').next().unwrap_or(""),
        None => command.split_whitespace().next().unwrap_or(""),
    }
}

/// True when `command` runs our relay. It is exact on the program's file name
/// (`mika-hook.exe`, in any folder, so an older install location still counts):
/// a foreign hook that merely *mentions* "mika-hook" somewhere in its command
/// line is not ours, and must never be edited or removed.
pub(crate) fn is_our_command(command: &str) -> bool {
    let program = program_of(command).replace('\\', "/");
    program
        .rsplit('/')
        .next()
        .map(|file| file.eq_ignore_ascii_case(MARKER))
        .unwrap_or(false)
}

fn hook_is_ours(hook: &Value) -> bool {
    hook.get("command").and_then(Value::as_str).map(is_our_command).unwrap_or(false)
}

/// True when the entry holds at least one of our hooks.
pub(crate) fn entry_is_ours(entry: &Value) -> bool {
    entry
        .get("hooks")
        .and_then(Value::as_array)
        .map(|hooks| hooks.iter().any(hook_is_ours))
        .unwrap_or(false)
}

/// The entry with our hooks taken out. `None` when nothing else is left in it, so
/// a matcher group that shares an entry with another tool keeps that tool's hooks
/// instead of being deleted whole.
fn without_our_hooks(entry: &Value) -> Option<Value> {
    if !entry_is_ours(entry) {
        return Some(entry.clone());
    }
    let remaining: Vec<Value> = entry
        .get("hooks")
        .and_then(Value::as_array)
        .map(|hooks| hooks.iter().filter(|h| !hook_is_ours(h)).cloned().collect())
        .unwrap_or_default();
    if remaining.is_empty() {
        return None;
    }
    let mut entry = entry.clone();
    entry["hooks"] = Value::Array(remaining);
    Some(entry)
}

/// The file with every MIKA entry removed, and nothing else changed.
pub(crate) fn without_ours(existing: &Value) -> Value {
    let mut root = existing.as_object().cloned().unwrap_or_default();
    let Some(hooks) = root.get("hooks").and_then(Value::as_object).cloned() else {
        return Value::Object(root);
    };
    let mut out = Map::new();
    for (event, value) in hooks {
        match value.as_array() {
            Some(list) => {
                let kept: Vec<Value> =
                    list.iter().filter_map(without_our_hooks).collect();
                if !kept.is_empty() {
                    out.insert(event, Value::Array(kept));
                }
            }
            None => {
                out.insert(event, value);
            }
        }
    }
    if out.is_empty() {
        root.remove("hooks");
    } else {
        root.insert("hooks".into(), Value::Object(out));
    }
    Value::Object(root)
}

fn pretty(v: &Value) -> String {
    serde_json::to_string_pretty(v).unwrap_or_default()
}

/// Down to the second: installing then uninstalling in the same minute must not
/// quietly overwrite the first backup.
fn stamp() -> String {
    let t = unsafe { GetLocalTime() };
    format!(
        "{:04}{:02}{:02}-{:02}{:02}{:02}",
        t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond
    )
}

/// `<file>.bak-<timestamp>` (`settings.json.bak-…`, `hooks.json.bak-…`), with a
/// counter if that name is already taken: a backup must never overwrite an
/// older one.
pub(crate) fn backup_path(file: &Path) -> PathBuf {
    let name = file
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "settings.json".into());
    let base = format!("{name}.bak-{}", stamp());
    let mut candidate = file.with_file_name(&base);
    let mut n = 1;
    while candidate.exists() {
        candidate = file.with_file_name(format!("{base}-{n}"));
        n += 1;
    }
    candidate
}

/// Identifies the exact bytes a preview was computed from. FNV-1a is plenty:
/// the question is only "is this still the file I showed the user?".
pub(crate) fn fingerprint(bytes: &[u8]) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        hash ^= *b as u64;
        hash = hash.wrapping_mul(0x1000_0000_01b3);
    }
    format!("{hash:016x}")
}

fn current_fingerprint(path: &Path) -> String {
    match std::fs::read(path) {
        Ok(bytes) => fingerprint(&bytes),
        Err(_) => fingerprint(b""),
    }
}

// ── Minimal unified diff (LCS) ────────────────────────────────────────────────

/// Hook files are short, so a plain O(n·m) LCS is the simplest honest diff.
fn unified_diff(before: &str, after: &str) -> String {
    let a: Vec<&str> = before.lines().collect();
    let b: Vec<&str> = after.lines().collect();
    let (n, m) = (a.len(), b.len());

    let mut lcs = vec![vec![0usize; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            lcs[i][j] = if a[i] == b[j] {
                lcs[i + 1][j + 1] + 1
            } else {
                lcs[i + 1][j].max(lcs[i][j + 1])
            };
        }
    }

    let mut out: Vec<String> = Vec::new();
    let (mut i, mut j) = (0usize, 0usize);
    while i < n && j < m {
        if a[i] == b[j] {
            out.push(format!("  {}", a[i]));
            i += 1;
            j += 1;
        } else if lcs[i + 1][j] >= lcs[i][j + 1] {
            out.push(format!("- {}", a[i]));
            i += 1;
        } else {
            out.push(format!("+ {}", b[j]));
            j += 1;
        }
    }
    while i < n {
        out.push(format!("- {}", a[i]));
        i += 1;
    }
    while j < m {
        out.push(format!("+ {}", b[j]));
        j += 1;
    }

    // Keep three lines of context around each change so the panel stays readable.
    let changed: Vec<usize> = out
        .iter()
        .enumerate()
        .filter(|(_, l)| l.starts_with('+') || l.starts_with('-'))
        .map(|(i, _)| i)
        .collect();
    if changed.is_empty() {
        return "No change.".into();
    }
    let mut keep = vec![false; out.len()];
    for idx in changed {
        let lo = idx.saturating_sub(3);
        let hi = (idx + 4).min(out.len());
        keep[lo..hi].fill(true);
    }
    let mut result = String::new();
    let mut gap = false;
    for (idx, line) in out.iter().enumerate() {
        if keep[idx] {
            result.push_str(line);
            result.push('\n');
            gap = false;
        } else if !gap {
            result.push_str("  …\n");
            gap = true;
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    const WHERE: &str = "settings.json";

    /// A stand-in agent: two events, the relay at a fixed path.
    fn sample(path: PathBuf) -> HookFile {
        HookFile {
            path,
            events: &[("PreToolUse", 10), ("Stop", 10)],
            handler: |event, timeout| {
                json!({ "type": "command", "command": format!("\"C:/x/mika-hook.exe\" {event}"), "timeout": timeout })
            },
        }
    }

    #[test]
    fn a_utf8_bom_is_stripped_not_treated_as_corruption() {
        // PowerShell 5's `Set-Content -Encoding utf8` produces exactly this.
        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend_from_slice(br#"{"model":"opus","hooks":{}}"#);
        let parsed = parse_settings(&bytes, WHERE).expect("a BOM must not defeat the parser");
        assert_eq!(parsed["model"], "opus");
    }

    #[test]
    fn unreadable_content_is_an_error_never_an_empty_object() {
        // This is the whole bug: returning {} here meant `merged()` produced a
        // file containing nothing but MIKA's hooks, and the write replaced
        // everything the user had.
        for bad in [&b"{ not json"[..], &b"[1,2,3]"[..], &b"\"a string\""[..]] {
            assert!(
                parse_settings(bad, WHERE).is_err(),
                "content we cannot use must refuse, not come back empty"
            );
        }
    }

    #[test]
    fn empty_and_whitespace_files_start_from_nothing() {
        assert_eq!(parse_settings(b"", WHERE).unwrap(), json!({}));
        assert_eq!(parse_settings(b"  \n\t ", WHERE).unwrap(), json!({}));
    }

    #[test]
    fn a_fingerprint_notices_any_change() {
        assert_eq!(fingerprint(b"{}"), fingerprint(b"{}"));
        assert_ne!(fingerprint(b"{}"), fingerprint(b"{ }"));
        assert_ne!(fingerprint(b""), fingerprint(b"{}"));
    }

    #[test]
    fn only_the_relay_program_counts_as_ours() {
        assert!(is_our_command(r#""C:/Users/a/AppData/Local/MIKA/bin/mika-hook.exe" PreToolUse"#));
        assert!(is_our_command(r"C:\x\mika-hook.exe Stop"));
        assert!(is_our_command(r#""D:/other place/MIKA-HOOK.EXE" Stop"#));
        // Codex's form: backslashes, then the flag.
        assert!(is_our_command(r#""C:\Users\a\AppData\Local\MIKA\bin\mika-hook.exe" --codex Stop"#));
        for foreign in [
            "node C:/tools/mika-hook-wrapper.js",
            "other.exe --note mika-hook.exe",
            r#""C:/x/not-mika-hook.exe" Stop"#,
            "echo mika-hook",
            "",
        ] {
            assert!(!is_our_command(foreign), "{foreign:?} is not our relay");
        }
    }

    #[test]
    fn a_foreign_hook_sharing_our_entry_survives_install_and_uninstall() {
        let shared = json!({ "hooks": [
            { "type": "command", "command": "\"C:/x/mika-hook.exe\" Stop" },
            { "type": "command", "command": "keep-me.exe" }
        ]});
        let existing = json!({ "hooks": { "Stop": [shared] } });

        let cleaned = without_ours(&existing);
        assert_eq!(
            cleaned["hooks"]["Stop"][0]["hooks"],
            json!([{ "type": "command", "command": "keep-me.exe" }]),
            "uninstalling must only remove our own hook"
        );

        let after = sample(PathBuf::from(WHERE)).merged(&existing);
        let stop = after["hooks"]["Stop"].as_array().unwrap();
        assert!(stop.iter().any(|e| e.to_string().contains("keep-me.exe")), "the other tool's hook was dropped");
        assert_eq!(stop.iter().filter(|e| entry_is_ours(e)).count(), 1, "exactly one of our entries");
    }

    #[test]
    fn our_entry_goes_after_everybody_elses() {
        // Codex keys its trust on an entry's position: other tools' entries must
        // keep their index when ours is added.
        let existing = json!({ "hooks": { "Stop": [
            { "hooks": [{ "type": "command", "command": "first.exe" }] },
            { "matcher": "x", "hooks": [{ "type": "command", "command": "second.exe" }] }
        ]}});
        let after = sample(PathBuf::from(WHERE)).merged(&existing);
        let stop = after["hooks"]["Stop"].as_array().unwrap();
        assert_eq!(stop[0], existing["hooks"]["Stop"][0]);
        assert_eq!(stop[1], existing["hooks"]["Stop"][1]);
        assert!(entry_is_ours(&stop[2]));
        // Installing twice changes nothing.
        assert_eq!(sample(PathBuf::from(WHERE)).merged(&after), after);
    }

    #[test]
    fn a_hook_that_only_mentions_our_name_is_left_alone() {
        let foreign = json!({ "hooks": [{ "type": "command", "command": "node mika-hook-wrapper.js" }] });
        let existing = json!({ "hooks": { "Stop": [foreign.clone()] } });
        assert!(!entry_is_ours(&foreign));
        assert_eq!(without_ours(&existing), existing);
    }

    #[test]
    fn backups_never_reuse_a_name_and_keep_the_file_name() {
        let tmp = std::env::temp_dir().join(format!("mika-bak-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        let settings = tmp.join("settings.json");
        let first = backup_path(&settings);
        assert!(first.file_name().unwrap().to_string_lossy().starts_with("settings.json.bak-"));
        std::fs::write(&first, b"x").unwrap();
        let second = backup_path(&settings);
        assert_ne!(first, second, "a second backup in the same second must get its own name");
        let hooks = backup_path(&tmp.join("hooks.json"));
        assert!(hooks.file_name().unwrap().to_string_lossy().starts_with("hooks.json.bak-"));
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
