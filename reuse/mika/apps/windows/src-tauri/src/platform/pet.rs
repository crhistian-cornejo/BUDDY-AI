//! The floating Mika: in mascot mode this desktop window owns the welcome, notices and chat while the notch stays
//! hidden. It appears where the user left it and can be dragged anywhere. Hovering opens the agents; a click opens
//! a small chat right there: the window grows around Mika without moving her. Windows only (macOS keeps a circle
//! beside the notch). Her place lives in %APPDATA%\MIKA\pet.json, apart from the settings the webviews rewrite whole.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, WebviewUrl, WebviewWindowBuilder};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LBUTTON};

use crate::services::{log, settings};

pub const WINDOW_LABEL: &str = "pet";
/// The window is a little larger than Mika so her particles (z's, sparkles) fit.
pub const SIZE: f64 = 96.0;
/// Must agree with RING_WINDOW in src/pet/layout.ts: a 7-item arc (radius up to ~135) around Mika plus its labels.
const RING: f64 = 340.0;
const CHAT_W: f64 = 420.0;
const CHAT_H: f64 = 460.0;
/// A speech bubble beside Mika ("¡Tengo un pick!"): the window widens toward the middle of the screen.
const SAY_W: f64 = 330.0;
/// The now-playing panel (cover, title, controls, volume) beside Mika.
const MEDIA_W: f64 = 420.0;
/// Mika's distance from the window edges while the chat is open.
const CORNER: f64 = SIZE / 2.0;

#[derive(Serialize, Deserialize, Clone, Copy)]
struct Spot { x: f64, y: f64 }

/// Where Mika is drawn inside the window, and how big the window is.
#[derive(Serialize, Clone, Copy)]
#[serde(rename_all = "camelCase")]
pub struct PetFrame {
    pub width: f64, pub height: f64, pub cx: f64, pub cy: f64, pub mode: &'static str,
    /// The half-circle faces away from the nearest work-area edge.
    pub arc: Option<&'static str>,
}

struct PetState {
    ready: bool,
    /// Mika's centre on screen (logical pixels). Only a drag in the plain mode changes it.
    home: Option<(f64, f64)>,
    frame: PetFrame,
}

static STATE: Mutex<PetState> = Mutex::new(PetState {
    ready: false,
    home: None,
    frame: PetFrame { width: SIZE, height: SIZE, cx: SIZE / 2.0, cy: SIZE / 2.0, mode: "pet", arc: None },
});

fn spot_path() -> std::path::PathBuf { settings::config_dir().join("pet.json") }

fn saved_spot() -> Option<Spot> {
    let spot: Spot = serde_json::from_slice(&std::fs::read(spot_path()).ok()?).ok()?;
    (spot.x.is_finite() && spot.y.is_finite()).then_some(spot)
}

fn page_url(app: &AppHandle) -> WebviewUrl {
    #[cfg(dev)]
    if let Some(mut base) = app.config().build.dev_url.clone() {
        base.set_path("/pet.html");
        return WebviewUrl::External(base);
    }
    let _ = app;
    WebviewUrl::App("pet.html".into())
}

/// Created hidden at launch, like the settings window (a WebView2 window created later comes up blank here).
pub fn create(app: &AppHandle, browser_args: &str) {
    let built = WebviewWindowBuilder::new(app, WINDOW_LABEL, page_url(app))
        .additional_browser_args(browser_args)
        .title("MIKA")
        .inner_size(SIZE, SIZE)
        .resizable(false)
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .focused(false)
        .visible(false)
        .build();
    let win = match built {
        Ok(win) => win,
        Err(err) => { log::line(format!("pet window failed: {err}")); return; }
    };
    // A drag (plain mode only) moves her home; it is saved once she stops moving.
    let generation = std::sync::Arc::new(AtomicU64::new(0));
    let moved = win.clone();
    win.on_window_event(move |event| {
        if let tauri::WindowEvent::CloseRequested { api, .. } = event {
            api.prevent_close();
            let _ = moved.emit("pet-close-chat", ());
        }
        if let tauri::WindowEvent::Moved(_) = event {
            let frame = STATE.lock().unwrap().frame;
            if frame.mode != "pet" { return; }
            let (Ok(pos), Ok(scale)) = (moved.outer_position(), moved.scale_factor()) else { return };
            let center = (pos.x as f64 / scale + frame.cx, pos.y as f64 / scale + frame.cy);
            STATE.lock().unwrap().home = Some(center);
            let mine = generation.fetch_add(1, Ordering::Relaxed) + 1;
            let generation = generation.clone();
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(Duration::from_millis(500)).await;
                if generation.load(Ordering::Relaxed) != mine { return; }
                let spot = Spot { x: center.0 - SIZE / 2.0, y: center.1 - SIZE / 2.0 };
                if let Ok(json) = serde_json::to_vec(&spot) { let _ = std::fs::write(spot_path(), json); }
            });
        }
    });
}

