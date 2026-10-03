//! Buddy for Windows: the floating pixel mascot on top of buddy-core (the same core the Mac app links through UniFFI).
//! The frontend only paints what these commands return; positions and data live in the core.

mod capture;
mod media;
mod notch;

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
const HISTORY: &str = "history";
const BAR: &str = "bar";
const SETTINGS: &str = "settings";
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
    /// What the next bubble says (the hello when empty).
    say: std::sync::Mutex<Option<String>>,
}

struct PetTokens {
    scale: f64,
    margin: f64,
}

fn pet_tokens() -> PetTokens {
    let json: serde_json::Value = serde_json::from_str(TOKENS).expect("design-tokens.json es JSON válido");
    PetTokens { scale: json["pet"]["scaleNormal"].as_f64().unwrap_or(2.0), margin: json["pet"]["margin"].as_f64().unwrap_or(24.0) }
}

#[tauri::command]
fn hello(state: State<'_, AppCore>) -> String {
    state.say.lock().ok().and_then(|mut s| s.take()).unwrap_or_else(|| state.core.hello())
}

#[tauri::command]
fn sprite(state: State<'_, AppCore>, id: String) -> Result<Sprite, String> {
    state.core.sprite(id).map_err(|e| e.to_string())
}

#[tauri::command]
fn show_pet_menu(window: WebviewWindow, state: State<'_, AppCore>) -> Result<(), String> {
    let app = window.app_handle();
    let e = |e: tauri::Error| e.to_string();
    let history = MenuItem::with_id(app, "history", "Historial de chats", true, Some("Ctrl+F")).map_err(e)?;
    let settings = MenuItem::with_id(app, "settings", "Ajustes…", true, Some("Ctrl+,")).map_err(e)?;
    let walk =
        CheckMenuItem::with_id(app, "wander", "Pasear por la pantalla", true, wander(&state.core), Some("Ctrl+Shift+P")).map_err(e)?;
    let quit = MenuItem::with_id(app, "quit", "Salir de Buddy", true, Some("Ctrl+Q")).map_err(e)?;
    let separator = PredefinedMenuItem::separator(app).map_err(e)?;
    let menu = Menu::with_items(app, &[&history, &settings, &walk, &separator, &quit]).map_err(e)?;
    window.popup_menu(&menu).map_err(e)
}

fn wander(core: &BuddyCore) -> bool {
    core.setting("pet.wander".into()).ok().flatten().as_deref() != Some("false")
}

#[tauri::command]
fn quit_app(app: AppHandle) {
    app.exit(0);
}

/// The next idle move, from the core's PetBrain (the same rules as on the Mac). The page tracks when Buddy was last
/// used, the pointer over it and whether it sits; the open chat also counts as in use.
#[tauri::command]
fn pet_next(
    window: WebviewWindow,
    state: State<'_, AppCore>,
    reduce_motion: bool,
    untouched_seconds: f64,
    hovering: bool,
    sitting: bool,
    sleeping: bool,
) -> Result<PetPlan, String> {
    let (window_rect, area) = pet_rects(&window).map_err(|e| e.to_string())?;
    let chat_open = state.chat_open.load(Ordering::SeqCst);
    Ok(state.brain.next(PetContext {
        x: window_rect.x,
        min_x: area.x,
        max_x: area.x + area.width - window_rect.width,
        reduce_motion,
        wander: wander(&state.core) && !chat_open,
        idle_seconds: idle_seconds(),
        untouched_seconds,
        engaged: hovering || chat_open,
        sitting,
        sleeping,
    }))
}

