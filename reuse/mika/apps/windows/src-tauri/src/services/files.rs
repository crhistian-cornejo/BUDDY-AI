// Dropped files are copied into %LOCALAPPDATA%\MIKA\inbox so the original is
// never touched and the copy survives the drag source going away.
// The inbox is swept of anything older than a week, as on macOS.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime};

use serde::Serialize;

use super::settings;

const KEEP_FOR: Duration = Duration::from_secs(7 * 24 * 60 * 60);

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct DroppedFile {
    pub name: String,
    pub path: String,
    pub size: u64,
}

pub fn inbox_dir() -> PathBuf {
    settings::local_dir().join("inbox")
}

/// `path` resolved (symlinks and junctions followed), but only when the result
/// really lives inside `inbox`. Pure so it can be tested without touching the
/// real inbox.
pub(crate) fn confined_to(inbox: &Path, path: &str) -> Option<PathBuf> {
    let inbox = inbox.canonicalize().ok()?;
    let target = Path::new(path).canonicalize().ok()?;
    target.starts_with(&inbox).then_some(target)
}

/// The chat may only read files MIKA itself copied into the inbox. The path comes
/// from the webview; without this, a compromised page could name any file on the
/// machine and have it sent to the API.
pub fn confined_to_inbox(path: &str) -> Option<PathBuf> {
    confined_to(&inbox_dir(), path)
}

/// How long a drop stays valid for `ingest_file`.
const GRANT_TTL: Duration = Duration::from_secs(120);

/// Paths the operating system itself reported as dropped on the island.
/// `ingest_file` accepts nothing else: the webview is told *what* was dropped, but
/// it cannot make up a path and have MIKA copy (and then read) an arbitrary file.
#[derive(Default)]
pub struct DropGrants(Mutex<Vec<(PathBuf, Instant)>>);

impl DropGrants {
    pub fn grant<P: AsRef<Path>>(&self, paths: &[P]) {
        let now = Instant::now();
        let mut list = self.0.lock().unwrap();
        list.retain(|(_, at)| now.duration_since(*at) < GRANT_TTL);
        for p in paths {
            let p = p.as_ref();
            list.push((p.canonicalize().unwrap_or_else(|_| p.to_path_buf()), now));
        }
    }

    pub fn allows(&self, path: &str) -> bool {
        let candidate = Path::new(path);
        let candidate = candidate.canonicalize().unwrap_or_else(|_| candidate.to_path_buf());
        let now = Instant::now();
        let mut list = self.0.lock().unwrap();
        list.retain(|(_, at)| now.duration_since(*at) < GRANT_TTL);
        list.iter().any(|(granted, _)| *granted == candidate)
    }
}

pub fn ingest(source: &str) -> Result<DroppedFile, String> {
    let src = Path::new(source);
    let meta = std::fs::metadata(src).map_err(|e| format!("cannot read {source}: {e}"))?;
    if meta.is_dir() {
        return Err("Folders can't be dropped yet.".into());
    }

    let dir = inbox_dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;

    let name = src
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "file".into());

    let mut dest = dir.join(&name);
    if dest.exists() {
        let stem = src.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        let ext = src.extension().map(|s| format!(".{}", s.to_string_lossy())).unwrap_or_default();
        for i in 2..1000 {
            let candidate = dir.join(format!("{stem} ({i}){ext}"));
            if !candidate.exists() {
                dest = candidate;
                break;
            }
        }
    }

    std::fs::copy(src, &dest).map_err(|e| format!("cannot copy: {e}"))?;
    // CopyFileEx carries the source's timestamps across, so a file last edited
    // three years ago would arrive already older than the sweep window and be
    // deleted on the spot. The inbox ages from when *we* copied it.
    if let Ok(file) = std::fs::File::options().write(true).open(&dest) {
        let _ = file.set_modified(SystemTime::now());
    }
    sweep(&dir);

    Ok(DroppedFile {
        name,
        path: dest.to_string_lossy().to_string(),
        size: meta.len(),
    })
}

