//! User-chosen notch tools, shared by both apps. References and saved text persist locally; nothing watches the clipboard.
use std::{path::Path, time::{SystemTime, UNIX_EPOCH}};
use serde::{Deserialize, Serialize};
use crate::{CoreError, store::Store};

pub mod calendar;
const KEY: &str = "notch.tools";
pub const MAX_FILES: usize = 32;
pub const MAX_CLIPS: usize = 20;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct ShelfFile { pub path: String, pub name: String, pub available: bool }

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct SavedClip { pub id: String, pub text: String, pub saved_at: i64 }

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct NotchTools {
    pub files: Vec<ShelfFile>,
    pub clips: Vec<SavedClip>,
    pub battery_enabled: bool,
    pub calendar_enabled: bool,
    pub clipboard_enabled: bool,
    pub calendar_path: Option<String>,
}

impl Default for NotchTools {
    fn default() -> Self {
        Self { files: vec![], clips: vec![], battery_enabled: true, calendar_enabled: true,
            clipboard_enabled: true, calendar_path: None }
    }
}

pub fn load(store: &Store) -> Result<NotchTools, CoreError> {
    let mut state: NotchTools = store.setting(KEY)?.and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
    for file in &mut state.files { file.available = Path::new(&file.path).exists(); }
    Ok(state)
}
fn save(store: &Store, state: NotchTools) -> Result<NotchTools, CoreError> {
    store.set_setting(KEY, &serde_json::to_string(&state).map_err(|e| CoreError::Io(e.to_string()))?)?;
    Ok(state)
}
fn display_path(path: &Path) -> String {
    let path = path.to_string_lossy();
    #[cfg(windows)] {
        if let Some(unc) = path.strip_prefix(r"\\?\UNC\") { return format!(r"\\{unc}"); }
        if let Some(disk) = path.strip_prefix(r"\\?\") { return disk.into(); }
    }
    path.into_owned()
}

pub fn add_files(store: &Store, paths: &[String]) -> Result<NotchTools, CoreError> {
    let mut state = load(store)?;
    for path in paths {
        let path = std::fs::canonicalize(path)?;
        let name = path.file_name().unwrap_or(path.as_os_str()).to_string_lossy().into_owned();
        let path = display_path(&path);
        if !state.files.iter().any(|f| f.path == path) {
            state.files.push(ShelfFile { path, name, available: true });
        }
    }
    if state.files.len() > MAX_FILES { return Err(CoreError::Io("La bandeja admite hasta 32 archivos o carpetas.".into())); }
    save(store, state)
}

pub fn remove_file(store: &Store, path: &str) -> Result<NotchTools, CoreError> {
    let mut state = load(store)?;
    let canonical = std::fs::canonicalize(path).ok().map(|p| display_path(&p));
    state.files.retain(|f| f.path != path && canonical.as_deref() != Some(&f.path));
    save(store, state)
}

pub fn add_clip(store: &Store, text: &str) -> Result<NotchTools, CoreError> {
    if text.trim().is_empty() { return Err(CoreError::Io("El portapapeles no contiene texto.".into())); }
    if text.len() > 100_000 { return Err(CoreError::Io("Guarda un texto de menos de 100 KB.".into())); }
    let mut state = load(store)?;
    let time = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
    let existing = state.clips.iter().find(|c| c.text == text).map(|c| c.id.clone());
    state.clips.retain(|c| c.text != text);
    state.clips.insert(0, SavedClip { id: existing.unwrap_or_else(|| format!("{:x}", time.as_nanos())),
        text: text.into(), saved_at: time.as_secs() as i64 });
    state.clips.truncate(MAX_CLIPS);
    save(store, state)
}

pub fn remove_clip(store: &Store, id: &str) -> Result<NotchTools, CoreError> {
    let mut state = load(store)?;
    state.clips.retain(|c| c.id != id);
    save(store, state)
}

pub fn widget(store: &Store, widget: &str, enabled: bool) -> Result<NotchTools, CoreError> {
    let mut state = load(store)?;
    match widget {
        "battery" => state.battery_enabled = enabled,
        "calendar" => state.calendar_enabled = enabled,
        "clipboard" => state.clipboard_enabled = enabled,
        _ => return Err(CoreError::Io("Widget desconocido.".into())),
    }
    save(store, state)
}

pub fn set_calendar(store: &Store, path: Option<String>) -> Result<NotchTools, CoreError> {
    let mut state = load(store)?;
    if let Some(path) = path {
        let path = std::fs::canonicalize(path)?;
        calendar::read(&path, calendar::now())?;
        state.calendar_path = Some(display_path(&path));
        state.calendar_enabled = true;
    } else { state.calendar_path = None; }
    save(store, state)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shelf_merges_deduplicates_persists_and_removal_never_deletes_files() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("notes.txt"); std::fs::write(&file, "hello").unwrap();
        let db = dir.path().join("buddy.sqlite");
        let store = Store::open(&db).unwrap();
        let path = file.to_string_lossy().into_owned();
        assert_eq!(add_files(&store, &[path.clone(), path.clone()]).unwrap().files.len(), 1);
        drop(store);
        let store = Store::open(&db).unwrap();
        assert_eq!(load(&store).unwrap().files[0].name, "notes.txt");
        remove_file(&store, &path).unwrap();
        assert!(file.exists()); assert!(load(&store).unwrap().files.is_empty());
    }
    #[test]
    fn clipboard_is_bounded_and_preserves_exact_text() {
        let store = Store::open_in_memory().unwrap();
        add_clip(&store, "  line\n").unwrap();
        add_clip(&store, "next").unwrap();
        let state = add_clip(&store, "  line\n").unwrap();
        assert_eq!(state.clips.len(), 2); assert_eq!(state.clips[0].text, "  line\n");
        for i in 0..25 { add_clip(&store, &format!("clip {i}")).unwrap(); }
        assert_eq!(load(&store).unwrap().clips.len(), MAX_CLIPS);
        assert!(add_clip(&store, "   ").is_err());
    }
    #[test]
    fn widget_choices_persist_and_missing_files_stay_removable() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("gone.txt"); std::fs::write(&file, "hello").unwrap();
        let store = Store::open_in_memory().unwrap();
        add_files(&store, &[file.to_string_lossy().into_owned()]).unwrap();
        std::fs::remove_file(file).unwrap();
        assert!(!load(&store).unwrap().files[0].available);
        widget(&store, "battery", false).unwrap();
        assert!(!load(&store).unwrap().battery_enabled);
        assert!(widget(&store, "unknown", true).is_err());
    }
}