/// Tells the pet page Buddy was just used (the chat opened or closed, Buddy talks): a seated Buddy stands up.
fn pet_used(app: &AppHandle) {
    let _ = app.emit_to(PET, "pet-used", ());
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
        .plugin(tauri_plugin_dialog::init())
        .manage(media::MediaWatch::default())
        .invoke_handler(tauri::generate_handler![
            hello,
            sprite,
            show_pet_menu,
            quit_app,
            pet_next,
            pet_step,
            pet_settle,
            toggle_chat,
            close_chat,
            chat_resize,
            send_message,
            queued_messages,
            redirect_queued,
            queued_thumbnail,
            take_queued,
            remove_queued,
            resume_queue,
            cancel_chat,
            chats,
            search_chats,
            delete_chat,
            regenerate,
            open_history,
            history_pick,
            bar_resize,
            notch::notch_tools,
            notch::notch_copy_paths,
            notch::notch_add_files,
            notch::notch_remove_file,
            notch::notch_pick_files,
            notch::notch_save_clip,
            notch::notch_copy_clip,
            notch::notch_give_clip,
            notch::notch_take_drafts,
            notch::notch_remove_clip,
            notch::notch_widget,
            notch::notch_pick_calendar,
            notch::notch_clear_calendar,
            notch::notch_calendar,
            notch::notch_open_appointment,
            notch::notch_battery,
            media_watch,
            media_now_playing,
            media_control,
            answer_approval,
            answer_approval_always,
            always_rules,
            remove_always_rule,
            sessions,
            hooks_status,
            connect_hooks,
            focus_start,
            focus_stop,
            focus_status,
            shortcuts,
            remove_shortcut,
            pick_shortcut,
            open_shortcut,
            reveal_path,
            open_document,
            give_files,
            pick_files,
            usage,
            refresh_usage,
            messages,
            agents,
            open_url,
            briefing,
            open_settings,
            setting_flag,
            set_setting_flag,
            folders,
            pick_folder,
            set_folder_edit,
            remove_folder,
            hooks_preview,
            hooks_write,
            token_report,
            token_activity,
            open_agents_folder,
            briefing_topics,
            spotify_connect,
            router_config,
            agent_permission_catalog,
            set_agent_permissions,
            set_agent_model,
            agent_look,
            set_agent_look,
            reset_agent_look,
            agent_sprite,
            look_preview,
            look_options,
            set_router_mode,
            set_router_tier,
            router_preview,
            spotify_disconnect,
            spotify_client_id,
            connectors,
            set_connector_enabled,
            set_connector_key,
            niko_status,
            niko_accounts,
            niko_set_enabled,
            niko_set_interval,
            niko_set_senders,
            niko_set_parent,
            niko_set_telegram,
            niko_review_now,
            niko_refresh_dashboard,
            telegram_account_request,
            parley_odds_request,
            telegram_status,
            telegram_connect,
            telegram_disconnect,
            telegram_new_pairing_code,
            telegram_send,
            set_briefing_topics,
            briefing_now
        ])
        .on_menu_event(|app, event| match event.id().as_ref() {
            "quit" => app.exit(0),
            "wander" => {
                let core = &app.state::<AppCore>().core;
                let _ = core.set_setting("pet.wander".into(), (!wander(core)).to_string());
            }
            "history" => {
                let _ = open_history(app.clone());
            }
            "settings" => {
                let _ = show_settings(app);
            }
            _ => {}
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
                say: std::sync::Mutex::new(None),
            });
            app.manage(notch::Inbox::default());
            forward_events(app.handle(), &core);

            let pet = app.get_webview_window(PET).expect("la ventana «pet» está en tauri.conf.json");
            pet.set_size(LogicalSize::new(side, side))?;
            place_pet(app.handle(), &pet)?;
            pet.show()?;
            open_bubble(app.handle())?;
            open_bar(app.handle())?;
            start_sessions(app.handle());
            start_briefing(&core);
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
    let max =
        PhysicalPosition::new(min.x + area.size.width as i32 - size.width as i32, min.y + area.size.height as i32 - size.height as i32);
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
        let Some(pet) = app.get_webview_window(PET) else {
            return;
        };
        let (Ok(Some(monitor)), Ok(pos)) = (pet.current_monitor(), pet.outer_position()) else {
            return;
        };
        let Some(name) = monitor.name().cloned() else {
            return;
        };
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
    pet_used(app);
    bubble.show()
}

/// Shows `text` in Buddy's bubble (replacing the one on screen).
fn say(app: &AppHandle, text: String) {
    if let Ok(mut next) = app.state::<AppCore>().say.lock() {
        *next = Some(text);
    }
    let app2 = app.clone();
    let _ = app.run_on_main_thread(move || {
        if let Some(old) = app2.get_webview_window(BUBBLE) {
            let _ = old.destroy();
        }
        let _ = open_bubble(&app2);
    });
}

/// One line for the bubble (it does not wrap).
fn short(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_string();
    }
    format!("{}…", text.chars().take(limit - 1).collect::<String>().trim_end())
}

/// Today's runs of the «mensajitos» (8, 16, 19 h): once a little after launch, then every minute; the core skips what ran.
fn start_briefing(core: &Arc<BuddyCore>) {
    let core = core.clone();
    std::thread::Builder::new()
        .name("buddy-briefing".into())
        .spawn(move || {
            std::thread::sleep(Duration::from_secs(20));
            loop {
                core.briefing_tick();
                std::thread::sleep(Duration::from_secs(60));
            }
        })
        .expect("briefing thread");
}

#[tauri::command]
fn briefing(state: State<'_, AppCore>) -> Vec<buddy_core::BriefingItem> {
    state.core.briefing()
}

