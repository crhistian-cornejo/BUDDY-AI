//! Buddy for Windows: the floating pixel mascot on top of buddy-core (the same core the Mac app links through UniFFI).
//! The frontend only paints what these commands return; positions and data live in the core.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use buddy_core::{Agent, BuddyCore, ChatMessage, ChatSummary, PetBrain, PetContext, PetPlan, PetRect, Sprite, clamp_to_area};
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::{
    AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, Monitor, PhysicalPosition, PhysicalSize, State,
    WebviewUrl, WebviewWindow, WebviewWindowBuilder, WindowEvent,
};

/// assets/design-tokens.json, shared with the Mac app.
const TOKENS: &str = include_str!("../../../../assets/design-tokens.json");

const PET: &str = "pet";
const BUBBLE: &str = "bubble";
const CHAT: &str = "chat";
const CHAT_WIDTH: f64 = 400.0;
const BUBBLE_SIZE: (f64, f64) = (340.0, 52.0);
const SPRITE: &str = "buddy-base";

struct AppCore {
    core: Arc<BuddyCore>,
    brain: PetBrain,
    /// Bumped on every move: only the last move of a drag is saved.
    moves: AtomicU64,
    /// While the chat is open Buddy stays put (the chat hangs from it).
    chat_open: AtomicBool,
    /// The chat window's content height (it grows with the answer).
    chat_height: std::sync::Mutex<f64>,
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
fn show_pet_menu(window: WebviewWindow, state: State<'_, AppCore>) -> Result<(), String> {
    let app = window.app_handle();
    let e = |e: tauri::Error| e.to_string();
    let walk = CheckMenuItem::with_id(app, "wander", "Pasear por la pantalla", true, wander(&state.core), None::<&str>)
        .map_err(e)?;
    let quit = MenuItem::with_id(app, "quit", "Salir de Buddy", true, None::<&str>).map_err(e)?;
    let separator = PredefinedMenuItem::separator(app).map_err(e)?;
    let menu = Menu::with_items(app, &[&walk, &separator, &quit]).map_err(e)?;
    window.popup_menu(&menu).map_err(e)
}

fn wander(core: &BuddyCore) -> bool {
    core.setting("pet.wander".into()).ok().flatten().as_deref() != Some("false")
}

/// The next idle move, from the core's PetBrain (the same rules as on the Mac).
#[tauri::command]
fn pet_next(window: WebviewWindow, state: State<'_, AppCore>, reduce_motion: bool) -> Result<PetPlan, String> {
    let (window_rect, area) = pet_rects(&window).map_err(|e| e.to_string())?;
    Ok(state.brain.next(PetContext {
        x: window_rect.x,
        min_x: area.x,
        max_x: area.x + area.width - window_rect.width,
        reduce_motion,
        wander: wander(&state.core) && !state.chat_open.load(Ordering::SeqCst),
        idle_seconds: idle_seconds(),
    }))
}

/// Moves Buddy `dx` logical pixels sideways, never past the work area.
#[tauri::command]
fn pet_step(window: WebviewWindow, dx: f64) -> Result<(), String> {
    let (mut rect, area) = pet_rects(&window).map_err(|e| e.to_string())?;
    rect.x += dx;
    let r = clamp_to_area(rect, area);
    window.set_position(LogicalPosition::new(r.x, r.y)).map_err(|e| e.to_string())
}

/// After a drag: back fully inside the work area (corners included).
#[tauri::command]
fn pet_settle(window: WebviewWindow) -> Result<(), String> {
    let (rect, area) = pet_rects(&window).map_err(|e| e.to_string())?;
    let r = clamp_to_area(rect, area);
    if r != rect {
        window.set_position(LogicalPosition::new(r.x, r.y)).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// The pet window and its monitor's work area, in logical pixels.
fn pet_rects(window: &WebviewWindow) -> tauri::Result<(PetRect, PetRect)> {
    let scale = window.scale_factor()?;
    let pos = window.outer_position()?.to_logical::<f64>(scale);
    let size = window.outer_size()?.to_logical::<f64>(scale);
    let rect = PetRect { x: pos.x, y: pos.y, width: size.width, height: size.height };
    let area = match window.current_monitor()?.or(window.primary_monitor()?) {
        Some(m) => {
            let a = m.work_area();
            let p = a.position.to_logical::<f64>(m.scale_factor());
            let s = a.size.to_logical::<f64>(m.scale_factor());
            PetRect { x: p.x, y: p.y, width: s.width, height: s.height }
        }
        None => rect,
    };
    Ok((rect, area))
}

#[cfg(windows)]
fn idle_seconds() -> f64 {
    use windows::Win32::System::SystemInformation::GetTickCount;
    use windows::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};
    let mut info = LASTINPUTINFO { cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32, dwTime: 0 };
    if unsafe { GetLastInputInfo(&mut info) }.as_bool() {
        unsafe { GetTickCount() }.wrapping_sub(info.dwTime) as f64 / 1000.0
    } else {
        0.0
    }
}

#[cfg(not(windows))]
fn idle_seconds() -> f64 {
    0.0
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|_, _, _| {}))
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            hello,
            sprite,
            show_pet_menu,
            pet_next,
            pet_step,
            pet_settle,
            toggle_chat,
            close_chat,
            chat_resize,
            send_message,
            cancel_chat,
            chats,
            messages,
            agents,
            open_url
        ])
        .on_menu_event(|app, event| {
            if event.id() == "quit" {
                app.exit(0);
            } else if event.id() == "wander" {
                let core = &app.state::<AppCore>().core;
                let _ = core.set_setting("pet.wander".into(), (!wander(core)).to_string());
            }
        })
        .setup(|app| {
            // An empty folder lets the core use %LOCALAPPDATA%\Buddy.
            let core = Arc::new(BuddyCore::open("")?);
            let side = core.sprite(SPRITE.into())?.size as f64 * pet_tokens().scale;
            app.manage(AppCore {
                core: core.clone(),
                brain: PetBrain::default(),
                moves: AtomicU64::new(0),
                chat_open: AtomicBool::new(false),
                chat_height: std::sync::Mutex::new(60.0),
            });
            forward_events(app.handle(), &core);

            let pet = app.get_webview_window(PET).expect("la ventana «pet» está en tauri.conf.json");
            pet.set_size(LogicalSize::new(side, side))?;
            place_pet(app.handle(), &pet)?;
            pet.show()?;
            open_bubble(app.handle())?;
            #[cfg(debug_assertions)]
            debug_prompt(app.handle());
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

/// Keeps the bubble and the chat next to Buddy and saves the place once the drag settles.
fn pet_moved(app: &AppHandle) {
    if let Some(bubble) = app.get_webview_window(BUBBLE) {
        let _ = place_bubble(app, &bubble);
    }
    if app.state::<AppCore>().chat_open.load(Ordering::SeqCst) {
        let _ = place_chat(app);
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

/// BUDDY_DEBUG_PROMPT="…" (debug builds): opens the chat and sends it, to try the whole flow from a terminal.
#[cfg(debug_assertions)]
fn debug_prompt(app: &AppHandle) {
    let Ok(prompt) = std::env::var("BUDDY_DEBUG_PROMPT") else { return };
    let app = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(1500));
        let _ = toggle_chat(app.clone());
        std::thread::sleep(Duration::from_millis(1500));
        if let Some(chat) = app.get_webview_window(CHAT) {
            let js = format!("document.getElementById('input').value = {}; document.getElementById('send').click();",
                serde_json::to_string(&prompt).unwrap_or_default());
            let _ = chat.eval(&js);
        }
    });
}

// MARK: Chat

/// Every core event goes to every window ("core-event"); each page takes what it draws.
fn forward_events(app: &AppHandle, core: &BuddyCore) {
    let rx = core.events();
    let app = app.clone();
    std::thread::Builder::new()
        .name("buddy-events".into())
        .spawn(move || {
            while let Ok(event) = rx.recv() {
                let _ = app.emit("core-event", &event);
            }
        })
        .expect("events thread");
}

#[tauri::command]
fn toggle_chat(app: AppHandle) -> Result<(), String> {
    let state = app.state::<AppCore>();
    if state.chat_open.load(Ordering::SeqCst) {
        return close_chat(app.clone());
    }
    let window = match app.get_webview_window(CHAT) {
        Some(w) => w,
        None => WebviewWindowBuilder::new(&app, CHAT, WebviewUrl::App("chat.html".into()))
            .title("Buddy")
            .inner_size(CHAT_WIDTH, *state.chat_height.lock().unwrap())
            .resizable(false)
            .decorations(false)
            .transparent(true)
            .shadow(false)
            .always_on_top(true)
            .skip_taskbar(true)
            .visible(false)
            .build()
            .map_err(|e| e.to_string())?,
    };
    state.chat_open.store(true, Ordering::SeqCst);
    place_chat(&app).map_err(|e| e.to_string())?;
    window.show().and_then(|_| window.set_focus()).map_err(|e| e.to_string())
}

#[tauri::command]
fn close_chat(app: AppHandle) -> Result<(), String> {
    app.state::<AppCore>().chat_open.store(false, Ordering::SeqCst);
    if let Some(window) = app.get_webview_window(CHAT) {
        window.hide().map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// The page reports its content height; the window follows it (bottom-anchored next to Buddy).
#[tauri::command]
fn chat_resize(app: AppHandle, height: f64) -> Result<(), String> {
    *app.state::<AppCore>().chat_height.lock().unwrap() = height.clamp(56.0, 640.0);
    place_chat(&app).map_err(|e| e.to_string())
}

/// On the side of Buddy with more room, its bottom level with Buddy's, never past the work area.
fn place_chat(app: &AppHandle) -> tauri::Result<()> {
    let (Some(pet), Some(chat)) = (app.get_webview_window(PET), app.get_webview_window(CHAT)) else { return Ok(()) };
    let (pet_rect, area) = pet_rects(&pet)?;
    let height = (*app.state::<AppCore>().chat_height.lock().unwrap()).min(area.height - 16.0);
    let left = pet_rect.x + pet_rect.width / 2.0 > area.x + area.width / 2.0;
    let x = if left { pet_rect.x - CHAT_WIDTH - 6.0 } else { pet_rect.x + pet_rect.width + 6.0 };
    let y = pet_rect.y + pet_rect.height - height;
    let r = clamp_to_area(PetRect { x, y, width: CHAT_WIDTH, height }, area);
    chat.set_size(LogicalSize::new(r.width, r.height))?;
    chat.set_position(LogicalPosition::new(r.x, r.y))
}

#[tauri::command]
fn send_message(state: State<'_, AppCore>, chat_id: Option<String>, text: String) -> Result<String, String> {
    state.core.send_message(chat_id, text).map_err(|e| e.to_string())
}

#[tauri::command]
fn cancel_chat(state: State<'_, AppCore>, chat_id: String) {
    state.core.cancel_chat(chat_id);
}

#[tauri::command]
fn chats(state: State<'_, AppCore>, limit: u32) -> Result<Vec<ChatSummary>, String> {
    state.core.chats(limit).map_err(|e| e.to_string())
}

#[tauri::command]
fn messages(state: State<'_, AppCore>, chat_id: String) -> Result<Vec<ChatMessage>, String> {
    state.core.messages(chat_id).map_err(|e| e.to_string())
}

#[tauri::command]
fn agents(state: State<'_, AppCore>) -> Vec<Agent> {
    state.core.agents()
}

/// Opens a web page in the browser: http and https only, nothing else ever leaves through here.
#[tauri::command]
fn open_url(app: AppHandle, url: String) -> Result<(), String> {
    if !is_web_url(&url) {
        return Err("solo se abren direcciones web".into());
    }
    use tauri_plugin_opener::OpenerExt;
    app.opener().open_url(url, None::<&str>).map_err(|e| e.to_string())
}

fn is_web_url(url: &str) -> bool {
    let lower = url.trim().to_ascii_lowercase();
    (lower.starts_with("https://") || lower.starts_with("http://")) && !lower.contains(char::is_whitespace)
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
    fn only_web_addresses_open() {
        assert!(is_web_url("https://senamhi.gob.pe"));
        assert!(!is_web_url("file:///C:/Windows"));
        assert!(!is_web_url("javascript:alert(1)"));
        assert!(!is_web_url("https://x.com/a b"));
    }

    #[test]
    fn tokens_have_a_pet_scale() {
        assert_eq!(pet_tokens().scale, 2.0);
    }
}
