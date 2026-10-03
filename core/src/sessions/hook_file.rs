// Ported from MIKA (MIT, revision d050bc5): apps/windows/src-tauri/src/agents/hook_file.rs
//! The JSON hooks file a coding agent reads — Claude Code's `~/.claude/settings.json`, Codex's
//! `~/.codex/hooks.json` — and the one way Buddy ever changes it.
//!
//! The rule is strict and is followed to the letter: read the file, take a dated backup, merge without touching
//! anybody else's hooks, show the diff, and write only after an explicit click. Uninstall removes Buddy's entries
//! and nothing else.
//!
//! Both agents use the same shape —
//!   `{ "hooks": { "<Event>": [ { "matcher"?, "hooks": [ { "type": "command", … } ] } ] } }`
//! — so one implementation serves both. Each agent module only says where its file is, which events it wants and
//! what one handler looks like (`HookFile`). Buddy's handlers are recognised by the program they run
//! (`buddy-hook`, `buddy-hook.exe`).

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde_json::{Map, Value, json};

/// File name of the relay (without `.exe`); identifies Buddy's hooks inside any agent's file.
const MARKER: &str = "buddy-hook";

/// What the user is shown before Buddy edits an agent's hooks file.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct HookPreview {
    /// The agent's hooks file.
    pub path: String,
    /// A unified-style diff of the change (`+`/`-` lines with a little context).
    pub diff: String,
    /// Identifies the bytes this diff was computed from; handed back to `hooks_write` so only what the user actually
    /// looked at is ever applied.
    pub fingerprint: String,
}

/// One agent's hooks file and what Buddy puts in it.
pub(crate) struct HookFile {
    pub path: PathBuf,
    /// The relay the handlers run (`<data_dir>/bin/buddy-hook`).
    pub relay: PathBuf,
    /// Every event Buddy listens to, with the timeout (seconds) written for it.
    pub events: &'static [(&'static str, u64)],
    /// Buddy's handler object for one event: `{"type":"command","command":…,…}`.
    pub handler: fn(relay: &Path, event: &str, timeout: u64) -> Value,
    /// How the file nests its events.
    pub layout: Layout,
}

/// How an agent's hooks file nests events and handlers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Layout {
    /// Claude Code and Codex: `{ "hooks": { "<Event>": [ { "matcher"?, "hooks": [handler…] } ] } }`.
    Hooks,
    /// Antigravity: `{ "<hook name>": { "enabled"?, "<Event>": [ … ] } }`, one named hook per tool. Buddy's lives
    /// under `name`. Tool events (`grouped`) hold `{ "matcher", "hooks": [handler…] }` groups; every other event
    /// holds the handlers themselves.
    Named { name: &'static str, grouped: &'static [&'static str] },
}

/// The user's home folder (`HOME` on Unix, `USERPROFILE` on Windows). There is deliberately no fallback: guessing
/// "." would put an agent's config wherever the process happens to run. `what` names the file we were looking for,
/// for the error.
pub(crate) fn home(what: &str) -> Result<PathBuf, String> {
    let var = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    std::env::var_os(var)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| format!("{var} no está definida, así que Buddy no sabe dónde está {what}."))
}