/// Centered above Buddy, or below when there is no room above.
fn place_bubble(app: &AppHandle, bubble: &WebviewWindow) -> tauri::Result<()> {
    let Some(pet) = app.get_webview_window(PET) else {
        return Ok(());
    };
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
    let Ok(prompt) = std::env::var("BUDDY_DEBUG_PROMPT") else {
        return;
    };
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

// MARK: Top bar (twin of the Mac notch)

/// The bar at the top centre of the primary screen: a thin pill at rest, sized by its page as it grows.
fn open_bar(app: &AppHandle) -> tauri::Result<()> {
    let bar = WebviewWindowBuilder::new(app, BAR, WebviewUrl::App("bar.html".into()))
        .title("Buddy")
        .inner_size(160.0, 8.0)
        .resizable(false)
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .focused(false)
        .visible(false)
        .build()?;
    place_bar(&bar, 160.0, 8.0)?;
    bar.show()
}

fn place_bar(bar: &WebviewWindow, width: f64, height: f64) -> tauri::Result<()> {
    let Some(monitor) = bar.primary_monitor()?.or(bar.current_monitor()?) else {
        return Ok(());
    };
    let scale = monitor.scale_factor();
    let area = monitor.work_area();
    let (x, y) = (area.position.x as f64 / scale, area.position.y as f64 / scale);
    let w = area.size.width as f64 / scale;
    bar.set_size(LogicalSize::new(width, height))?;
    bar.set_position(LogicalPosition::new(x + (w - width) / 2.0, y))
}

#[tauri::command]
fn bar_resize(app: AppHandle, width: f64, height: f64) -> Result<(), String> {
    let Some(bar) = app.get_webview_window(BAR) else {
        return Ok(());
    };
    place_bar(&bar, width.clamp(40.0, 600.0), height.clamp(4.0, 420.0)).map_err(|e| e.to_string())
}

/// The bar listens to the system's "now playing" only while it is open.
#[tauri::command]
fn media_watch(app: AppHandle, watch: State<'_, media::MediaWatch>, on: bool) {
    watch.set(&app, BAR, on);
}

#[tauri::command]
async fn media_now_playing() -> Option<media::NowPlaying> {
    tauri::async_runtime::spawn_blocking(media::now_playing).await.ok().flatten()
}

#[tauri::command]
async fn media_control(action: String) -> bool {
    let Some(action) = media::Action::parse(&action) else {
        return false;
    };
    tauri::async_runtime::spawn_blocking(move || media::control(action)).await.unwrap_or(false)
}

/// Claude Code and Codex hooks: the relay sits next to Buddy's exe (installed build) or in the target folder (dev).
fn start_sessions(app: &AppHandle) {
    let exe = if cfg!(windows) { "buddy-hook.exe" } else { "buddy-hook" };
    let relay = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join(exe)))
        .filter(|p| p.exists())
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    if let Err(e) = app.state::<AppCore>().core.start_sessions(relay) {
        buddy_core::log::line(format!("hooks: {e}"));
    }
}

#[tauri::command]
fn answer_approval(state: State<'_, AppCore>, request_id: String, allow: bool) {
    state.core.answer_approval(request_id, allow);
}

#[tauri::command]
fn answer_approval_always(state: State<'_, AppCore>, request_id: String) {
    state.core.answer_approval_always(request_id);
}

#[tauri::command]
fn always_rules(state: State<'_, AppCore>) -> Vec<buddy_core::sessions::always::AlwaysRule> {
    state.core.always_rules()
}

#[tauri::command]
fn remove_always_rule(state: State<'_, AppCore>, agent: String, prefix: String) {
    state.core.remove_always_rule(agent, prefix);
}

#[tauri::command]
fn sessions(state: State<'_, AppCore>) -> Vec<buddy_core::SessionInfo> {
    state.core.sessions()
}

#[tauri::command]
fn hooks_status(state: State<'_, AppCore>) -> Vec<buddy_core::HookStatusInfo> {
    state.core.hooks_status()
}