/// The work area of the display Mika is on (or the main one), in logical pixels: x, y, width, height.
fn work_area(win: &tauri::WebviewWindow) -> Option<(f64, f64, f64, f64)> {
    let home = STATE.lock().unwrap().home;
    // A freshly created hidden window is on the primary display, even if Mika's saved home is elsewhere.
    let saved_monitor = home.and_then(|(x, y)| win.available_monitors().ok()?.into_iter().find(|m| {
        let scale = m.scale_factor();
        let (p, s) = (m.position(), m.size());
        let (mx, my) = (p.x as f64 / scale, p.y as f64 / scale);
        x >= mx && y >= my && x < mx + s.width as f64 / scale && y < my + s.height as f64 / scale
    }));
    let monitor = saved_monitor.or_else(|| win.current_monitor().ok().flatten()).or_else(|| win.primary_monitor().ok().flatten())?;
    let scale = monitor.scale_factor();
    let area = monitor.work_area();
    Some((area.position.x as f64 / scale, area.position.y as f64 / scale,
          area.size.width as f64 / scale, area.size.height as f64 / scale))
}

/// The window for a mode, with Mika's centre staying at `home` (moved inside the screen if it has to).
pub fn frame_for(mode: &str, home: (f64, f64), area: (f64, f64, f64, f64)) -> (PetFrame, (f64, f64)) {
    let (ax, ay, aw, ah) = area;
    // Display changes and the taskbar can leave a saved centre outside the current work area.
    let home = (
        home.0.clamp(ax + SIZE / 2.0, (ax + aw - SIZE / 2.0).max(ax + SIZE / 2.0)),
        home.1.clamp(ay + SIZE / 2.0, (ay + ah - SIZE / 2.0).max(ay + SIZE / 2.0)),
    );
    let frame = match mode {
        "radial" => {
            let width = RING.min(aw);
            let height = RING.min(ah);
            let nearest = [
                (home.1 - ay, "down"), (ay + ah - home.1, "up"),
                (home.0 - ax, "right"), (ax + aw - home.0, "left"),
            ].into_iter().min_by(|a, b| a.0.total_cmp(&b.0)).unwrap().1;
            PetFrame { width, height, cx: width / 2.0, cy: height / 2.0, mode: "radial", arc: Some(nearest) }
        }
        // The chat opens toward the middle of the screen, with Mika in the corner nearest to where she sits.
        "chat" => {
            let right = home.0 > ax + aw / 2.0;
            let below = home.1 > ay + ah / 2.0;
            let width = CHAT_W.min(aw);
            let height = CHAT_H.min(ah);
            PetFrame { width, height,
                       cx: if right { width - CORNER } else { CORNER }, cy: if below { height - CORNER } else { CORNER }, mode: "chat", arc: None }
        }
        "media" => {
            let right = home.0 > ax + aw / 2.0;
            let width = MEDIA_W.min(aw);
            PetFrame { width, height: SIZE, cx: if right { width - SIZE / 2.0 } else { SIZE / 2.0 }, cy: SIZE / 2.0, mode: "media", arc: None }
        }
        "say" | "notice" => {
            let right = home.0 > ax + aw / 2.0;
            let width = SAY_W.min(aw);
            PetFrame { width, height: SIZE, cx: if right { width - SIZE / 2.0 } else { SIZE / 2.0 }, cy: SIZE / 2.0,
                       mode: if mode == "notice" { "notice" } else { "say" }, arc: None }
        }
        _ => PetFrame { width: SIZE, height: SIZE, cx: SIZE / 2.0, cy: SIZE / 2.0, mode: "pet", arc: None },
    };
    let x = (home.0 - frame.cx).clamp(ax, (ax + aw - frame.width).max(ax));
    let y = (home.1 - frame.cy).clamp(ay, (ay + ah - frame.height).max(ay));
    // Kept inside the screen, Mika may sit a little off her home: draw her where she really is.
    let frame = PetFrame { cx: home.0 - x, cy: home.1 - y, ..frame };
    (frame, (x, y))
}

