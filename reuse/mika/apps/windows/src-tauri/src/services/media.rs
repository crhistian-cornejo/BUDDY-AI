//! Media control: what Windows reports as playing (Spotify, YouTube in Chrome/Edge, any player that talks to the
//! system media overlay) and the three buttons previous / play-pause / next.
//!
//! It uses the Global System Media Transport Controls (WinRT `Windows.Media.Control`): the same source as the
//! volume flyout. No network, no keys, nothing leaves the machine.
//!
//! Cost model. Nothing runs by default. The frontend calls `media_watch(true)` only while the island is open and
//! the setting is on; that starts ONE worker thread that sleeps on a channel and wakes when Windows raises
//! CurrentSessionChanged / MediaPropertiesChanged / PlaybackInfoChanged (bursts are debounced). `media_watch(false)`
//! removes the handlers and ends the thread. There is no polling loop.

use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{AppHandle, Emitter};

use crate::services::{base64, log};

/// Event name; payload is `Option<NowPlaying>` (null = nothing to show).
pub const EVENT: &str = "media-changed";

/// Longest edge of the thumbnail we hand to the webview, and the most bytes of JPEG we allow.
const THUMB_EDGE: u32 = 96;
const THUMB_MAX_BYTES: usize = 64 * 1024;
/// Do not even try to decode an image larger than this.
const THUMB_MAX_SOURCE: u64 = 8 * 1024 * 1024;
/// Text fields are cut to this many characters (the card ellipsizes much sooner).
const TEXT_MAX: usize = 200;

// ── Pure parts ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    PlayPause,
    Next,
    Previous,
}

impl Action {
    /// Only these three strings are accepted from the webview.
    pub fn parse(s: &str) -> Option<Action> {
        match s {
            "play_pause" => Some(Action::PlayPause),
            "next" => Some(Action::Next),
            "previous" => Some(Action::Previous),
            _ => None,
        }
    }
}

/// What the island shows. Mirror of the TypeScript `NowPlaying` (src/integrations/media.ts).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NowPlaying {
    /// Friendly source name: "Spotify", "Chrome", "Edge"…
    pub app: String,
    pub title: String,
    pub artist: String,
    /// "playing" | "paused" | "stopped"
    pub status: &'static str,
    pub position_ms: Option<u64>,
    pub duration_ms: Option<u64>,
    /// `data:image/jpeg;base64,…`, at most ~64 KB, or null.
    pub thumbnail: Option<String>,
    pub can_play_pause: bool,
    pub can_next: bool,
    pub can_previous: bool,
}

/// The raw values read from a session, before shaping.
#[derive(Debug, Default, Clone)]
pub struct Raw {
    pub aumid: String,
    pub title: String,
    pub artist: String,
    /// 4 = playing, 5 = paused; anything else counts as stopped (see `status_name`).
    pub status: i32,
    pub position_100ns: Option<i64>,
    pub start_100ns: Option<i64>,
    pub end_100ns: Option<i64>,
    pub can_play_pause: bool,
    pub can_next: bool,
    pub can_previous: bool,
}

/// GlobalSystemMediaTransportControlsSessionPlaybackStatus: Closed 0, Opened 1, Changing 2, Stopped 3, Playing 4, Paused 5.
pub fn status_name(code: i32) -> &'static str {
    match code {
        4 => "playing",
        5 => "paused",
        _ => "stopped",
    }
}

fn clean(s: &str) -> String {
    s.trim().chars().take(TEXT_MAX).collect()
}