/// Drops anything copied here more than a week ago. `ingest` stamps every copy
/// with the time it landed, so this really is the age of the copy and not the
/// age of whatever the user happened to drag in.
fn sweep(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let now = SystemTime::now();
    for entry in entries.flatten() {
        let Ok(meta) = entry.metadata() else { continue };
        let Ok(copied) = meta.modified() else { continue };
        if now.duration_since(copied).map(|age| age > KEEP_FOR).unwrap_or(false) {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ingest_copies_and_never_overwrites() {
        let tmp = std::env::temp_dir().join(format!("mika-test-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        let source = tmp.join("note.txt");
        std::fs::write(&source, b"hello").unwrap();

        let first = ingest(source.to_str().unwrap()).unwrap();
        assert_eq!(first.name, "note.txt");
        assert_eq!(std::fs::read(&first.path).unwrap(), b"hello");

        // A second drop of the same name must not clobber the first copy.
        std::fs::write(&source, b"second").unwrap();
        let second = ingest(source.to_str().unwrap()).unwrap();
        assert_ne!(first.path, second.path);
        assert_eq!(std::fs::read(&first.path).unwrap(), b"hello");
        assert_eq!(std::fs::read(&second.path).unwrap(), b"second");

        // Folders are refused rather than silently ignored.
        assert!(ingest(tmp.to_str().unwrap()).is_err());

        // An ancient source must not arrive already older than the sweep window.
        let old_source = tmp.join("ancient.txt");
        std::fs::write(&old_source, b"old").unwrap();
        let long_ago = SystemTime::now() - KEEP_FOR - Duration::from_secs(60 * 60);
        std::fs::File::options()
            .write(true)
            .open(&old_source)
            .unwrap()
            .set_modified(long_ago)
            .unwrap();
        let aged = ingest(old_source.to_str().unwrap()).unwrap();
        assert!(
            Path::new(&aged.path).exists(),
            "a file copied just now was swept as if it were a week old"
        );
        let _ = std::fs::remove_file(&aged.path);

        let _ = std::fs::remove_file(&first.path);
        let _ = std::fs::remove_file(&second.path);
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn the_chat_can_only_read_inside_the_inbox() {
        let tmp = std::env::temp_dir().join(format!("mika-confine-{}", std::process::id()));
        let inbox = tmp.join("inbox");
        std::fs::create_dir_all(&inbox).unwrap();
        let inside = inbox.join("a.txt");
        let outside = tmp.join("secret.txt");
        std::fs::write(&inside, b"ok").unwrap();
        std::fs::write(&outside, b"no").unwrap();

        assert!(confined_to(&inbox, inside.to_str().unwrap()).is_some());
        assert!(confined_to(&inbox, outside.to_str().unwrap()).is_none());
        // `..` must not climb out.
        let sneaky = format!("{}\\..\\secret.txt", inbox.display());
        assert!(confined_to(&inbox, &sneaky).is_none(), "{sneaky}");
        assert!(confined_to(&inbox, inbox.join("missing.txt").to_str().unwrap()).is_none());
        // The inbox directory itself is not a readable file target either way, but
        // a sibling that merely shares its prefix must be refused.
        let sibling = tmp.join("inbox-evil");
        std::fs::create_dir_all(&sibling).unwrap();
        let evil = sibling.join("b.txt");
        std::fs::write(&evil, b"no").unwrap();
        assert!(confined_to(&inbox, evil.to_str().unwrap()).is_none());

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn only_paths_the_os_reported_can_be_ingested() {
        let tmp = std::env::temp_dir().join(format!("mika-grants-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        let dropped = tmp.join("dropped.txt");
        let invented = tmp.join("invented.txt");
        std::fs::write(&dropped, b"x").unwrap();
        std::fs::write(&invented, b"x").unwrap();

        let grants = DropGrants::default();
        assert!(!grants.allows(dropped.to_str().unwrap()), "nothing is allowed before a drop");
        grants.grant(std::slice::from_ref(&dropped));
        assert!(grants.allows(dropped.to_str().unwrap()));
        assert!(!grants.allows(invented.to_str().unwrap()), "a path the OS never reported");

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