/// Grows or shrinks the window for the plain Mika, the ring of agents, or the small chat.
pub fn layout(app: &AppHandle, mode: &str) -> Option<PetFrame> {
    let win = app.get_webview_window(WINDOW_LABEL)?;
    let area = work_area(&win)?;
    let home = {
        let state = STATE.lock().unwrap();
        state.home.or_else(|| {
            let (pos, scale) = (win.outer_position().ok()?, win.scale_factor().ok()?);
            Some((pos.x as f64 / scale + state.frame.cx, pos.y as f64 / scale + state.frame.cy))
        })?
    };
    let (frame, (x, y)) = frame_for(mode, home, area);
    {
        let mut state = STATE.lock().unwrap();
        state.frame = frame;
        state.home = Some((x + frame.cx, y + frame.cy));
    }
    let _ = win.set_size(LogicalSize::new(frame.width, frame.height));
    let _ = win.set_position(LogicalPosition::new(x, y));
    if mode == "chat" { let _ = win.set_focus(); }
    Some(frame)
}

/// Shows or hides the floating Mika. The first time, it sits near the bottom-right corner of the main display.
pub fn show(app: &AppHandle, visible: bool) {
    if !STATE.lock().unwrap().ready { return; }
    let Some(win) = app.get_webview_window(WINDOW_LABEL) else { return };
    if !visible {
        let _ = win.hide();
        let _ = app.emit_to(WINDOW_LABEL, "pet-visible", false);
        return;
    }
    // A tray open or a settings save must not reset an existing chat or greeting.
    if win.is_visible().unwrap_or(false) { return; }
    let Some(area) = work_area(&win) else { return };
    let (ax, ay, aw, ah) = area;
    let corner = (ax + aw - SIZE / 2.0 - 24.0, ay + ah - SIZE / 2.0 - 24.0);
    // A saved spot on a display that is gone would leave Mika out of reach.
    let home = saved_spot().map(|s| (s.x + SIZE / 2.0, s.y + SIZE / 2.0)).filter(|&(x, y)| {
        win.available_monitors().ok().is_some_and(|monitors| monitors.iter().any(|m| {
            let scale = m.scale_factor();
            let (p, s) = (m.position(), m.size());
            let (mx, my) = (p.x as f64 / scale, p.y as f64 / scale);
            x >= mx && y >= my && x <= mx + s.width as f64 / scale && y <= my + s.height as f64 / scale
        }))
    }).unwrap_or(corner);
    STATE.lock().unwrap().home = Some(home);
    let _ = layout(app, "pet");
    let _ = win.show();
    let _ = app.emit_to(WINDOW_LABEL, "pet-visible", true);
}

pub fn ready(app: &AppHandle, visible: bool) {
    STATE.lock().unwrap().ready = true;
    // A webview reload recreates its listeners and starts with visible=false, while its native window is still up.
    // show() intentionally leaves an existing window alone; replay the visibility signal to this fresh frontend.
    if visible && app.get_webview_window(WINDOW_LABEL).is_some_and(|win| win.is_visible().unwrap_or(false)) {
        let _ = app.emit_to(WINDOW_LABEL, "pet-visible", true);
        return;
    }
    show(app, visible);
}