/// `path` as a double-quoted shell word. On Unix the agents run hooks through a POSIX shell, so the characters that
/// stay special inside double quotes are escaped; on Windows (cmd.exe, Git Bash) paths cannot contain `"` and are
/// quoted as they are.
pub(crate) fn quoted(path: &str) -> String {
    if cfg!(windows) {
        return format!("\"{path}\"");
    }
    let mut out = String::with_capacity(path.len() + 2);
    out.push('"');
    for c in path.chars() {
        if matches!(c, '"' | '\\' | '$' | '`') {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('"');
    out
}

impl HookFile {
    /// Reads the file.
    ///
    /// The only error that means "start from nothing" is the file not being there. Everything else — a lock held by
    /// another process, a permission problem, JSON we cannot parse — is reported, because the alternative is
    /// treating somebody's unreadable config as an empty object and then writing that back over it.
    fn read(&self) -> Result<Value, String> {
        match std::fs::read(&self.path) {
            Ok(bytes) => parse_settings(&bytes, &self.path.display().to_string()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(json!({})),
            Err(err) => Err(format!("No se puede leer {}: {err}", self.path.display())),
        }
    }

    /// The file with Buddy's hooks added; everything else is left untouched. Our entries go last in each event's
    /// list, so the position of every other tool's entry stays what it was.
    pub(crate) fn merged(&self, existing: &Value) -> Value {
        let (key, grouped): (&str, &[&str]) = match self.layout {
            Layout::Hooks => ("hooks", &[]),
            Layout::Named { name, grouped } => (name, grouped),
        };
        // A named layout may hold a copy of ours under another name (the user renamed it): one set of hooks only.
        let base = if self.layout == Layout::Hooks { existing.clone() } else { self.cleaned(existing) };
        let mut root = base.as_object().cloned().unwrap_or_default();
        let mut hooks = root.get(key).and_then(Value::as_object).cloned().unwrap_or_else(Map::new);

        for (event, timeout) in self.events {
            let mut list = hooks.get(*event).and_then(Value::as_array).cloned().unwrap_or_default();
            list = list.iter().filter_map(without_ours_in).collect();
            let handler = (self.handler)(&self.relay, event, *timeout);
            list.push(match self.layout {
                Layout::Hooks => json!({ "hooks": [handler] }),
                Layout::Named { .. } if grouped.contains(event) => json!({ "matcher": "*", "hooks": [handler] }),
                Layout::Named { .. } => handler,
            });
            hooks.insert((*event).to_string(), Value::Array(list));
        }

        root.insert(key.into(), Value::Object(hooks));
        Value::Object(root)
    }

    /// The file with every Buddy entry removed, and nothing else changed.
    pub(crate) fn cleaned(&self, existing: &Value) -> Value {
        match self.layout {
            Layout::Hooks => without_ours(existing),
            Layout::Named { .. } => without_ours_named(existing),
        }
    }

    /// True when the file holds at least one of Buddy's hooks. Never fails: an unreadable file counts as "not
    /// installed" here (anything that writes uses `read()` and surfaces the error instead).
    pub fn installed(&self) -> bool {
        let Ok(current) = self.read() else { return false };
        let ours = |events: &Value| {
            events.as_object().is_some_and(|m| m.values().filter_map(Value::as_array).flatten().any(is_ours_in))
        };
        match self.layout {
            Layout::Hooks => current.get("hooks").is_some_and(ours),
            Layout::Named { .. } => current.as_object().is_some_and(|root| root.values().any(ours)),
        }
    }

    pub fn preview(&self, install: bool) -> Result<HookPreview, String> {
        let current = self.read()?;
        let next = if install { self.merged(&current) } else { self.cleaned(&current) };
        Ok(HookPreview {
            path: self.path.to_string_lossy().to_string(),
            diff: unified_diff(&pretty(&current), &pretty(&next)),
            fingerprint: current_fingerprint(&self.path),
        })
    }

    /// Writes the merged (or cleaned) file after taking a dated backup, and returns the backup's path (empty when
    /// there was no file to back up).
    ///
    /// `fingerprint` is the one the preview was computed from. If the file changed in between — another tool,
    /// another window, the user's own editor — we stop and make them look at a fresh diff, because the only thing
    /// worse than not installing the hooks is silently reverting somebody else's edit.
    pub fn write(&self, install: bool, fingerprint: &str) -> Result<String, String> {
        let path = &self.path;
        let dir = path.parent().unwrap_or(Path::new("."));
        std::fs::create_dir_all(dir).map_err(|e| format!("No se puede crear {}: {e}", dir.display()))?;

        // Read before the backup: an unreadable file must abort before we touch anything at all.
        let current = self.read()?;
        if current_fingerprint(path) != fingerprint {
            return Err(format!(
                "{} cambió desde la vista previa. No se escribió nada: revisa los cambios de nuevo.",
                path.display()
            ));
        }

        let mut backup = String::new();
        if path.exists() {
            let target = backup_path(path);
            std::fs::copy(path, &target).map_err(|e| format!("No se pudo hacer la copia de seguridad: {e}"))?;
            backup = target.to_string_lossy().to_string();
        }

        let next = if install { self.merged(&current) } else { self.cleaned(&current) };
        let mut text = pretty(&next);
        text.push('\n');

        // Write beside the target and rename over it: a crash or a full disk leaves the original file intact rather
        // than half a file.
        let temp = path.with_extension(format!("json.buddy-{}", std::process::id()));
        std::fs::write(&temp, text.as_bytes()).map_err(|e| format!("No se pudo escribir: {e}"))?;
        if let Err(err) = std::fs::rename(&temp, path) {
            let _ = std::fs::remove_file(&temp);
            return Err(format!("No se pudo escribir: {err}"));
        }
        Ok(backup)
    }
}

/// The parsing half of `HookFile::read`, split out so it can be tested without a file.
pub(crate) fn parse_settings(bytes: &[u8], path: &str) -> Result<Value, String> {
    // PowerShell writes a UTF-8 BOM with `Set-Content -Encoding utf8`, and serde_json refuses it. Stripping it is
    // safe and well defined; guessing at anything else is not.
    let text = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    if text.iter().all(u8::is_ascii_whitespace) {
        return Ok(json!({}));
    }
    match serde_json::from_slice::<Value>(text) {
        Ok(v) if v.is_object() => Ok(v),
        Ok(_) => Err(format!("{path} no es un objeto JSON: Buddy no lo va a tocar.")),
        Err(err) => Err(format!(
            "{path} no es JSON válido ({err}). Corrígelo o muévelo y vuelve a intentarlo: Buddy no lo va a sobrescribir."
        )),
    }
}

/// The program a hook command runs: its first token, quoted or not. Inside double quotes a backslash before
/// `"`, `\`, `$` or `` ` `` is a POSIX escape; any other backslash (a Windows path) stays as it is.
fn program_of(command: &str) -> String {
    let command = command.trim_start();
    let Some(rest) = command.strip_prefix('"') else {
        return command.split_whitespace().next().unwrap_or("").to_string();
    };
    let mut out = String::new();
    let mut chars = rest.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' => break,
            '\\' if matches!(chars.peek(), Some('"' | '\\' | '$' | '`')) => out.push(chars.next().unwrap_or('\\')),
            _ => out.push(c),
        }
    }
    out
}

/// True when `command` runs our relay. It is exact on the program's file name (`buddy-hook` or `buddy-hook.exe`,
/// in any folder, so an older install location still counts): a foreign hook that merely *mentions* "buddy-hook"
/// somewhere in its command line is not ours, and must never be edited or removed.
pub(crate) fn is_our_command(command: &str) -> bool {
    let program = program_of(command).replace('\\', "/");
    program
        .rsplit('/')
        .next()
        .map(|file| {
            let file = file.to_ascii_lowercase();
            file == MARKER || file.strip_suffix(".exe") == Some(MARKER)
        })
        .unwrap_or(false)
}

fn hook_is_ours(hook: &Value) -> bool {
    hook.get("command").and_then(Value::as_str).map(is_our_command).unwrap_or(false)
}

/// True when the entry holds at least one of our hooks.
pub(crate) fn entry_is_ours(entry: &Value) -> bool {
    entry.get("hooks").and_then(Value::as_array).map(|hooks| hooks.iter().any(hook_is_ours)).unwrap_or(false)
}

/// The entry with our hooks taken out. `None` when nothing else is left in it, so a matcher group that shares an
/// entry with another tool keeps that tool's hooks instead of being deleted whole.
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

/// True when one item of an event's list is ours: a group holding our handler, or (Antigravity's flat events) our
/// handler itself.
fn is_ours_in(item: &Value) -> bool {
    hook_is_ours(item) || entry_is_ours(item)
}

/// One item of an event's list without our handlers; `None` when nothing else is left of it.
fn without_ours_in(item: &Value) -> Option<Value> {
    if hook_is_ours(item) {
        return None;
    }
    without_our_hooks(item)
}

/// Antigravity's file (`{ "<hook name>": { "<Event>": [ … ] } }`) with every Buddy handler removed, under whatever
/// name it sits. A named hook we emptied goes away whole (its `enabled` flag included); everything else stays as
/// it was.
pub(crate) fn without_ours_named(existing: &Value) -> Value {
    let Some(root) = existing.as_object() else { return existing.clone() };
    let mut out = Map::new();
    for (name, hook) in root {
        let Some(events) = hook.as_object() else {
            out.insert(name.clone(), hook.clone());
            continue;
        };
        let mut kept = Map::new();
        let mut removed = false;
        for (key, value) in events {
            match value.as_array() {
                Some(list) => {
                    let left: Vec<Value> = list.iter().filter_map(without_ours_in).collect();
                    let changed = left.as_slice() != list.as_slice();
                    removed |= changed;
                    if !(changed && left.is_empty()) {
                        kept.insert(key.clone(), Value::Array(left));
                    }
                }
                None => {
                    kept.insert(key.clone(), value.clone());
                }
            }
        }
        if removed && !kept.values().any(Value::is_array) {
            continue;
        }
        out.insert(name.clone(), Value::Object(kept));
    }
    Value::Object(out)
}

/// The file with every Buddy entry removed, and nothing else changed.
pub(crate) fn without_ours(existing: &Value) -> Value {
    let mut root = existing.as_object().cloned().unwrap_or_default();
    let Some(hooks) = root.get("hooks").and_then(Value::as_object).cloned() else {
        return Value::Object(root);
    };
    let mut out = Map::new();
    for (event, value) in hooks {
        match value.as_array() {
            Some(list) => {
                let kept: Vec<Value> = list.iter().filter_map(without_our_hooks).collect();
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

/// Down to the second, in UTC: installing then uninstalling in the same minute must not quietly overwrite the first
/// backup.
fn stamp() -> String {
    let [y, mo, d, h, mi, s] = crate::log::utc_parts(SystemTime::now());
    format!("{y:04}{mo:02}{d:02}-{h:02}{mi:02}{s:02}Z")
}

/// `<file>.bak-<timestamp>` (`settings.json.bak-…`, `hooks.json.bak-…`), with a counter if that name is already
/// taken: a backup must never overwrite an older one.
pub(crate) fn backup_path(file: &Path) -> PathBuf {
    let name = file.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "settings.json".into());
    let base = format!("{name}.bak-{}", stamp());
    let mut candidate = file.with_file_name(&base);
    let mut n = 1;
    while candidate.exists() {
        candidate = file.with_file_name(format!("{base}-{n}"));
        n += 1;
    }
    candidate
}

/// Identifies the exact bytes a preview was computed from. FNV-1a is plenty: the question is only "is this still
/// the file I showed the user?".
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
            lcs[i][j] = if a[i] == b[j] { lcs[i + 1][j + 1] + 1 } else { lcs[i + 1][j].max(lcs[i][j + 1]) };
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
    let changed: Vec<usize> =
        out.iter().enumerate().filter(|(_, l)| l.starts_with('+') || l.starts_with('-')).map(|(i, _)| i).collect();
    if changed.is_empty() {
        return "Sin cambios.".into();
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

/// Serializes the tests that move `HOME` / `USERPROFILE` / `CODEX_HOME`: the environment is process-wide.
#[cfg(test)]
pub(crate) static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
mod tests {
    use super::*;

    const WHERE: &str = "settings.json";

    /// A stand-in agent: two events, the relay at a fixed path.
    fn sample(path: PathBuf) -> HookFile {
        HookFile {
            path,
            relay: PathBuf::from("/x/bin/buddy-hook"),
            events: &[("PreToolUse", 10), ("Stop", 10)],
            handler: |relay, event, timeout| {
                json!({ "type": "command", "command": format!("{} {event}", quoted(&relay.to_string_lossy())), "timeout": timeout })
            },
            layout: Layout::Hooks,
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
        // Returning {} here would mean `merged()` produced a file containing nothing but Buddy's hooks, and the write
        // replaced everything the user had.
        for bad in [&b"{ not json"[..], &b"[1,2,3]"[..], &b"\"a string\""[..]] {
            assert!(parse_settings(bad, WHERE).is_err(), "content we cannot use must refuse, not come back empty");
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
        assert!(is_our_command(r#""/Users/a/Library/Application Support/Buddy/bin/buddy-hook" PreToolUse"#));
        assert!(is_our_command("/home/a/.local/share/buddy/bin/buddy-hook --codex Stop"));
        assert!(is_our_command(r#""C:/Users/a/AppData/Local/Buddy/bin/buddy-hook.exe" PreToolUse"#));
        assert!(is_our_command(r"C:\x\buddy-hook.exe Stop"));
        assert!(is_our_command(r#""D:/other place/BUDDY-HOOK.EXE" Stop"#));
        // Codex's Windows form: backslashes, then the flag.
        assert!(is_our_command(r#""C:\Users\a\AppData\Local\Buddy\bin\buddy-hook.exe" --codex Stop"#));
        // A Unix path with characters escaped inside the quotes.
        assert!(is_our_command(r#""/Users/a \"b\" \$c/bin/buddy-hook" Stop"#));
        for foreign in [
            "node /tools/buddy-hook-wrapper.js",
            "other.exe --note buddy-hook",
            r#""C:/x/not-buddy-hook.exe" Stop"#,
            "/x/buddy-hook.sh Stop",
            "echo buddy-hook",
            "",
        ] {
            assert!(!is_our_command(foreign), "{foreign:?} is not our relay");
        }
    }

    #[test]
    fn unix_quoting_round_trips_through_the_parser() {
        if cfg!(windows) {
            return;
        }
        let path = r#"/Users/a "b" $c `d` \e/bin/buddy-hook"#;
        let q = quoted(path);
        assert_eq!(program_of(&format!("{q} Stop")), path);
        assert_eq!(quoted("/Users/a/Library/Application Support/Buddy/bin/buddy-hook"), "\"/Users/a/Library/Application Support/Buddy/bin/buddy-hook\"");
    }

    #[test]
    fn a_foreign_hook_sharing_our_entry_survives_install_and_uninstall() {
        let shared = json!({ "hooks": [
            { "type": "command", "command": "\"/x/bin/buddy-hook\" Stop" },
            { "type": "command", "command": "keep-me" }
        ]});
        let existing = json!({ "hooks": { "Stop": [shared] } });

        let cleaned = without_ours(&existing);
        assert_eq!(
            cleaned["hooks"]["Stop"][0]["hooks"],
            json!([{ "type": "command", "command": "keep-me" }]),
            "uninstalling must only remove our own hook"
        );

        let after = sample(PathBuf::from(WHERE)).merged(&existing);
        let stop = after["hooks"]["Stop"].as_array().unwrap();
        assert!(stop.iter().any(|e| e.to_string().contains("keep-me")), "the other tool's hook was dropped");
        assert_eq!(stop.iter().filter(|e| entry_is_ours(e)).count(), 1, "exactly one of our entries");
    }

    #[test]
    fn our_entry_goes_after_everybody_elses() {
        // Codex keys its trust on an entry's position: other tools' entries must keep their index when ours is added.
        let existing = json!({ "hooks": { "Stop": [
            { "hooks": [{ "type": "command", "command": "first" }] },
            { "matcher": "x", "hooks": [{ "type": "command", "command": "second" }] }
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
        let foreign = json!({ "hooks": [{ "type": "command", "command": "node buddy-hook-wrapper.js" }] });
        let existing = json!({ "hooks": { "Stop": [foreign.clone()] } });
        assert!(!entry_is_ours(&foreign));
        assert_eq!(without_ours(&existing), existing);
    }

    #[test]
    fn the_diff_shows_what_changes_and_says_when_nothing_does() {
        let diff = unified_diff("{\n  \"a\": 1\n}", "{\n  \"a\": 2\n}");
        assert!(diff.contains("-   \"a\": 1") && diff.contains("+   \"a\": 2"), "got {diff}");
        assert_eq!(unified_diff("x", "x"), "Sin cambios.");
    }

    #[test]
    fn backups_never_reuse_a_name_and_keep_the_file_name() {
        let tmp = tempfile::tempdir().unwrap();
        let settings = tmp.path().join("settings.json");
        let first = backup_path(&settings);
        assert!(first.file_name().unwrap().to_string_lossy().starts_with("settings.json.bak-"));
        std::fs::write(&first, b"x").unwrap();
        let second = backup_path(&settings);
        assert_ne!(first, second, "a second backup in the same second must get its own name");
        let hooks = backup_path(&tmp.path().join("hooks.json"));
        assert!(hooks.file_name().unwrap().to_string_lossy().starts_with("hooks.json.bak-"));
    }
}