/// "Spotify.exe", "SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify", "Chrome", "MSEdge"… → a name for a human.
pub fn friendly_app(aumid: &str) -> String {
    let lower = aumid.to_lowercase();
    for (needle, name) in [
        ("spotify", "Spotify"),
        ("msedge", "Edge"),
        ("chrome", "Chrome"),
        ("firefox", "Firefox"),
        ("brave", "Brave"),
        ("opera", "Opera"),
        ("vlc", "VLC"),
        ("zune", "Media Player"),
        ("applemusic", "Apple Music"),
        ("itunes", "iTunes"),
    ] {
        if lower.contains(needle) {
            return name.to_string();
        }
    }
    let tail = aumid.rsplit('!').next().unwrap_or(aumid);
    let tail = tail.split('_').next().unwrap_or(tail);
    let tail = tail.strip_suffix(".exe").or_else(|| tail.strip_suffix(".EXE")).unwrap_or(tail);
    let tail = tail.rsplit('.').find(|p| !p.is_empty()).unwrap_or(tail);
    let mut chars = tail.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

fn ticks_to_ms(t: i64) -> Option<u64> {
    (t >= 0).then(|| (t / 10_000) as u64)
}

/// Raw session values → payload. `None` when there is nothing worth a card (no title and no artist).
pub fn shape(raw: &Raw, thumbnail: Option<String>) -> Option<NowPlaying> {
    let title = clean(&raw.title);
    let artist = clean(&raw.artist);
    if title.is_empty() && artist.is_empty() {
        return None;
    }
    let start = raw.start_100ns.unwrap_or(0);
    let duration_ms = raw.end_100ns.and_then(|end| ticks_to_ms(end - start)).filter(|d| *d > 0);
    let position_ms = raw
        .position_100ns
        .and_then(|p| ticks_to_ms(p - start))
        .map(|p| duration_ms.map_or(p, |d| p.min(d)));
    Some(NowPlaying {
        app: friendly_app(&raw.aumid),
        title,
        artist,
        status: status_name(raw.status),
        position_ms,
        duration_ms,
        thumbnail: thumbnail.filter(|t| t.len() <= THUMB_MAX_BYTES * 4 / 3 + 64),
        can_play_pause: raw.can_play_pause,
        can_next: raw.can_next,
        can_previous: raw.can_previous,
    })
}

/// JPEG bytes → data URL, refusing anything over the cap.
pub fn jpeg_data_url(bytes: &[u8]) -> Option<String> {
    if bytes.is_empty() || bytes.len() > THUMB_MAX_BYTES {
        return None;
    }
    Some(format!("data:image/jpeg;base64,{}", base64(bytes)))
}

/// Size that fits inside `edge`×`edge` keeping the aspect ratio; never upscales, never returns 0.
pub fn fit_within(w: u32, h: u32, edge: u32) -> (u32, u32) {
    if w == 0 || h == 0 {
        return (edge.max(1), edge.max(1));
    }
    if w <= edge && h <= edge {
        return (w, h);
    }
    let scale = edge as f64 / w.max(h) as f64;
    (((w as f64 * scale).round() as u32).max(1), ((h as f64 * scale).round() as u32).max(1))
}

// ── Windows ───────────────────────────────────────────────────────────────────

#[cfg(windows)]
mod win {
    use super::*;
    use windows::core::Interface;
    use windows::Foundation::TypedEventHandler;
    use windows::Graphics::Imaging::{
        BitmapAlphaMode, BitmapDecoder, BitmapEncoder, BitmapInterpolationMode, BitmapPixelFormat, BitmapTransform,
        ColorManagementMode, ExifOrientationMode,
    };
    use windows::Media::Control::{
        GlobalSystemMediaTransportControlsSession as Session, GlobalSystemMediaTransportControlsSessionManager as Manager,
    };
    use windows::Storage::Streams::{DataReader, IRandomAccessStream, IRandomAccessStreamReference, InMemoryRandomAccessStream};
    use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};

    /// Blocking threads: join the multithreaded apartment (a repeat call is harmless).
    pub fn init_com() {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        }
    }

    pub fn manager() -> Option<Manager> {
        Manager::RequestAsync().ok()?.get().ok()
    }

    pub fn current(manager: &Manager) -> Option<Session> {
        manager.GetCurrentSession().ok()
    }

    fn read_stream(stream: &IRandomAccessStream) -> Option<Vec<u8>> {
        let size = stream.Size().ok()?;
        if size == 0 || size > THUMB_MAX_SOURCE {
            return None;
        }
        let reader = DataReader::CreateDataReader(&stream.GetInputStreamAt(0).ok()?).ok()?;
        reader.LoadAsync(size as u32).ok()?.get().ok()?;
        let mut buf = vec![0u8; size as usize];
        reader.ReadBytes(&mut buf).ok()?;
        Some(buf)
    }

    /// Decodes whatever Windows hands over (JPEG, PNG…) and re-encodes it as a small JPEG.
    fn thumbnail(reference: &IRandomAccessStreamReference) -> Option<String> {
        let source = reference.OpenReadAsync().ok()?.get().ok()?;
        let stream: IRandomAccessStream = source.cast().ok()?;
        if stream.Size().ok()? > THUMB_MAX_SOURCE {
            return None;
        }
        let decoder = BitmapDecoder::CreateAsync(&stream).ok()?.get().ok()?;
        let (w, h) = fit_within(decoder.PixelWidth().ok()?, decoder.PixelHeight().ok()?, THUMB_EDGE);
        let transform = BitmapTransform::new().ok()?;
        transform.SetScaledWidth(w).ok()?;
        transform.SetScaledHeight(h).ok()?;
        transform.SetInterpolationMode(BitmapInterpolationMode::Fant).ok()?;
        let pixels = decoder
            .GetPixelDataTransformedAsync(
                BitmapPixelFormat::Bgra8,
                BitmapAlphaMode::Straight,
                &transform,
                ExifOrientationMode::IgnoreExifOrientation,
                ColorManagementMode::DoNotColorManage,
            )
            .ok()?
            .get()
            .ok()?
            .DetachPixelData()
            .ok()?;
        let out = InMemoryRandomAccessStream::new().ok()?;
        let encoder = BitmapEncoder::CreateAsync(BitmapEncoder::JpegEncoderId().ok()?, &out).ok()?.get().ok()?;
        encoder.SetPixelData(BitmapPixelFormat::Bgra8, BitmapAlphaMode::Ignore, w, h, 96.0, 96.0, &pixels).ok()?;
        encoder.FlushAsync().ok()?.get().ok()?;
        let out_stream: IRandomAccessStream = out.cast().ok()?;
        jpeg_data_url(&read_stream(&out_stream)?)
    }

    /// Reads the session. `cache` holds the last successful thumbnail with the track it belonged to, so a
    /// play/pause or a seek never decodes the cover again.
    pub fn read(session: &Session, cache: &mut Option<(String, String)>) -> Option<NowPlaying> {
        let props = session.TryGetMediaPropertiesAsync().ok()?.get().ok()?;
        let info = session.GetPlaybackInfo().ok()?;
        let controls = info.Controls().ok();
        let timeline = session.GetTimelineProperties().ok();
        let raw = Raw {
            aumid: session.SourceAppUserModelId().ok()?.to_string(),
            title: props.Title().map(|t| t.to_string()).unwrap_or_default(),
            artist: props.Artist().map(|t| t.to_string()).unwrap_or_default(),
            status: info.PlaybackStatus().ok()?.0,
            position_100ns: timeline.as_ref().and_then(|t| t.Position().ok()).map(|t| t.Duration),
            start_100ns: timeline.as_ref().and_then(|t| t.StartTime().ok()).map(|t| t.Duration),
            end_100ns: timeline.as_ref().and_then(|t| t.EndTime().ok()).map(|t| t.Duration),
            can_play_pause: controls
                .as_ref()
                .is_some_and(|c| c.IsPlayPauseToggleEnabled().unwrap_or(false) || c.IsPlayEnabled().unwrap_or(false) || c.IsPauseEnabled().unwrap_or(false)),
            can_next: controls.as_ref().is_some_and(|c| c.IsNextEnabled().unwrap_or(false)),
            can_previous: controls.as_ref().is_some_and(|c| c.IsPreviousEnabled().unwrap_or(false)),
        };
        let key = format!("{}\u{1}{}\u{1}{}", raw.aumid, raw.title, raw.artist);
        let thumb = match cache {
            Some((k, t)) if *k == key => Some(t.clone()),
            _ => {
                let fresh = props.Thumbnail().ok().and_then(|r| thumbnail(&r));
                if let Some(t) = &fresh {
                    *cache = Some((key, t.clone()));
                } else {
                    *cache = None;
                }
                fresh
            }
        };
        shape(&raw, thumb)
    }

    pub fn control(action: Action) -> bool {
        let Some(manager) = manager() else { return false };
        let Some(session) = current(&manager) else { return false };
        let op = match action {
            Action::PlayPause => session.TryTogglePlayPauseAsync(),
            Action::Next => session.TrySkipNextAsync(),
            Action::Previous => session.TrySkipPreviousAsync(),
        };
        op.ok().and_then(|o| o.get().ok()).unwrap_or(false)
    }

    pub fn current_aumid() -> Option<String> {
        let manager = manager()?;
        let session = current(&manager)?;
        Some(session.SourceAppUserModelId().ok()?.to_string()).filter(|s| !s.is_empty())
    }

    pub fn now_playing() -> Option<NowPlaying> {
        let manager = manager()?;
        let session = current(&manager)?;
        read(&session, &mut None)
    }

    /// The worker: handlers on the manager and on the current session post `Changed`; everything else happens here.
    pub fn watch(app: AppHandle, owners: Owners, tx: Sender<Msg>, rx: mpsc::Receiver<Msg>) {
        init_com();
        let Some(manager) = manager() else {
            log::line("media: no session manager");
            return;
        };
        let t = tx.clone();
        let manager_token = manager
            .CurrentSessionChanged(&TypedEventHandler::new(move |_, _| {
                let _ = t.send(Msg::Changed);
                Ok(())
            }))
            .ok();

        // (session, properties-token, playback-token)
        let mut bound: Option<(Session, Option<i64>, Option<i64>)> = None;
        let mut cache = None;
        let mut last: Option<Option<NowPlaying>> = None;

        'outer: loop {
            // Follow the system's current session: move the two handlers when it changes.
            let session = current(&manager);
            let same = match (&bound, &session) {
                (Some((b, _, _)), Some(s)) => b.as_raw() == s.as_raw(),
                (None, None) => true,
                _ => false,
            };
            if !same {
                if let Some((old, p, q)) = bound.take() {
                    if let Some(p) = p { let _ = old.RemoveMediaPropertiesChanged(p); }
                    if let Some(q) = q { let _ = old.RemovePlaybackInfoChanged(q); }
                }
                if let Some(s) = &session {
                    let (a, b) = (tx.clone(), tx.clone());
                    let p = s
                        .MediaPropertiesChanged(&TypedEventHandler::new(move |_, _| {
                            let _ = a.send(Msg::Changed);
                            Ok(())
                        }))
                        .ok();
                    let q = s
                        .PlaybackInfoChanged(&TypedEventHandler::new(move |_, _| {
                            let _ = b.send(Msg::Changed);
                            Ok(())
                        }))
                        .ok();
                    bound = Some((s.clone(), p, q));
                }
            }

            let now = session.as_ref().and_then(|s| read(s, &mut cache));
            if last.as_ref() != Some(&now) {
                for label in owners.lock().unwrap().iter() {
                    let _ = app.emit_to(label.as_str(), EVENT, now.clone());
                }
                last = Some(now);
            }

            // Sleep until Windows says something changed, then let a burst settle (at most 600 ms).
            match rx.recv() {
                Ok(Msg::Changed) => {}
                _ => break,
            }
            let deadline = Instant::now() + Duration::from_millis(600);
            loop {
                let wait = Duration::from_millis(150).min(deadline.saturating_duration_since(Instant::now()));
                match rx.recv_timeout(wait) {
                    Ok(Msg::Changed) if Instant::now() < deadline => continue,
                    Ok(Msg::Changed) | Err(RecvTimeoutError::Timeout) => break,
                    Ok(Msg::Stop) | Err(RecvTimeoutError::Disconnected) => break 'outer,
                }
            }
        }

        if let Some(token) = manager_token {
            let _ = manager.RemoveCurrentSessionChanged(token);
        }
        if let Some((s, p, q)) = bound {
            if let Some(p) = p { let _ = s.RemoveMediaPropertiesChanged(p); }
            if let Some(q) = q { let _ = s.RemovePlaybackInfoChanged(q); }
        }
    }
}

