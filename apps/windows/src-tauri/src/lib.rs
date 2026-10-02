//! Buddy for Windows: the floating pixel mascot on top of buddy-core (the same core the Mac app links through UniFFI).
//! The frontend only paints what these commands return; positions and data live in the core.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use buddy_core::{BuddyCore, Sprite};
use tauri::menu::{Menu, MenuItem};
use tauri::{
    AppHandle, LogicalSize, Manager, Monitor, PhysicalPosition, PhysicalSize, State, WebviewUrl, WebviewWindow,
    WebviewWindowBuilder, WindowEvent,
};

/// assets/design-tokens.json, shared with the Mac app.
const TOKENS: &str = include_str!("../../../../assets/design-tokens.json");

const PET: &str = "pet";
const BUBBLE: &str = "bubble";
const BUBBLE_SIZE: (f64, f64) = (340.0, 52.0);
const SPRITE: &str = "buddy-base";

struct AppCore {
    core: Arc<BuddyCore>,
    /// Bumped on every move: only the last move of a drag is saved.
    moves: AtomicU64,
}

struct PetTokens {
    scale: f64,
    margin: f64,
}

fn pet_tokens() -> PetTokens {
    let json: serde_json::Value = serde_json::from_str(TOKENS).expect("design-tokens.json es JSON válido");
    PetTokens {
        scale: json["pet"]["scaleNormal"].as_f64().unwrap_or(2.0),
        margin: json["pet"]["margin"].as_f64().unwrap_or(24.0),
    }
}

#[tauri::command]
fn hello(state: State<'_, AppCore>) -> String {
    state.core.hello()
}

#[tauri::command]
fn sprite(state: State<'_, AppCore>, id: String) -> Result<Sprite, String> {
    state.core.sprite(id).map_err(|e| e.to_string())
}

#[tauri::command]
fn show_pet_menu(window: WebviewWindow) -> Result<(), String> {
    let app = window.app_handle();
    let quit = MenuItem::with_id(app, "quit", "Salir de Buddy", true, None::<&str>).map_err(|e| e.to_string())?;
    let menu = Menu::with_items(app, &[&quit]).map_err(|e| e.to_string())?;
    window.popup_menu(&menu).map_err(|e| e.to_string())
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|_, _, _| {}))
        .invoke_handler(tauri::generate_handler![hello, sprite, show_pet_menu])
        .on_menu_event(|app, event| {
            if event.id() == "quit" {
                app.exit(0);
            }
        })
        .setup(|app| {
            // An empty folder lets the core use %LOCALAPPDATA%\Buddy.
            let core = Arc::new(BuddyCore::open("")?);
            let side = core.sprite(SPRITE.into())?.size as f64 * pet_tokens().scale;
            app.manage(AppCore { core, moves: AtomicU64::new(0) });

            let pet = app.get_webview_window(PET).expect("la ventana «pet» está en tauri.conf.json");
            pet.set_size(LogicalSize::new(side, side))?;
            place_pet(app.handle(), &pet)?;
            pet.show()?;
            open_bubble(app.handle())?;
            Ok(())
        })
        .on_window_event(|window, event| {
            if window.label() == PET {
                if let WindowEvent::Moved(_) = event {
                    pet_moved(window.app_handle());
                }
            }
        })
        .run(tauri::generate_context!())
        .expect("error al iniciar Buddy");
}

// MARK: Position

/// The saved place on the saved monitor, kept inside its work area; otherwise the bottom-right corner of the primary.
fn place_pet(app: &AppHandle, pet: &WebviewWindow) -> tauri::Result<()> {
    let core = &app.state::<AppCore>().core;
    let size = pet.outer_size()?;
    let saved_monitor = core.setting("pet.monitor".into()).ok().flatten();
    if let Some(name) = saved_monitor {
        let monitor = pet.available_monitors()?.into_iter().find(|m| m.name().is_some_and(|n| *n == name));
        let origin = core.setting(format!("pet.origin.{name}")).ok().flatten().and_then(|s| parse_point(&s));
        if let (Some(monitor), Some((x, y))) = (monitor, origin) {
            let (min, max) = work_area(&monitor, size);
            return pet.set_position(PhysicalPosition::new(x.clamp(min.x, max.x), y.clamp(min.y, max.y)));
        }
    }
    if let Some(monitor) = pet.primary_monitor()?.or(pet.current_monitor()?) {
        let margin = (pet_tokens().margin * monitor.scale_factor()) as i32;
        let (_, max) = work_area(&monitor, size);
        pet.set_position(PhysicalPosition::new(max.x - margin, max.y - margin))?;
    }
    Ok(())
}