/// Windows takes pointer capture during a native drag, so the webview may never receive pointerup.
pub async fn drag(app: &AppHandle) -> Result<(), String> {
    let held = || unsafe { GetAsyncKeyState(VK_LBUTTON.0 as i32) < 0 };
    let win = app.get_webview_window(WINDOW_LABEL).ok_or("Mascota no disponible")?;
    if !held() { return Ok(()); }
    win.start_dragging().map_err(|err| err.to_string())?;
    while held() { tokio::time::sleep(Duration::from_millis(16)).await; }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const AREA: (f64, f64, f64, f64) = (0.0, 0.0, 1920.0, 1040.0);

    #[test]
    fn a_saved_home_outside_the_work_area_keeps_the_mascot_inside_every_window() {
        // Observed on desktop: home X 2827, work area width 1920, bubble width 330, canvas centre X 1237.
        for home in [(2827.0, 968.0), (-100.0, -20.0), (900.0, 1100.0)] {
            for mode in ["pet", "say", "notice", "media", "chat", "radial"] {
                let (frame, _) = frame_for(mode, home, AREA);
                assert!(frame.cx >= SIZE / 2.0 && frame.cx <= frame.width - SIZE / 2.0, "{mode}: x {}", frame.cx);
                assert!(frame.cy >= SIZE / 2.0 && frame.cy <= frame.height - SIZE / 2.0, "{mode}: y {}", frame.cy);
            }
        }
    }

    #[test]
    fn the_ring_and_the_chat_grow_around_mika_without_moving_her() {
        let home = (900.0, 500.0);
        for mode in ["pet", "radial", "chat", "say", "notice", "media"] {
            let (frame, (x, y)) = frame_for(mode, home, AREA);
            assert_eq!((x + frame.cx, y + frame.cy), home, "{mode}");
        }
    }

    #[test]
    fn the_chat_opens_toward_the_middle_of_the_screen() {
        let (frame, _) = frame_for("chat", (1850.0, 990.0), AREA);
        assert_eq!((frame.cx, frame.cy), (CHAT_W - CORNER, CHAT_H - CORNER), "bottom-right Mika: chat above and to the left");
        let (frame, _) = frame_for("chat", (60.0, 60.0), AREA);
        assert_eq!((frame.cx, frame.cy), (CORNER, CORNER));
    }

    #[test]
    fn near_an_edge_the_window_and_the_entire_mascot_stay_on_screen() {
        let home = (30.0, 30.0);
        let (frame, (x, y)) = frame_for("radial", home, AREA);
        assert_eq!((x, y), (0.0, 0.0));
        assert_eq!((frame.cx, frame.cy), (SIZE / 2.0, SIZE / 2.0));
    }

    #[test]
    fn overlays_fit_a_work_area_smaller_than_their_preferred_size() {
        let area = (1920.0, -4.0, 300.0, 400.0);
        for home in [(1968.0, 44.0), (2172.0, 348.0), (2070.0, 196.0)] {
            for mode in ["radial", "chat", "say", "notice", "media"] {
                let (f, (x, y)) = frame_for(mode, home, area);
                assert!(x >= area.0 && x + f.width <= area.0 + area.2, "{mode}: horizontal bounds");
                assert!(y >= area.1 && y + f.height <= area.1 + area.3, "{mode}: vertical bounds");
                assert_eq!((x + f.cx, y + f.cy), home, "{mode}: keep Mika where she was dropped");
            }
        }
    }

    #[test]
    fn the_half_circle_faces_away_from_the_nearest_edge_on_either_display() {
        for (home, arc) in [
            ((960.0, 48.0), "down"), ((960.0, 992.0), "up"),
            ((48.0, 520.0), "right"), ((1872.0, 520.0), "left"),
        ] {
            let (frame, _) = frame_for("radial", home, AREA);
            assert_eq!(frame.arc, Some(arc));
            let shifted = (AREA.0 + 1920.0, AREA.1 - 4.0, AREA.2, AREA.3);
            let (frame, _) = frame_for("radial", (home.0 + 1920.0, home.1 - 4.0), shifted);
            assert_eq!(frame.arc, Some(arc));
        }
    }

    #[test]
    fn hook_notices_have_the_same_native_size_as_a_greeting_bubble() {
        for home in [(48.0, 48.0), (1872.0, 48.0), (48.0, 992.0), (1872.0, 992.0)] {
            let (greeting, _) = frame_for("say", home, AREA);
            let (notice, position) = frame_for("notice", home, AREA);
            assert_eq!(notice.mode, "notice");
            assert_eq!((notice.width, notice.height), (greeting.width, greeting.height));
            assert_eq!((position.0 + notice.cx, position.1 + notice.cy), home);
        }
    }

    #[test]
    fn the_ring_window_is_square_clamped_to_the_screen_and_keeps_every_corner_and_edge_at_the_right_distance() {
        // The ring geometry itself (arc, spacing, labels) is tested in tests/pet-layout.test.mjs against these frames.
        let homes = [
            ((48.0, 48.0), "down", true, true), ((1872.0, 48.0), "down", true, true),
            ((48.0, 992.0), "up", true, true), ((1872.0, 992.0), "up", true, true),
            ((960.0, 48.0), "down", false, true), ((960.0, 992.0), "up", false, true),
            ((48.0, 520.0), "right", true, false), ((1872.0, 520.0), "left", true, false),
            ((960.0, 520.0), "down", false, false),
        ];
        for (home, arc, near_x, near_y) in homes {
            let (f, (x, y)) = frame_for("radial", home, AREA);
            assert_eq!((f.width, f.height), (RING, RING), "{home:?}");
            assert_eq!(f.arc, Some(arc), "{home:?}");
            assert!(x >= AREA.0 && y >= AREA.1 && x + f.width <= AREA.0 + AREA.2 && y + f.height <= AREA.1 + AREA.3, "{home:?}: window inside the work area");
            // Mika is off-centre exactly when an edge pushes the window; the TS layout reads this to pick quarter or half circle.
            assert_eq!(f.cx < RING / 2.0 - 1.0 || f.cx > RING / 2.0 + 1.0, near_x, "{home:?}: cx {}", f.cx);
            assert_eq!(f.cy < RING / 2.0 - 1.0 || f.cy > RING / 2.0 + 1.0, near_y, "{home:?}: cy {}", f.cy);
            assert_eq!((x + f.cx, y + f.cy), home);
        }
    }
}