// ── Public surface ────────────────────────────────────────────────────────────

pub enum Msg {
    Changed,
    Stop,
}

/// Windows (webview labels) that want `media-changed`: the island while it is open, the floating Mika while she is on screen.
pub type Owners = Arc<Mutex<Vec<String>>>;

#[derive(Default)]
struct Inner {
    tx: Option<Sender<Msg>>,
    owners: Owners,
}

/// The running worker's mailbox, if any, and who listens. Managed state (lib.rs).
#[derive(Default)]
pub struct MediaWatch(Mutex<Inner>);

/// Adds or removes `label` from the listeners. True when the worker has to run afterwards.
pub fn update_owners(owners: &mut Vec<String>, label: &str, on: bool) -> bool {
    owners.retain(|l| l != label);
    if on {
        owners.push(label.to_string());
    }
    !owners.is_empty()
}

impl MediaWatch {
    /// `label` starts (true) or stops (false) listening. One worker serves every listener; it ends with the last one.
    /// Idempotent.
    pub fn set(&self, app: &AppHandle, label: &str, on: bool) {
        let mut inner = self.0.lock().unwrap();
        let run = update_owners(&mut inner.owners.lock().unwrap(), label, on);
        if !run {
            if let Some(tx) = inner.tx.take() {
                let _ = tx.send(Msg::Stop);
            }
            return;
        }
        // Already running: the new listener asks for the current state itself (media_now_playing).
        if inner.tx.is_some() {
            return;
        }
        #[cfg(windows)]
        {
            let (tx, rx) = mpsc::channel();
            let app = app.clone();
            let own = tx.clone();
            let owners = inner.owners.clone();
            if std::thread::Builder::new().name("mika-media".into()).spawn(move || win::watch(app, owners, own, rx)).is_ok() {
                inner.tx = Some(tx);
            }
        }
        #[cfg(not(windows))]
        let _ = app;
    }
}