/// Shows what would change in each agent's configuration (native dialog) and writes it only after «Conectar».
#[tauri::command]
async fn connect_hooks(app: AppHandle) -> Result<(), String> {
    use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};
    let core = app.state::<AppCore>().core.clone();
    for status in core.hooks_status().into_iter().filter(|s| s.available && !s.installed) {
        let preview = core.hooks_preview(status.agent.clone(), true).map_err(|e| e.to_string())?;
        let text = format!(
            "Buddy añadirá sus avisos a {}:\n{}\n\nAntes guarda una copia del archivo, no toca tus otros hooks y puedes quitarlos cuando quieras.",
            preview.path,
            hook_events(&preview.diff).join(", ")
        );
        let dialog = app.dialog().clone();
        let title = format!("¿Conectar {} con Buddy?", status.name);
        let ok = tauri::async_runtime::spawn_blocking(move || {
            dialog
                .message(text)
                .title(title)
                .kind(MessageDialogKind::Info)
                .buttons(MessageDialogButtons::OkCancelCustom("Conectar".into(), "Cancelar".into()))
                .blocking_show()
        })
        .await
        .unwrap_or(false);
        if ok {
            core.hooks_write(status.agent.clone(), true, preview.fingerprint).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

#[tauri::command]
fn usage(state: State<'_, AppCore>) -> Vec<buddy_core::ProviderUsage> {
    state.core.usage()
}

#[tauri::command]
fn refresh_usage(state: State<'_, AppCore>) {
    state.core.refresh_usage();
}

#[tauri::command]
fn focus_start(state: State<'_, AppCore>, minutes: u32) -> buddy_core::FocusStatus {
    state.core.focus_start(minutes)
}

#[tauri::command]
fn focus_stop(state: State<'_, AppCore>) {
    state.core.focus_stop();
}

#[tauri::command]
fn focus_status(state: State<'_, AppCore>) -> buddy_core::FocusStatus {
    state.core.focus_status()
}

#[tauri::command]
fn shortcuts(state: State<'_, AppCore>) -> Result<Vec<buddy_core::Shortcut>, String> {
    state.core.shortcuts().map_err(|e| e.to_string())
}

#[tauri::command]
fn remove_shortcut(state: State<'_, AppCore>, id: String) -> Result<Vec<buddy_core::Shortcut>, String> {
    state.core.remove_shortcut(id).map_err(|e| e.to_string())
}

#[tauri::command]
fn queued_messages(state: State<'_, AppCore>, chat_id: String) -> Vec<buddy_core::QueuedMessage> {
    state.core.queued_messages(chat_id)
}

#[tauri::command]
fn remove_queued(state: State<'_, AppCore>, chat_id: String, message_id: String) {
    state.core.remove_queued(chat_id, message_id);
}

#[tauri::command]
fn queued_thumbnail(state: State<'_, AppCore>, chat_id: String, message_id: String) -> Option<String> {
    state.core.queued_thumbnail(chat_id, message_id)
}

#[tauri::command]
fn redirect_queued(state: State<'_, AppCore>, chat_id: String, message_id: String) -> Result<(), String> {
    state.core.redirect_queued(chat_id, message_id).map_err(|e| e.to_string())
}

#[tauri::command]
fn take_queued(state: State<'_, AppCore>, chat_id: String, message_id: String) -> Result<buddy_core::QueuedMessage, String> {
    state.core.take_queued(chat_id, message_id).map_err(|e| e.to_string())
}

#[tauri::command]
fn resume_queue(state: State<'_, AppCore>, chat_id: String) -> Result<(), String> {
    state.core.resume_queue(chat_id).map_err(|e| e.to_string())
}

/// The system's file picker: an app (.exe / .lnk) or a file to pin.
#[tauri::command]
async fn pick_shortcut(app: AppHandle) -> Result<Vec<buddy_core::Shortcut>, String> {
    use tauri_plugin_dialog::DialogExt;
    let dialog = app.dialog().clone();
    let picked = tauri::async_runtime::spawn_blocking(move || dialog.file().set_title("Fijar en Atajos").blocking_pick_file())
        .await
        .ok()
        .flatten();
    let core = app.state::<AppCore>().core.clone();
    match picked.and_then(|p| p.into_path().ok()) {
        Some(path) => core.add_shortcut(path.to_string_lossy().into_owned()).map_err(|e| e.to_string()),
        None => core.shortcuts().map_err(|e| e.to_string()),
    }
}

/// Opens a pinned shortcut: a web address in the browser (http/https only), anything else with its own app.
#[tauri::command]
fn open_shortcut(app: AppHandle, target: String) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    if is_web_url(&target) {
        app.opener().open_url(target, None::<&str>)
    } else {
        app.opener().open_path(target, None::<&str>)
    }
    .map_err(|e| e.to_string())
}

#[tauri::command]
fn reveal_path(app: AppHandle, path: String) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    app.opener().reveal_item_in_dir(path).map_err(|e| e.to_string())
}

/// Opens a document Buddy made or a file the user attached, in its default app: only inside Buddy's documents and
/// attachments folders (the page never opens arbitrary paths).
#[tauri::command]
fn open_document(app: AppHandle, path: String) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    let data = std::path::PathBuf::from(app.state::<AppCore>().core.data_dir());
    let file = std::fs::canonicalize(&path).map_err(|_| "ya no está".to_string())?;
    let allowed = ["documentos", "adjuntos"]
        .iter()
        .filter_map(|d| std::fs::canonicalize(data.join(d)).ok())
        .any(|root| file.starts_with(root));
    if !allowed || !file.is_file() {
        return Err("solo se abren archivos de Buddy".into());
    }
    app.opener().open_path(file.to_string_lossy(), None::<&str>).map_err(|e| e.to_string())
}

/// Files dropped on the bar: a new chat with them attached.
#[tauri::command]
fn give_files(app: AppHandle, paths: Vec<String>) -> Result<(), String> {
    notch::give_draft(&app, notch::Draft { text: None, paths })
}

/// The hook events a preview adds (the last word of each new `buddy-hook` command line), for a short summary.
fn hook_events(diff: &str) -> Vec<String> {
    let mut events: Vec<String> = diff
        .lines()
        .filter(|l| l.starts_with('+') && l.contains("buddy-hook"))
        .filter_map(|l| l.trim_end_matches([',', '"', ' ']).rsplit(' ').next().map(str::to_string))
        .collect();
    events.dedup();
    events
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
                // A new «mensajito»: Buddy says the first line; the bar keeps the list.
                if let buddy_core::Event::BriefingReady { count, headline } = &event {
                    let more = if *count > 1 { format!(" (+{})", count - 1) } else { String::new() };
                    if *count > 0 { say(&app, format!("{}{more}", short(headline, 72))); }
                }
                // The user allowed a screenshot on the card: take it off this thread, then tell the core.
                if let buddy_core::Event::ScreenshotRequest { path } = &event {
                    let (core, path) = (app.state::<AppCore>().core.clone(), path.clone());
                    std::thread::spawn(move || {
                        let ok = capture::grab()
                            .is_some_and(|(w, h, bgra)| buddy_core::images::write_bgra_png(w, h, &bgra, std::path::Path::new(&path)).is_ok());
                        core.screenshot_taken(path, ok);
                    });
                }
                if let buddy_core::Event::MediaCommand { action, uri } = &event {
                    run_media_command(&app, action.clone(), uri.clone());
                }
                let _ = app.emit("core-event", &event);
            }
        })
        .expect("events thread");
}

