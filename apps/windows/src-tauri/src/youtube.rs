use crate::AppCore;
use tauri::{AppHandle, Manager, State};

#[tauri::command]
pub fn youtube_status(state: State<'_, AppCore>) -> buddy_core::YouTubeStatus {
    state.core.youtube_status()
}
#[tauri::command]
pub fn youtube_extension_path(state: State<'_, AppCore>) -> String {
    state.core.youtube_extension_path()
}
#[tauri::command]
pub fn youtube_prepare(state: State<'_, AppCore>, browser: String) -> Result<String, String> {
    let host = std::env::current_exe().map_err(|e| e.to_string())?;
    state
        .core
        .youtube_prepare_with_host(browser, host.to_string_lossy().into_owned())
        .map_err(|e| e.to_string())
}
#[tauri::command]
pub fn youtube_enable(state: State<'_, AppCore>, enabled: bool) -> Result<(), String> {
    state
        .core
        .youtube_enable(enabled)
        .map_err(|e| e.to_string())
}
#[tauri::command]
pub fn youtube_open(
    state: State<'_, AppCore>,
    source_id: String,
    video_id: String,
) -> Result<buddy_core::YouTubeVideo, String> {
    state
        .core
        .youtube_open(source_id, video_id)
        .map_err(|e| e.to_string())
}
#[tauri::command]
pub fn youtube_started(state: State<'_, AppCore>, source_id: String, video_id: String) {
    state.core.youtube_started(source_id, video_id);
}
#[tauri::command]
pub fn youtube_close(state: State<'_, AppCore>) {
    state.core.youtube_close();
}
#[tauri::command]
pub fn youtube_toggle(state: State<'_, AppCore>, source_id: String, video_id: String) -> Result<(), String> {
    state.core.youtube_toggle(source_id, video_id).map_err(|e| e.to_string())
}
#[tauri::command]
pub fn youtube_show_folder(app: AppHandle) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    let folder = app.state::<AppCore>().core.youtube_extension_path();
    if folder.is_empty() {
        return Err("Configura primero la extensión.".into());
    }
    app.opener()
        .open_path(folder, None::<&str>)
        .map_err(|e| e.to_string())
}
#[tauri::command]
pub fn youtube_copy_folder(state: State<'_, AppCore>) -> Result<(), String> {
    let folder = state.core.youtube_extension_path();
    if folder.is_empty() {
        return Err("Configura primero la extensión.".into());
    }
    arboard::Clipboard::new()
        .and_then(|mut c| c.set_text(folder))
        .map_err(|e| e.to_string())
}
#[tauri::command]
pub fn youtube_browser_page(browser: String) -> Result<(), String> {
    if !["chrome", "edge"].contains(&browser.as_str()) {
        return Err("Elige Chrome o Edge.".into());
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let file = if browser == "chrome" {
            "Google/Chrome/Application/chrome.exe"
        } else {
            "Microsoft/Edge/Application/msedge.exe"
        };
        let exe = ["LOCALAPPDATA", "PROGRAMFILES", "PROGRAMFILES(X86)"]
            .into_iter()
            .filter_map(std::env::var_os)
            .map(|root| std::path::PathBuf::from(root).join(file))
            .find(|p| p.is_file())
            .ok_or("No se encontró ese navegador. Elige el que tienes instalado.")?;
        std::process::Command::new(exe)
            .arg(format!("{browser}://extensions"))
            .creation_flags(0x08000000)
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .args([
                "-a",
                if browser == "chrome" {
                    "Google Chrome"
                } else {
                    "Microsoft Edge"
                },
                "--args",
                &format!("{browser}://extensions"),
            ])
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// YouTube explicitly requires desktop WebViews to identify the app through Referer.
pub fn identify_player(window: &tauri::WebviewWindow) {
    #[cfg(windows)]
    let _ = window.with_webview(|webview| unsafe {
        use webview2_com::{
            Microsoft::Web::WebView2::Win32::COREWEBVIEW2_WEB_RESOURCE_CONTEXT_DOCUMENT,
            WebResourceRequestedEventHandler,
        };
        use windows_core::HSTRING;
        let Ok(core) = webview.controller().CoreWebView2() else {
            return;
        };
        let _ = core.AddWebResourceRequestedFilter(
            &HSTRING::from("https://www.youtube.com/embed/*"),
            COREWEBVIEW2_WEB_RESOURCE_CONTEXT_DOCUMENT,
        );
        let handler = WebResourceRequestedEventHandler::create(Box::new(|_, args| {
            if let Some(args) = args {
                args.Request()?.Headers()?.SetHeader(
                    &HSTRING::from("Referer"),
                    &HSTRING::from("https://io.github.crhistian-cornejo.buddy"),
                )?;
            }
            Ok(())
        }));
        let mut token = 0;
        let _ = core.add_WebResourceRequested(&handler, &mut token);
    });
    #[cfg(not(windows))]
    let _ = window;
}

#[tauri::command]
pub fn youtube_open_floating(app: AppHandle, source_id: String, video_id: String) -> Result<buddy_core::YouTubeVideo, String> {
    let v = app.state::<AppCore>().core.youtube_open_floating(source_id, video_id).map_err(|e| e.to_string())?;
    Ok(v)
}
#[tauri::command]
pub fn youtube_move(app: AppHandle, destination: String) -> Result<(), String> {
    app.state::<AppCore>().core.youtube_move(destination).map_err(|e| e.to_string())?;
    Ok(())
}
#[tauri::command]
pub fn youtube_position(state: State<'_, AppCore>, source_id: String, video_id: String, seconds: f64) {
    state.core.youtube_position(source_id, video_id, seconds);
}
#[tauri::command]
pub fn video_browser_pip(state: State<'_, AppCore>, source_id: String, video_id: String) -> Result<(), String> {
    state.core.video_browser_pip(source_id, video_id).map_err(|e| e.to_string())
}
#[tauri::command]
pub fn video_ask(app: AppHandle) -> Result<(), String> {
    use std::sync::atomic::Ordering;
    use tauri::Emitter;
    let state = app.state::<AppCore>();
    state.video_question.store(true, Ordering::SeqCst);
    if !state.chat_open.load(Ordering::SeqCst) { crate::toggle_chat(app.clone())?; }
    app.emit_to(crate::CHAT, "video-question", ()).map_err(|e| e.to_string())
}
#[tauri::command]
pub fn video_take_question(state: State<'_, AppCore>) -> bool {
    state.video_question.swap(false, std::sync::atomic::Ordering::SeqCst)
}
#[tauri::command]
pub fn send_video_message(state: State<'_, AppCore>, chat_id: Option<String>, text: String) -> Result<String, String> {
    state.core.send_video_message(chat_id, text).map_err(|e| e.to_string())
}
pub fn sync_window(app: &AppHandle) -> Result<(), String> {
    use tauri::{WebviewUrl, WebviewWindowBuilder, LogicalPosition};
    let status = app.state::<AppCore>().core.youtube_status();
    if status.viewer.is_none() || status.destination != "floating" {
        if let Some(w) = app.get_webview_window("video") { let _ = w.close(); }
        return Ok(());
    }
    if app.get_webview_window("video").is_some() { return Ok(()); }
    let window = WebviewWindowBuilder::new(app, "video", WebviewUrl::App("video.html".into()))
        .title("Video · Buddy").inner_size(480.0, 350.0).min_inner_size(400.0, 290.0)
        .always_on_top(true).resizable(true).skip_taskbar(true).visible(false)
        .build().map_err(|e| e.to_string())?;
    identify_player(&window);
    if let Some(pet) = app.get_webview_window(crate::PET) {
        if let Ok((r, area)) = crate::pet_rects(&pet) {
            let placed = buddy_core::clamp_to_area(buddy_core::PetRect { x: r.x + r.width / 2.0 - 240.0, y: r.y - 380.0, width: 480.0, height: 380.0 }, area);
            let _ = window.set_position(LogicalPosition::new(placed.x, placed.y));
        }
    }
    let handle = app.clone();
    window.on_window_event(move |event| {
        if matches!(event, tauri::WindowEvent::Destroyed) {
            let state = handle.state::<AppCore>();
            if state.core.youtube_status().destination == "floating" { state.core.youtube_close(); }
        }
    });
    window.show().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn is_video_chat(state: State<'_, AppCore>, chat_id: String) -> bool { state.core.is_video_chat(chat_id) }