/// Blocking threads: join the COM multithreaded apartment (a repeat call is harmless).
pub fn init_com() {
    #[cfg(windows)]
    win::init_com();
}

/// The app id of the system's current media session ("Spotify.exe", "Chrome"...), or `None`.
pub fn current_aumid() -> Option<String> {
    #[cfg(windows)]
    {
        win::init_com();
        std::panic::catch_unwind(win::current_aumid).ok().flatten()
    }
    #[cfg(not(windows))]
    None
}

/// The current session as the island shows it, or `None` (nothing playing, app closed, any failure).
pub fn now_playing() -> Option<NowPlaying> {
    #[cfg(windows)]
    {
        win::init_com();
        // A WinRT hiccup must never take the app down.
        std::panic::catch_unwind(win::now_playing).ok().flatten()
    }
    #[cfg(not(windows))]
    None
}

/// Sends the action to the current session. False when there is none or it refused.
pub fn control(action: Action) -> bool {
    #[cfg(windows)]
    {
        win::init_com();
        std::panic::catch_unwind(|| win::control(action)).unwrap_or(false)
    }
    #[cfg(not(windows))]
    {
        let _ = action;
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_three_actions_are_accepted() {
        assert_eq!(Action::parse("play_pause"), Some(Action::PlayPause));
        assert_eq!(Action::parse("next"), Some(Action::Next));
        assert_eq!(Action::parse("previous"), Some(Action::Previous));
        for bad in ["", "Next", "stop", "play", "next ", "seek", "../x"] {
            assert_eq!(Action::parse(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn the_worker_runs_while_anyone_listens() {
        let mut owners = Vec::new();
        assert!(update_owners(&mut owners, "island", true));
        assert!(update_owners(&mut owners, "pet", true));
        assert!(update_owners(&mut owners, "pet", true), "asking twice does not duplicate");
        assert_eq!(owners, ["island", "pet"]);
        assert!(update_owners(&mut owners, "island", false), "the pet still listens");
        assert!(!update_owners(&mut owners, "pet", false));
        assert!(!update_owners(&mut owners, "ghost", false));
    }

    #[test]
    fn status_codes_map_to_three_names() {
        assert_eq!(status_name(4), "playing");
        assert_eq!(status_name(5), "paused");
        for other in [0, 1, 2, 3, 9, -1] {
            assert_eq!(status_name(other), "stopped");
        }
    }

    #[test]
    fn app_ids_become_friendly_names() {
        assert_eq!(friendly_app("Spotify.exe"), "Spotify");
        assert_eq!(friendly_app("SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify"), "Spotify");
        assert_eq!(friendly_app("Chrome"), "Chrome");
        assert_eq!(friendly_app("MSEdge"), "Edge");
        assert_eq!(friendly_app("Microsoft.ZuneMusic_8wekyb3d8bbwe!Microsoft.ZuneMusic"), "Media Player");
        assert_eq!(friendly_app("tidal.exe"), "Tidal");
        assert_eq!(friendly_app(""), "");
    }

    fn raw() -> Raw {
        Raw {
            aumid: "Spotify.exe".into(),
            title: "  Song  ".into(),
            artist: "Band".into(),
            status: 4,
            position_100ns: Some(30 * 10_000_000),
            start_100ns: Some(0),
            end_100ns: Some(200 * 10_000_000),
            can_play_pause: true,
            can_next: true,
            can_previous: false,
        }
    }

    #[test]
    fn payload_is_shaped_and_trimmed() {
        let p = shape(&raw(), None).unwrap();
        assert_eq!((p.app.as_str(), p.title.as_str(), p.artist.as_str(), p.status), ("Spotify", "Song", "Band", "playing"));
        assert_eq!((p.position_ms, p.duration_ms), (Some(30_000), Some(200_000)));
        assert!(p.can_play_pause && p.can_next && !p.can_previous);
        assert_eq!(p.thumbnail, None);
        let json = serde_json::to_value(&p).unwrap();
        assert_eq!(json["durationMs"], 200_000);
        assert_eq!(json["canPlayPause"], true);
    }

    #[test]
    fn nothing_to_show_is_none_and_odd_timelines_are_dropped() {
        let mut r = raw();
        r.title = "  ".into();
        r.artist = String::new();
        assert!(shape(&r, None).is_none());
        let mut r = raw();
        r.end_100ns = Some(0);
        r.position_100ns = Some(-5);
        let p = shape(&r, None).unwrap();
        assert_eq!((p.position_ms, p.duration_ms), (None, None));
        let mut r = raw();
        r.position_100ns = Some(900 * 10_000_000);
        assert_eq!(shape(&r, None).unwrap().position_ms, Some(200_000));
        let mut r = raw();
        r.title = "x".repeat(1000);
        assert_eq!(shape(&r, None).unwrap().title.chars().count(), TEXT_MAX);
    }

    #[test]
    fn thumbnails_over_the_cap_are_refused() {
        assert!(jpeg_data_url(&[]).is_none());
        let ok = jpeg_data_url(&[1, 2, 3]).unwrap();
        assert_eq!(ok, "data:image/jpeg;base64,AQID");
        assert!(jpeg_data_url(&vec![0u8; THUMB_MAX_BYTES]).is_some());
        assert!(jpeg_data_url(&vec![0u8; THUMB_MAX_BYTES + 1]).is_none());
        let huge = Some("x".repeat(THUMB_MAX_BYTES * 2));
        assert!(shape(&raw(), huge).unwrap().thumbnail.is_none());
    }

    #[test]
    fn thumbnails_keep_their_aspect_and_never_upscale() {
        assert_eq!(fit_within(640, 640, 96), (96, 96));
        assert_eq!(fit_within(1280, 720, 96), (96, 54));
        assert_eq!(fit_within(50, 40, 96), (50, 40));
        assert_eq!(fit_within(10000, 1, 96), (96, 1));
        assert_eq!(fit_within(0, 0, 96), (96, 96));
    }

    /// Manual check against whatever is playing right now: `cargo test live_now_playing -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn live_now_playing() {
        let p = now_playing();
        println!("{:?}", p.map(|mut p| { p.thumbnail = p.thumbnail.map(|t| format!("{} bytes", t.len())); p }));
    }
}