/// An agent's music request, already checked by the core: a button, a clean `spotify:<kind>:<id>` to play, or a
/// `spotify:search:…` to show. Windows has no "play this URI": Spotify opens it, and if it only shows the item we
/// press play once.
fn run_media_command(app: &AppHandle, action: String, uri: String) {
    use tauri_plugin_opener::OpenerExt;
    let app = app.clone();
    std::thread::spawn(move || {
        let playing = || media::now_playing().is_some_and(|n| n.status == "playing");
        match action.as_str() {
            "open" | "search" => {
                if !uri.starts_with("spotify:") || app.opener().open_url(&uri, None::<&str>).is_err() {
                    return;
                }
                if action == "open" {
                    std::thread::sleep(Duration::from_millis(2500));
                    if !playing() {
                        media::control(media::Action::PlayPause);
                    }
                }
            }
            "play" if !playing() => {
                media::control(media::Action::PlayPause);
            }
            "pause" if playing() => {
                media::control(media::Action::PlayPause);
            }
            "toggle" => {
                media::control(media::Action::PlayPause);
            }
            "next" => {
                media::control(media::Action::Next);
            }
            "previous" => {
                media::control(media::Action::Previous);
            }
            _ => {}
        }
    });
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
    pet_used(&app);
    // Buddy gets ready while the user types, so the first words come sooner.
    state.core.prewarm();
    place_chat(&app).map_err(|e| e.to_string())?;
    window.show().and_then(|_| window.set_focus()).map_err(|e| e.to_string())
}