/// The range a window of `size` may take inside the monitor's work area (physical pixels).
fn work_area(monitor: &Monitor, size: PhysicalSize<u32>) -> (PhysicalPosition<i32>, PhysicalPosition<i32>) {
    let area = monitor.work_area();
    let min = area.position;
    let max = PhysicalPosition::new(
        min.x + area.size.width as i32 - size.width as i32,
        min.y + area.size.height as i32 - size.height as i32,
    );
    (min, PhysicalPosition::new(max.x.max(min.x), max.y.max(min.y)))
}

fn parse_point(s: &str) -> Option<(i32, i32)> {
    let (x, y) = s.split_once(',')?;
    Some((x.trim().parse().ok()?, y.trim().parse().ok()?))
}

/// Keeps the bubble next to Buddy and saves the place once the drag settles.
fn pet_moved(app: &AppHandle) {
    if let Some(bubble) = app.get_webview_window(BUBBLE) {
        let _ = place_bubble(app, &bubble);
    }
    let generation = app.state::<AppCore>().moves.fetch_add(1, Ordering::SeqCst) + 1;
    let app = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(300));
        let state = app.state::<AppCore>();
        if state.moves.load(Ordering::SeqCst) != generation {
            return;
        }
        let Some(pet) = app.get_webview_window(PET) else { return };
        let (Ok(Some(monitor)), Ok(pos)) = (pet.current_monitor(), pet.outer_position()) else { return };
        let Some(name) = monitor.name().cloned() else { return };
        let _ = state.core.set_setting("pet.monitor".into(), name.clone());
        let _ = state.core.set_setting(format!("pet.origin.{name}"), format!("{},{}", pos.x, pos.y));
    });
}

// MARK: Bubble

/// The hello bubble: its own window that never takes clicks, closed by its page after a few seconds.
fn open_bubble(app: &AppHandle) -> tauri::Result<()> {
    let bubble = WebviewWindowBuilder::new(app, BUBBLE, WebviewUrl::App("bubble.html".into()))
        .title("Buddy")
        .inner_size(BUBBLE_SIZE.0, BUBBLE_SIZE.1)
        .resizable(false)
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .focused(false)
        .visible(false)
        .build()?;
    bubble.set_ignore_cursor_events(true)?;
    place_bubble(app, &bubble)?;
    bubble.show()
}

/// Centered above Buddy, or below when there is no room above.
fn place_bubble(app: &AppHandle, bubble: &WebviewWindow) -> tauri::Result<()> {
    let Some(pet) = app.get_webview_window(PET) else { return Ok(()) };
    let (pos, size, b) = (pet.outer_position()?, pet.outer_size()?, bubble.outer_size()?);
    let mut x = pos.x + size.width as i32 / 2 - b.width as i32 / 2;
    let mut y = pos.y - b.height as i32;
    if let Some(monitor) = pet.current_monitor()? {
        let (min, max) = work_area(&monitor, b);
        if y < min.y {
            y = pos.y + size.height as i32;
        }
        x = x.clamp(min.x, max.x);
    }
    bubble.set_position(PhysicalPosition::new(x, y))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_saved_points() {
        assert_eq!(parse_point("10,-20"), Some((10, -20)));
        assert_eq!(parse_point(" 3 , 4 "), Some((3, 4)));
        assert_eq!(parse_point("x,1"), None);
        assert_eq!(parse_point("12"), None);
    }

    #[test]
    fn tokens_have_a_pet_scale() {
        assert_eq!(pet_tokens().scale, 2.0);
    }
}
