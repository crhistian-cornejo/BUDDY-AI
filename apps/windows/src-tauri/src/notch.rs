//! Native commands for the user-chosen tools. Clipboard access is only by a button press.
use tauri::{AppHandle, Emitter, Manager, State};
use buddy_core::{NotchTools, Appointment};
use crate::{AppCore, CHAT};

#[derive(serde::Serialize)]
pub(crate) struct Draft { pub text: Option<String>, pub paths: Vec<String> }
#[derive(Default)]
pub(crate) struct Inbox(std::sync::Mutex<Vec<Draft>>);

/// A newly created chat can load after the button press. Keep deliveries until its listener is ready.
pub(crate) fn give_draft(app: &AppHandle, draft: Draft) -> Result<(), String> {
    if !app.state::<AppCore>().chat_open.load(std::sync::atomic::Ordering::SeqCst) {
        crate::toggle_chat(app.clone())?;
    }
    app.state::<Inbox>().0.lock().unwrap().push(draft);
    app.emit_to(CHAT, "notch-draft", ()).map_err(|e| e.to_string())
}
#[tauri::command]
pub(crate) fn notch_take_drafts(state: State<'_, Inbox>) -> Vec<Draft> {
    std::mem::take(&mut *state.0.lock().unwrap())
}

#[tauri::command]
pub fn notch_tools(state: State<'_, AppCore>) -> Result<NotchTools, String> { state.core.notch_tools().map_err(|e| e.to_string()) }
#[tauri::command]
pub fn notch_add_files(state: State<'_, AppCore>, paths: Vec<String>) -> Result<NotchTools, String> {
    state.core.notch_add_files(paths).map_err(|e| e.to_string())
}
#[tauri::command]
pub fn notch_remove_file(state: State<'_, AppCore>, path: String) -> Result<NotchTools, String> {
    state.core.notch_remove_file(path).map_err(|e| e.to_string())
}
#[tauri::command]
pub async fn notch_pick_files(app: AppHandle) -> Result<NotchTools, String> {
    use tauri_plugin_dialog::DialogExt;
    let dialog = app.dialog().clone();
    let picked = tauri::async_runtime::spawn_blocking(move || dialog.file().set_title("Añadir a la bandeja").blocking_pick_files())
        .await.map_err(|e| e.to_string())?;
    let paths = picked.unwrap_or_default().into_iter().filter_map(|p| p.into_path().ok())
        .map(|p| p.to_string_lossy().into_owned()).collect();
    app.state::<AppCore>().core.notch_add_files(paths).map_err(|e| e.to_string())
}
#[tauri::command]
pub fn notch_copy_paths(state: State<'_, AppCore>) -> Result<(), String> {
    let paths = state.core.notch_tools().map_err(|e| e.to_string())?.files.into_iter()
        .filter(|f| f.available).map(|f| f.path).collect::<Vec<_>>().join("\n");
    arboard::Clipboard::new().and_then(|mut c| c.set_text(paths)).map_err(|_| "No se pudieron copiar las rutas.".into())
}
#[tauri::command]
pub fn notch_save_clip(state: State<'_, AppCore>) -> Result<NotchTools, String> {
    let text = arboard::Clipboard::new().and_then(|mut c| c.get_text()).map_err(|_| "El portapapeles no contiene texto.".to_string())?;
    state.core.notch_save_clip(text).map_err(|e| e.to_string())
}
fn clip(state: &AppCore, id: &str) -> Result<String, String> {
    state.core.notch_tools().map_err(|e| e.to_string())?.clips.into_iter().find(|c| c.id == id).map(|c| c.text)
        .ok_or_else(|| "El texto ya no está guardado.".into())
}
#[tauri::command]
pub fn notch_copy_clip(state: State<'_, AppCore>, id: String) -> Result<(), String> {
    let text = clip(&state, &id)?;
    arboard::Clipboard::new().and_then(|mut c| c.set_text(text)).map_err(|_| "No se pudo copiar el texto.".into())
}
#[tauri::command]
pub fn notch_give_clip(app: AppHandle, id: String) -> Result<(), String> {
    let text = clip(&app.state::<AppCore>(), &id)?;
    give_draft(&app, Draft { text: Some(text), paths: vec![] })
}
#[tauri::command]
pub fn notch_remove_clip(state: State<'_, AppCore>, id: String) -> Result<NotchTools, String> {
    state.core.notch_remove_clip(id).map_err(|e| e.to_string())
}
#[tauri::command]
pub fn notch_widget(state: State<'_, AppCore>, widget: String, enabled: bool) -> Result<NotchTools, String> {
    state.core.notch_widget(widget, enabled).map_err(|e| e.to_string())
}
#[tauri::command]
pub async fn notch_pick_calendar(app: AppHandle) -> Result<NotchTools, String> {
    use tauri_plugin_dialog::DialogExt;
    let dialog = app.dialog().clone();
    let picked = tauri::async_runtime::spawn_blocking(move || dialog.file().set_title("Elegir agenda local (.ics)").add_filter("Agenda", &["ics"]).blocking_pick_file())
        .await.map_err(|e| e.to_string())?;
    let core = app.state::<AppCore>().core.clone();
    match picked.and_then(|p| p.into_path().ok()) {
        Some(path) => tauri::async_runtime::spawn_blocking(move || core.notch_set_calendar(Some(path.to_string_lossy().into_owned())).map_err(|e| e.to_string())).await.map_err(|e| e.to_string())?,
        None => core.notch_tools().map_err(|e| e.to_string()),
    }
}
#[tauri::command]
pub fn notch_clear_calendar(state: State<'_, AppCore>) -> Result<NotchTools, String> {
    state.core.notch_set_calendar(None).map_err(|e| e.to_string())
}
#[tauri::command]
pub async fn notch_calendar(app: AppHandle) -> Result<Option<Appointment>, String> {
    let core = app.state::<AppCore>().core.clone();
    tauri::async_runtime::spawn_blocking(move || core.notch_calendar().map_err(|e| e.to_string())).await.map_err(|e| e.to_string())?
}
#[tauri::command]
pub async fn notch_open_appointment(app: AppHandle) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    let core = app.state::<AppCore>().core.clone();
    let (url, path) = tauri::async_runtime::spawn_blocking(move || {
        let event = core.notch_calendar().map_err(|e| e.to_string())?.ok_or("No hay una próxima cita en la agenda.")?;
        if let Some(url) = event.url { Ok((Some(url), None)) }
        else { core.notch_appointment_file().map(|path| (None, Some(path))).map_err(|e| e.to_string()) }
    }).await.map_err(|e| e.to_string())??;
    if let Some(url) = url { app.opener().open_url(url, None::<&str>).map_err(|e| e.to_string()) }
    else { app.opener().open_path(path.unwrap_or_default(), None::<&str>).map_err(|e| e.to_string()) }
}

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Battery { pub percent: Option<u8>, pub charging: bool, pub plugged_in: bool }

#[tauri::command]
pub fn notch_battery() -> Option<Battery> {
    #[cfg(windows)] {
        use windows::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};
        let mut status = SYSTEM_POWER_STATUS::default();
        if unsafe { GetSystemPowerStatus(&mut status) }.is_err() || status.BatteryFlag == 128 { return None; }
        return Some(Battery { percent: (status.BatteryLifePercent <= 100).then_some(status.BatteryLifePercent),
            charging: status.BatteryFlag != 255 && status.BatteryFlag & 8 != 0, plugged_in: status.ACLineStatus == 1 });
    }
    #[cfg(not(windows))] { None }
}