#[tauri::command]
fn close_chat(app: AppHandle) -> Result<(), String> {
    app.state::<AppCore>().chat_open.store(false, Ordering::SeqCst);
    pet_used(&app);
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
    let (Some(pet), Some(chat)) = (app.get_webview_window(PET), app.get_webview_window(CHAT)) else {
        return Ok(());
    };
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
fn send_message(state: State<'_, AppCore>, chat_id: Option<String>, text: String, attachments: Vec<String>) -> Result<String, String> {
    state.core.send_message(chat_id, text, attachments).map_err(|e| e.to_string())
}

/// The system's file picker for attachments (several at once).
#[tauri::command]
async fn pick_files(app: AppHandle) -> Vec<String> {
    use tauri_plugin_dialog::DialogExt;
    let dialog = app.dialog().clone();
    tauri::async_runtime::spawn_blocking(move || dialog.file().set_title("Adjuntar a Buddy").blocking_pick_files())
        .await
        .ok()
        .flatten()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|p| p.into_path().ok())
        .map(|p| p.to_string_lossy().into_owned())
        .collect()
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
fn search_chats(state: State<'_, AppCore>, query: String, limit: u32) -> Result<Vec<ChatSummary>, String> {
    state.core.search_chats(query, limit).map_err(|e| e.to_string())
}

#[tauri::command]
fn delete_chat(state: State<'_, AppCore>, chat_id: String) -> Result<(), String> {
    state.core.delete_chat(chat_id).map_err(|e| e.to_string())
}

#[tauri::command]
fn regenerate(state: State<'_, AppCore>, chat_id: String) -> Result<(), String> {
    state.core.regenerate(chat_id).map_err(|e| e.to_string())
}

/// The history search, centred on the screen where Buddy is.
#[tauri::command]
fn open_history(app: AppHandle) -> Result<(), String> {
    let e = |e: tauri::Error| e.to_string();
    let window = match app.get_webview_window(HISTORY) {
        Some(w) => w,
        None => WebviewWindowBuilder::new(&app, HISTORY, WebviewUrl::App("history.html".into()))
            .title("Historial de Buddy")
            .inner_size(560.0, 420.0)
            .resizable(false)
            .decorations(false)
            .transparent(true)
            .shadow(false)
            .always_on_top(true)
            .skip_taskbar(true)
            .visible(false)
            .build()
            .map_err(e)?,
    };
    if let Some(pet) = app.get_webview_window(PET) {
        let (_, area) = pet_rects(&pet).map_err(e)?;
        let x = area.x + (area.width - 560.0) / 2.0;
        let y = area.y + (area.height - 420.0) / 2.0 - area.height * 0.08;
        window.set_position(LogicalPosition::new(x, y)).map_err(e)?;
    }
    window.show().and_then(|_| window.set_focus()).map_err(e)
}

/// A chat picked in the history: it opens in the chat next to Buddy.
#[tauri::command]
fn history_pick(app: AppHandle, chat_id: String) -> Result<(), String> {
    if let Some(history) = app.get_webview_window(HISTORY) {
        let _ = history.hide();
    }
    if !app.state::<AppCore>().chat_open.load(Ordering::SeqCst) {
        toggle_chat(app.clone())?;
    }
    app.emit_to(CHAT, "open-chat", chat_id).map_err(|e| e.to_string())
}

#[tauri::command]
fn messages(state: State<'_, AppCore>, chat_id: String) -> Result<Vec<ChatMessage>, String> {
    state.core.messages(chat_id).map_err(|e| e.to_string())
}

#[tauri::command]
fn agents(state: State<'_, AppCore>) -> Vec<Agent> {
    state.core.agents()
}

#[tauri::command]
fn agent_permission_catalog(state: State<'_, AppCore>) -> Vec<buddy_core::PermissionInfo> {
    state.core.agent_permission_catalog()
}

#[tauri::command]
fn set_agent_permissions(state: State<'_, AppCore>, agent_id: String, permissions: Vec<String>) -> Result<(), String> {
    state.core.set_agent_permissions(agent_id, permissions).map_err(|e| e.to_string())
}

#[tauri::command]
fn set_agent_model(state: State<'_, AppCore>, agent_id: String, model: String) -> Result<(), String> {
    state.core.set_agent_model(agent_id, model).map_err(|e| e.to_string())
}

/// An agent's face («cara»): colour, accessory, eyes and the accessory's colour.
#[tauri::command]
fn agent_look(state: State<'_, AppCore>, agent_id: String) -> buddy_core::AgentLook {
    state.core.agent_look(agent_id)
}

#[tauri::command]
fn set_agent_look(state: State<'_, AppCore>, agent_id: String, look: buddy_core::AgentLook) -> Result<(), String> {
    state.core.set_agent_look(agent_id, look).map_err(|e| e.to_string())
}

#[tauri::command]
fn reset_agent_look(state: State<'_, AppCore>, agent_id: String) -> Result<(), String> {
    state.core.reset_agent_look(agent_id).map_err(|e| e.to_string())
}

/// The agent wearing its face (the same sprite format as `sprite`, with its avatar square).
#[tauri::command]
fn agent_sprite(state: State<'_, AppCore>, agent_id: String) -> Result<Sprite, String> {
    state.core.agent_sprite(agent_id).map_err(|e| e.to_string())
}

#[tauri::command]
fn look_preview(state: State<'_, AppCore>, look: buddy_core::AgentLook) -> Result<Sprite, String> {
    state.core.look_preview(look).map_err(|e| e.to_string())
}

#[tauri::command]
fn look_options(state: State<'_, AppCore>) -> buddy_core::LookOptions {
    state.core.look_options()
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

// MARK: Settings (twin of the Mac's SettingsView)

/// The Settings window: a normal titled window, centred; brought to the front when it is already open.
fn show_settings(app: &AppHandle) -> tauri::Result<()> {
    let window = match app.get_webview_window(SETTINGS) {
        Some(w) => w,
        None => WebviewWindowBuilder::new(app, SETTINGS, WebviewUrl::App("settings.html".into()))
            .title("Ajustes de Buddy")
            .inner_size(680.0, 520.0)
            .resizable(false)
            .maximizable(false)
            .center()
            .visible(false)
            .build()?,
    };
    if window.is_minimized()? {
        window.unminimize()?;
    }
    window.show()?;
    window.set_focus()
}

/// Async on purpose: on Windows, building a window from a synchronous command can deadlock.
#[tauri::command]
async fn open_settings(app: AppHandle) -> Result<(), String> {
    show_settings(&app).map_err(|e| e.to_string())
}

/// The on/off settings the Settings page may read and change; nothing else goes through here.
const FLAGS: [&str; 3] = ["pet.wander", "commands.enabled", "briefing.enabled"];

fn flag_key(key: &str) -> Result<String, String> {
    FLAGS.iter().find(|k| **k == key).map(|k| k.to_string()).ok_or_else(|| format!("ajuste desconocido: {key}"))
}

/// "false" is off; anything else, missing included, is on (as on the Mac).
#[tauri::command]
fn setting_flag(state: State<'_, AppCore>, key: String) -> Result<bool, String> {
    let key = flag_key(&key)?;
    Ok(state.core.setting(key).map_err(|e| e.to_string())?.as_deref() != Some("false"))
}

#[tauri::command]
fn set_setting_flag(state: State<'_, AppCore>, key: String, on: bool) -> Result<(), String> {
    let key = flag_key(&key)?;
    state.core.set_setting(key, on.to_string()).map_err(|e| e.to_string())
}

#[tauri::command]
fn folders(state: State<'_, AppCore>) -> Result<Vec<buddy_core::AuthorizedFolder>, String> {
    state.core.folders().map_err(|e| e.to_string())
}

/// The system's folder picker; the chosen folder is authorized read-only. Cancelling returns the list as it was.
#[tauri::command]
async fn pick_folder(app: AppHandle) -> Result<Vec<buddy_core::AuthorizedFolder>, String> {
    use tauri_plugin_dialog::DialogExt;
    let dialog = app.dialog().clone();
    let picked = tauri::async_runtime::spawn_blocking(move || {
        dialog.file().set_title("Elige una carpeta que Buddy pueda leer").blocking_pick_folder()
    })
    .await
    .ok()
    .flatten();
    let core = app.state::<AppCore>().core.clone();
    match picked.and_then(|p| p.into_path().ok()) {
        Some(path) => core.add_folder(path.to_string_lossy().into_owned(), false).map_err(|e| e.to_string()),
        None => core.folders().map_err(|e| e.to_string()),
    }
}

/// «Puede editar» on a folder that is already authorized (new folders only come in through the picker).
#[tauri::command]
fn set_folder_edit(state: State<'_, AppCore>, path: String, can_edit: bool) -> Result<Vec<buddy_core::AuthorizedFolder>, String> {
    let core = &state.core;
    if !core.folders().map_err(|e| e.to_string())?.iter().any(|f| f.path == path) {
        return Err("esa carpeta no está autorizada".into());
    }
    core.add_folder(path, can_edit).map_err(|e| e.to_string())
}

#[tauri::command]
fn remove_folder(state: State<'_, AppCore>, path: String) -> Result<Vec<buddy_core::AuthorizedFolder>, String> {
    state.core.remove_folder(path).map_err(|e| e.to_string())
}

/// What connecting (or disconnecting) an agent would change, shown before anything is written.
#[tauri::command]
fn hooks_preview(state: State<'_, AppCore>, agent: String, install: bool) -> Result<buddy_core::HookPreview, String> {
    state.core.hooks_preview(agent, install).map_err(|e| e.to_string())
}

/// Writes exactly the previewed change (the fingerprint ties it to what the user saw); returns the backup's path.
#[tauri::command]
fn hooks_write(state: State<'_, AppCore>, agent: String, install: bool, fingerprint: String) -> Result<String, String> {
    state.core.hooks_write(agent, install, fingerprint).map_err(|e| e.to_string())
}

#[tauri::command]
fn token_report(state: State<'_, AppCore>, days: u32) -> Result<Vec<buddy_core::TokenReport>, String> {
    state.core.token_report(days.clamp(1, 90)).map_err(|e| e.to_string())
}

#[tauri::command]
fn token_activity(state: State<'_, AppCore>, days: u32) -> Result<Vec<buddy_core::TokenDay>, String> {
    state.core.token_activity(days).map_err(|e| e.to_string())
}

/// Opens `<data_dir>/agents` in the file manager (the path is decided here, never by the page).
#[tauri::command]
fn open_agents_folder(app: AppHandle) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    let dir = std::path::Path::new(&app.state::<AppCore>().core.data_dir()).join("agents");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    app.opener().open_path(dir.to_string_lossy(), None::<&str>).map_err(|e| e.to_string())
}

#[tauri::command]
fn router_config(state: State<'_, AppCore>) -> buddy_core::RouterConfig {
    state.core.router_config()
}

#[tauri::command]
fn set_router_mode(state: State<'_, AppCore>, mode: String) -> Result<(), String> {
    state.core.set_router_mode(mode).map_err(|e| e.to_string())
}

#[tauri::command]
fn set_router_tier(state: State<'_, AppCore>, tier: String, model: String, effort: String) -> Result<(), String> {
    let tier = buddy_core::Tier::parse(&tier).ok_or("nivel desconocido")?;
    state.core.set_router_tier(tier, model, effort).map_err(|e| e.to_string())
}

#[tauri::command]
fn router_preview(state: State<'_, AppCore>, text: String) -> String {
    state.core.router_preview(text.chars().take(2000).collect())
}

/// Settings › Conectores (MCP): the built-in remote servers with their switch and whether a key is saved.
#[tauri::command]
fn connectors(state: State<'_, AppCore>) -> Vec<buddy_core::connectors::ConnectorInfo> {
    state.core.connectors()
}

#[tauri::command]
fn set_connector_enabled(state: State<'_, AppCore>, id: String, on: bool) -> Result<(), String> {
    state.core.set_connector_enabled(id, on).map_err(|e| e.to_string())
}

/// Keeps the optional key only in Credential Manager (empty removes it).
#[tauri::command]
fn set_connector_key(state: State<'_, AppCore>, id: String, key: String) -> Result<(), String> {
    state.core.set_connector_key(id, key).map_err(|e| e.to_string())
}

// MARK: Niko · finanzas (Settings; the core does all of it)

#[tauri::command]
fn niko_status(state: State<'_, AppCore>) -> buddy_core::niko::NikoStatus {
    state.core.niko_status()
}

/// Gmail, Notion and Drive as connected in claude.ai (asks Claude Code: a few seconds, off the UI thread).
#[tauri::command]
async fn niko_accounts(app: AppHandle) -> Vec<buddy_core::accounts::AccountStatus> {
    let core = app.state::<AppCore>().core.clone();
    tauri::async_runtime::spawn_blocking(move || core.niko_accounts()).await.unwrap_or_default()
}

#[tauri::command]
fn niko_set_enabled(state: State<'_, AppCore>, on: bool) {
    state.core.niko_set_enabled(on);
}

#[tauri::command]
fn niko_set_interval(state: State<'_, AppCore>, minutes: u32) -> Result<(), String> {
    state.core.niko_set_interval(minutes).map_err(|e| e.to_string())
}

#[tauri::command]
fn niko_set_senders(state: State<'_, AppCore>, senders: String) -> String {
    state.core.niko_set_senders(senders.chars().take(4000).collect())
}

#[tauri::command]
fn niko_set_parent(state: State<'_, AppCore>, page: String) -> Result<(), String> {
    state.core.niko_set_parent(page.chars().take(500).collect()).map_err(|e| e.to_string())
}

#[tauri::command]
fn niko_set_telegram(state: State<'_, AppCore>, on: bool) {
    state.core.niko_set_telegram(on);
}

#[tauri::command]
fn niko_review_now(state: State<'_, AppCore>) {
    state.core.niko_review_now();
}

#[tauri::command]
fn niko_refresh_dashboard(state: State<'_, AppCore>) {
    state.core.niko_refresh_dashboard();
}

/// Checks the pair with Spotify, keeps the Client ID in settings and the Client Secret in Credential Manager.
#[tauri::command]
async fn spotify_connect(app: AppHandle, client_id: String, client_secret: String) -> Result<(), String> {
    let core = app.state::<AppCore>().core.clone();
    tauri::async_runtime::spawn_blocking(move || core.spotify_connect(client_id, client_secret))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn spotify_disconnect(state: State<'_, AppCore>) -> Result<(), String> {
    state.core.spotify_disconnect().map_err(|e| e.to_string())
}

/// The Client ID when connected (empty otherwise); the secret never comes back.
#[tauri::command]
fn spotify_client_id(state: State<'_, AppCore>) -> String {
    state.core.spotify_client_id()
}

/// Telegram for Settings: connected bot, paired chat and the code to send as `/start <code>`.
#[tauri::command]
fn telegram_status(state: State<'_, AppCore>) -> buddy_core::TelegramStatus {
    state.core.telegram_status()
}

/// Checks the bot token with Telegram, then keeps it only in Credential Manager (off the UI thread).
#[tauri::command]
async fn telegram_connect(app: AppHandle, token: String) -> Result<(), String> {
    let core = app.state::<AppCore>().core.clone();
    tauri::async_runtime::spawn_blocking(move || core.telegram_connect(token))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn telegram_disconnect(state: State<'_, AppCore>) -> Result<(), String> {
    state.core.telegram_disconnect().map_err(|e| e.to_string())
}

#[tauri::command]
fn telegram_new_pairing_code(state: State<'_, AppCore>) -> Result<String, String> {
    state.core.telegram_new_pairing_code().map_err(|e| e.to_string())
}

/// Sends text to the paired chat (the user's click: «Enviar prueba», later PARLEY's picks).
#[tauri::command]
async fn telegram_send(app: AppHandle, text: String) -> Result<(), String> {
    let core = app.state::<AppCore>().core.clone();
    tauri::async_runtime::spawn_blocking(move || core.telegram_send(text))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn briefing_topics(state: State<'_, AppCore>) -> String {
    state.core.briefing_topics()
}

/// Saves the topics (empty goes back to the defaults) and returns what is now in use.
#[tauri::command]
fn set_briefing_topics(state: State<'_, AppCore>, topics: String) -> Result<String, String> {
    state.core.set_briefing_topics(topics).map_err(|e| e.to_string())?;
    Ok(state.core.briefing_topics())
}

#[tauri::command]
fn briefing_now(state: State<'_, AppCore>) {
    state.core.briefing_now();
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
    fn the_connect_summary_lists_the_events() {
        let diff = "+   \"command\": \"\\\"/x/buddy-hook\\\" Stop\",\n+   \"command\": \"\\\"/x/buddy-hook\\\" PermissionRequest\",\n  \"model\": \"x\"";
        assert_eq!(hook_events(diff), ["Stop", "PermissionRequest"]);
    }

    #[test]
    fn only_web_addresses_open() {
        assert!(is_web_url("https://senamhi.gob.pe"));
        assert!(!is_web_url("file:///C:/Windows"));
        assert!(!is_web_url("javascript:alert(1)"));
        assert!(!is_web_url("https://x.com/a b"));
    }

    #[test]
    fn only_known_flags_are_reachable() {
        assert_eq!(flag_key("pet.wander").as_deref(), Ok("pet.wander"));
        assert!(flag_key("pet.origin.x").is_err());
        assert!(flag_key("").is_err());
    }

    #[test]
    fn tokens_have_a_pet_scale() {
        assert_eq!(pet_tokens().scale, 2.0);
    }
}

/// Personal-account operations run away from the window thread.
#[tauri::command]
async fn telegram_account_request(app: AppHandle, action: String, value: String) -> Result<String, String> {
    let core = app.state::<AppCore>().core.clone();
    tauri::async_runtime::spawn_blocking(move || core.telegram_account_request(action, value))
        .await.map_err(|_| "La conexión con Telegram se interrumpió.".to_string())?
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn parley_odds_request(app: AppHandle, action: String, value: String) -> Result<String, String> {
    let core = app.state::<AppCore>().core.clone();
    tauri::async_runtime::spawn_blocking(move || core.parley_odds_request(action, value))
        .await.map_err(|_| "La configuración de cuotas se interrumpió.".to_string())?
        .map_err(|e| e.to_string())
}
