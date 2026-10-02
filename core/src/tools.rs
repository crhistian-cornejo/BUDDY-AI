//! Small productivity tools of the notch / top bar, the same on both apps: a focus timer and the user's pinned
//! shortcuts (apps, folders, files, web pages). Their state lives here; the apps only draw it and open things.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::events::{Event, EventBus};
use crate::{CoreError, store::Store};

/// What the focus timer is doing. Times are unix seconds, so each app draws the countdown itself (only while visible).
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct FocusStatus {
    pub running: bool,
    pub started_at: i64,
    pub ends_at: i64,
    pub minutes: u32,
}

impl FocusStatus {
    fn idle() -> Self {
        Self { running: false, started_at: 0, ends_at: 0, minutes: 0 }
    }
}

/// One focus block at a time. A thread sleeps until the end (no ticking) and then says so on the bus.
#[derive(Default)]
pub struct Focus {
    status: Mutex<Option<FocusStatus>>,
    generation: Arc<AtomicU64>,
}

impl Focus {
    pub fn start(&self, minutes: u32, bus: &Arc<EventBus>) -> FocusStatus {
        let minutes = minutes.clamp(1, 180);
        let now = now();
        let status = FocusStatus { running: true, started_at: now, ends_at: now + i64::from(minutes) * 60, minutes };
        *self.status.lock().unwrap() = Some(status.clone());
        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        let (current, bus) = (self.generation.clone(), bus.clone());
        bus.publish(Event::FocusChanged { running: true, ends_at: status.ends_at });
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_secs(u64::from(minutes) * 60));
            if current.load(Ordering::SeqCst) == generation {
                bus.publish(Event::FocusFinished { minutes });
                bus.publish(Event::MascotState { state: "done".into() });
            }
        });
        status
    }

    pub fn stop(&self, bus: &EventBus) {
        self.generation.fetch_add(1, Ordering::SeqCst);
        *self.status.lock().unwrap() = None;
        bus.publish(Event::FocusChanged { running: false, ends_at: 0 });
    }

    pub fn status(&self) -> FocusStatus {
        let mut guard = self.status.lock().unwrap();
        match guard.as_ref() {
            Some(s) if s.ends_at > now() => s.clone(),
            _ => {
                *guard = None;
                FocusStatus::idle()
            }
        }
    }
}

/// A pinned thing to open with one click.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct Shortcut {
    pub id: String,
    pub name: String,
    /// A file system path (app, folder, file) or an http(s) address.
    pub target: String,
    /// "app" | "folder" | "file" | "web"
    pub kind: String,
}

const SHORTCUTS_KEY: &str = "tools.shortcuts";
pub const MAX_SHORTCUTS: usize = 8;

pub fn shortcuts(store: &Store) -> Result<Vec<Shortcut>, CoreError> {
    Ok(store.setting(SHORTCUTS_KEY)?.and_then(|json| serde_json::from_str(&json).ok()).unwrap_or_default())
}

/// Pins `target` (a path or a web address). The name and kind are worked out from it; a repeat is ignored.
pub fn add_shortcut(store: &Store, target: &str) -> Result<Vec<Shortcut>, CoreError> {
    let target = target.trim();
    let kind = kind_of(target).ok_or_else(|| CoreError::Io("solo se pueden fijar apps, carpetas, archivos o páginas web".into()))?;
    let mut list = shortcuts(store)?;
    if list.iter().any(|s| s.target == target) {
        return Ok(list);
    }
    if list.len() >= MAX_SHORTCUTS {
        return Err(CoreError::Io(format!("caben {MAX_SHORTCUTS} atajos; quita uno primero")));
    }
    list.push(Shortcut { id: format!("s{:x}", now_nanos()), name: name_of(target, kind), target: target.into(), kind: kind.into() });
    save(store, &list)?;
    Ok(list)
}

pub fn remove_shortcut(store: &Store, id: &str) -> Result<Vec<Shortcut>, CoreError> {
    let mut list = shortcuts(store)?;
    list.retain(|s| s.id != id);
    save(store, &list)?;
    Ok(list)
}

fn save(store: &Store, list: &[Shortcut]) -> Result<(), CoreError> {
    store.set_setting(SHORTCUTS_KEY, &serde_json::to_string(list).unwrap_or_else(|_| "[]".into()))
}

fn kind_of(target: &str) -> Option<&'static str> {
    let lower = target.to_ascii_lowercase();
    if lower.starts_with("https://") || lower.starts_with("http://") {
        return Some("web");
    }
    let path = std::path::Path::new(target);
    if !path.is_absolute() {
        return None;
    }
    if lower.ends_with(".app") || lower.ends_with(".exe") || lower.ends_with(".lnk") {
        Some("app")
    } else if path.is_dir() {
        Some("folder")
    } else {
        Some("file")
    }
}

fn name_of(target: &str, kind: &str) -> String {
    if kind == "web" {
        let host = target.split("://").nth(1).unwrap_or(target).split(['/', '?', '#']).next().unwrap_or(target);
        return host.trim_start_matches("www.").to_string();
    }
    let file = std::path::Path::new(target).file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_else(|| target.into());
    match kind {
        "app" => file.trim_end_matches(".app").trim_end_matches(".exe").trim_end_matches(".lnk").to_string(),
        _ => file,
    }
}

fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

fn now_nanos() -> u128 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn focus_starts_stops_and_reports() {
        let bus = Arc::new(EventBus::default());
        let rx = bus.subscribe();
        let focus = Focus::default();
        assert!(!focus.status().running);
        let s = focus.start(25, &bus);
        assert!(s.running && s.ends_at - s.started_at == 25 * 60);
        assert_eq!(focus.status(), s);
        assert!(matches!(rx.try_recv().unwrap(), Event::FocusChanged { running: true, .. }));
        focus.stop(&bus);
        assert!(!focus.status().running);
        assert_eq!(focus.start(0, &bus).minutes, 1, "at least one minute");
    }

    #[test]
    fn shortcuts_are_named_typed_deduplicated_and_capped() {
        let store = Store::open_in_memory().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().to_string_lossy().into_owned();
        let list = add_shortcut(&store, &folder).unwrap();
        assert_eq!(list[0].kind, "folder");
        let list = add_shortcut(&store, "https://www.github.com/crhistian-cornejo").unwrap();
        assert_eq!((list[1].name.as_str(), list[1].kind.as_str()), ("github.com", "web"));
        assert_eq!(add_shortcut(&store, &folder).unwrap().len(), 2, "no repeats");
        assert!(add_shortcut(&store, "relativo/nada").is_err());
        assert!(add_shortcut(&store, "javascript:alert(1)").is_err());
        assert_eq!(name_of("/Applications/Visual Studio Code.app", "app"), "Visual Studio Code");
        let id = list[0].id.clone();
        assert_eq!(remove_shortcut(&store, &id).unwrap().len(), 1);
        for i in 0..7 {
            add_shortcut(&store, &format!("https://ejemplo{i}.com")).unwrap();
        }
        assert!(add_shortcut(&store, "https://uno-mas.com").is_err());
    }
}
