//! «Permitir siempre»: a command the user allowed once can be allowed for good, by its program and subcommand
//! (`git status`, `npm run test`), per asker (Buddy's own agents, or the user's Claude Code / Codex / Gemini
//! sessions). Only plain commands qualify: nothing chained, piped or redirected, and never a dangerous program.
//! The rules live in `<data>/permisos-siempre.json` and are listed (and removable) in Settings.

use std::path::{Path, PathBuf};

const FILE: &str = "permisos-siempre.json";

/// Programs never allowed for good, whatever the subcommand.
const DANGEROUS: [&str; 34] = [
    "rm", "rmdir", "sudo", "su", "doas", "dd", "mkfs", "fdisk", "diskutil", "chmod", "chown", "chgrp", "curl",
    "wget", "ssh", "scp", "sftp", "rsync", "nc", "ncat", "kill", "killall", "pkill", "shutdown", "reboot", "halt",
    "launchctl", "osascript", "security", "defaults", "eval", "exec", "xargs", "find",
];
/// Interpreters: allowing «python» would allow any script.
const INTERPRETERS: [&str; 12] = ["sh", "bash", "zsh", "fish", "python", "python3", "node", "ruby", "perl", "php", "pwsh", "powershell"];
/// Subcommands that take one more word to mean something (`npm run test`).
const RUNNERS: [&str; 4] = ["run", "exec", "x", "script"];

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct AlwaysRule {
    /// Who asked: "buddy" (Buddy's agents), "claude", "codex" or "antigravity" (the user's sessions).
    pub agent: String,
    /// Program and subcommand, e.g. «git status».
    pub prefix: String,
    /// Unix seconds.
    pub added_at: i64,
}

/// A shell wrapper (`/bin/zsh -lc 'git status'`, Codex's way) unwrapped to the command inside.
pub fn unwrap_shell(command: &str) -> String {
    let c = command.trim();
    let mut words = c.splitn(3, char::is_whitespace);
    let (Some(shell), Some(flag), Some(rest)) = (words.next(), words.next(), words.next()) else { return c.to_string() };
    let shell_name = shell.rsplit('/').next().unwrap_or(shell);
    if ["sh", "bash", "zsh"].contains(&shell_name) && ["-c", "-lc", "-ic"].contains(&flag) {
        let rest = rest.trim();
        let inner = rest
            .strip_prefix('\'')
            .and_then(|r| r.strip_suffix('\''))
            .or_else(|| rest.strip_prefix('"').and_then(|r| r.strip_suffix('"')))
            .unwrap_or(rest);
        return inner.trim().to_string();
    }
    c.to_string()
}

/// The rule a command would get («git status»), or None when it may not be allowed for good.
pub fn prefix_for(command: &str) -> Option<String> {
    let command = unwrap_shell(command);
    if command.is_empty() || ["|", ";", "&", ">", "<", "`", "$(", "\n", "\\"].iter().any(|c| command.contains(c)) {
        return None;
    }
    let words: Vec<&str> = command.split_whitespace().collect();
    let program = words.first()?.rsplit('/').next()?;
    if DANGEROUS.contains(&program) || INTERPRETERS.contains(&program) || program.contains('=') {
        return None;
    }
    let plain = |w: &&&str| !w.is_empty() && !w.starts_with('-') && w.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == ':');
    let mut prefix = vec![program.to_string()];
    if let Some(sub) = words.get(1).filter(plain) {
        prefix.push(sub.to_string());
        if RUNNERS.contains(sub) {
            prefix.push(words.get(2).filter(plain)?.to_string());
        }
    }
    Some(prefix.join(" "))
}

/// Whether `command` is covered by a rule with `prefix`.
/// (`git` alone covers any plain `git …`; `git status` covers `git status -s` but not `git push`.)
pub fn covers(prefix: &str, command: &str) -> bool {
    prefix_for(command).is_some_and(|own| own == prefix || own.starts_with(&format!("{prefix} ")))
}

fn path(data_dir: &Path) -> PathBuf {
    data_dir.join(FILE)
}

pub fn load(data_dir: &Path) -> Vec<AlwaysRule> {
    std::fs::read_to_string(path(data_dir)).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

pub fn save(data_dir: &Path, rules: &[AlwaysRule]) {
    if let Ok(text) = serde_json::to_string_pretty(rules) {
        let _ = std::fs::write(path(data_dir), text);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_commands_get_program_and_subcommand() {
        assert_eq!(prefix_for("git status").as_deref(), Some("git status"));
        assert_eq!(prefix_for("git status -s").as_deref(), Some("git status"));
        assert_eq!(prefix_for("/bin/zsh -lc 'git --version'").as_deref(), Some("git"));
        assert_eq!(prefix_for("npm run test -- --watch").as_deref(), Some("npm run test"));
        assert_eq!(prefix_for("cargo test -p core").as_deref(), Some("cargo test"));
        assert_eq!(prefix_for("ls -la /tmp").as_deref(), Some("ls"));
        assert_eq!(prefix_for("/usr/bin/git log").as_deref(), Some("git log"));
    }

    #[test]
    fn risky_commands_never_qualify() {
        for c in [
            "rm -rf build", "sudo ls", "git status && rm x", "ls | grep a", "echo hi > f", "curl https://x",
            "python script.py", "bash -c 'rm x'", "ls `pwd`", "echo $(whoami)", "A=1 ls", "npm run", "",
            "find . -delete", "xargs rm",
        ] {
            assert_eq!(prefix_for(c), None, "{c:?}");
        }
    }

    #[test]
    fn rules_cover_their_variants_only() {
        assert!(covers("git status", "git status -s"));
        assert!(covers("git status", "/bin/zsh -lc 'git status --short'"));
        assert!(!covers("git status", "git push"));
        assert!(!covers("git status", "git status; rm -rf ~"));
        assert!(covers("git", "git --version"));
        assert!(covers("ls", "ls -la"));
        assert!(!covers("npm run test", "npm run build"));
    }

    #[test]
    fn rules_round_trip_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        assert!(load(dir.path()).is_empty());
        let rules = vec![AlwaysRule { agent: "buddy".into(), prefix: "git status".into(), added_at: 1 }];
        save(dir.path(), &rules);
        assert_eq!(load(dir.path()), rules);
    }
}
