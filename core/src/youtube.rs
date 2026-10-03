//! Local browser handoff. No network, account credentials, scraping of media, or history persistence.
use crate::{CoreError, Event, EventBus};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

pub const HOST: &str = "io.github.crhistian_cornejo.buddy_youtube";
pub const EXTENSION_ID: &str = include_str!("../../extensions/youtube/extension-id.txt");
const FRESH: Duration = Duration::from_secs(30);

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct YouTubeVideo {
    pub source_id: String,
    pub tab_id: u32,
    pub video_id: String,
    pub title: String,
    pub browser: String,
    pub seconds: f64,
    #[serde(default)]
    pub duration: Option<f64>,
    pub playing: bool,
    #[serde(default)]
    pub caption: String,
    pub service: String,
    pub url: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct YouTubeStatus {
    pub enabled: bool,
    pub connected: bool,
    pub detected: Option<YouTubeVideo>,
    pub viewer: Option<YouTubeVideo>,
    pub destination: String,
}

struct Connection {
    seen: Instant,
    videos: Vec<(YouTubeVideo, bool)>,
    commands: Vec<Value>,
}
#[derive(Default)]
struct State {
    enabled: bool,
    connections: HashMap<String, Connection>,
    viewer: Option<YouTubeVideo>,
    destination: String,
}
pub struct YouTube {
    state: Mutex<State>,
    bus: Arc<EventBus>,
}

impl YouTube {
    pub fn new(bus: Arc<EventBus>) -> Self {
        Self {
            state: Mutex::new(State::default()),
            bus,
        }
    }
    pub fn enable(&self, enabled: bool) {
        let mut s = self.state.lock().unwrap_or_else(|p| p.into_inner());
        s.enabled = enabled;
        if !enabled {
            s.connections.clear();
            s.viewer = None;
        }
        drop(s);
        self.bus.publish(Event::YouTubeChanged);
    }
    pub fn status(&self) -> YouTubeStatus {
        let s = self.state.lock().unwrap_or_else(|p| p.into_inner());
        let detected = s
            .connections
            .values()
            .filter(|c| c.seen.elapsed() < FRESH)
            .flat_map(|c| c.videos.iter().map(move |(v, active)| (v, *active, c.seen)))
            .max_by_key(|(v, active, seen)| (*active, v.playing, *seen))
            .map(|(v, _, _)| v.clone());
        YouTubeStatus {
            enabled: s.enabled,
            connected: s.enabled && s.connections.values().any(|c| c.seen.elapsed() < FRESH),
            detected: if s.enabled { detected } else { None },
            viewer: s.viewer.clone(),
            destination: if s.destination.is_empty() { "notch".into() } else { s.destination.clone() },
        }
    }
    /// Runs on the private, owner-only hooks channel, under a separate protocol from agent tools.
    pub fn receive(&self, payload: &Value) -> String {
        let source = payload["sourceId"].as_str().unwrap_or("");
        if source.is_empty() || source.len() > 100 {
            return json!({"enabled":false,"commands":[]}).to_string();
        }
        let before = self.status();
        let mut s = self.state.lock().unwrap_or_else(|p| p.into_inner());
        s.connections.retain(|_, c| c.seen.elapsed() < FRESH);
        if payload["disconnect"] == true {
            s.connections.remove(source);
        } else if s.enabled {
            let browser = payload["browser"]
                .as_str()
                .filter(|b| ["Chrome", "Edge"].contains(b))
                .unwrap_or("Chrome");
            let videos = payload["videos"]
                .as_array()
                .into_iter()
                .flatten()
                .take(20)
                .filter_map(|v| {
                    let id = v["videoId"].as_str()?;
                    let seconds = v["seconds"].as_f64()?;
                    let tab = v["tabId"].as_u64().filter(|t| *t <= u32::MAX as u64)?;
                    if !valid_id(id) || !seconds.is_finite() || !(0.0..=604800.0).contains(&seconds)
                    {
                        return None;
                    }
                    let service = if v["service"] == "browser" { "browser" } else { "youtube" };
                    let url = if service == "youtube" { format!("https://www.youtube.com/watch?v={id}") } else {
                        let mut u = url::Url::parse(v["url"].as_str()?).ok()?;
                        if u.scheme() != "https" || !u.username().is_empty() || u.password().is_some() { return None; }
                        u.set_query(None); u.set_fragment(None); u.to_string()
                    };
                    Some((
                        YouTubeVideo {
                            service: service.into(), url,
                            source_id: source.into(),
                            tab_id: tab as u32,
                            video_id: id.into(),
                            title: v["title"]
                                .as_str()
                                .unwrap_or("YouTube")
                                .chars()
                                .filter(|c| !c.is_control())
                                .take(240)
                                .collect(),
                            browser: browser.into(),
                            seconds,
                            duration: v["duration"].as_f64().filter(|d| d.is_finite() && *d > 0.0 && *d <= 604800.0),
                            playing: v["playing"] == true,
                            caption: v["caption"].as_str().unwrap_or("").chars().filter(|c| !c.is_control() || *c == '\n').take(2000).collect(),
                        },
                        v["active"] == true,
                    ))
                })
                .collect();
            let c = s.connections.entry(source.into()).or_insert(Connection {
                seen: Instant::now(),
                videos: vec![],
                commands: vec![],
            });
            c.seen = Instant::now();
            c.videos = videos;
        }
        let commands = s
            .connections
            .get_mut(source)
            .map(|c| std::mem::take(&mut c.commands))
            .unwrap_or_default();
        let enabled = s.enabled;
        drop(s);
        if payload["action"]["type"] == "open" {
            if let (Some(id), Some(destination)) = (payload["action"]["videoId"].as_str(), payload["action"]["destination"].as_str()) {
                let _ = self.open_at(source, id, destination);
            }
        }
        let after = self.status();
        if before.connected != after.connected || before.detected != after.detected {
            self.bus.publish(Event::YouTubeChanged);
        }
        json!({"enabled":enabled,"commands":commands}).to_string()
    }
    pub fn open(&self, source: &str, video: &str) -> Result<YouTubeVideo, CoreError> {
        self.open_at(source, video, "notch")
    }
    pub fn open_at(&self, source: &str, video: &str, destination: &str) -> Result<YouTubeVideo, CoreError> {
        if !["notch", "floating"].contains(&destination) { return Err(CoreError::Io("Elige notch o ventana.".into())); }
        let found = self
            .status()
            .detected
            .filter(|v| v.source_id == source && v.video_id == video && v.service == "youtube")
            .ok_or_else(|| {
                CoreError::Io(
                    "El video cambió o el navegador se desconectó. Vuelve a intentarlo.".into(),
                )
            })?;
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        state.viewer = Some(found.clone()); state.destination = destination.into(); drop(state);
        self.bus.publish(Event::YouTubeChanged);
        Ok(found)
    }
    pub fn move_viewer(&self, destination: &str) -> Result<(), CoreError> {
        if !["notch", "floating"].contains(&destination) { return Err(CoreError::Io("Elige notch o ventana.".into())); }
        let mut s = self.state.lock().unwrap_or_else(|p| p.into_inner());
        if s.viewer.is_none() { return Err(CoreError::Io("Abre primero un video.".into())); }
        s.destination = destination.into(); drop(s);
        self.bus.publish(Event::YouTubeChanged); Ok(())
    }
    pub fn browser_pip(&self, source: &str, video: &str) -> Result<(), CoreError> {
        let v = self.status().detected.filter(|v| v.source_id == source && v.video_id == video)
            .ok_or_else(|| CoreError::Io("El video cambió.".into()))?;
        let mut s = self.state.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(c) = s.connections.get_mut(source) { c.commands.push(json!({"type":"pip","tabId":v.tab_id,"videoId":v.video_id})); }
        Ok(())
    }
    pub fn position(&self, source: &str, video: &str, seconds: f64) {
        if !seconds.is_finite() || !(0.0..=604800.0).contains(&seconds) { return; }
        let mut s = self.state.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(v) = s.viewer.as_mut().filter(|v| v.source_id == source && v.video_id == video) {
            v.seconds = seconds; v.caption.clear();
        }
    }
    pub fn context(&self) -> Option<YouTubeVideo> {
        let status = self.status(); status.viewer.or(status.detected)
    }
    pub fn toggle(&self, source: &str, video: &str) -> Result<(), CoreError> {
        let found = self.status().detected.filter(|v| v.source_id == source && v.video_id == video)
            .ok_or_else(|| CoreError::Io("El video cambió. Vuelve a intentarlo.".into()))?;
        let mut s = self.state.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(c) = s.connections.get_mut(source) {
            let command = json!({"type":"toggle","tabId":found.tab_id,"videoId":found.video_id});
            if !c.commands.contains(&command) { c.commands.push(command); }
        }
        Ok(())
    }
    /// Only called when the official embedded player reports PLAYING, never merely when the user clicks.
    pub fn started(&self, source: &str, video: &str) {
        let mut s = self.state.lock().unwrap_or_else(|p| p.into_inner());
        let Some(v) = s
            .viewer
            .clone()
            .filter(|v| v.source_id == source && v.video_id == video)
        else {
            return;
        };
        if let Some(c) = s.connections.get_mut(source) {
            let command = json!({"type":"pause","tabId":v.tab_id,"videoId":v.video_id});
            if !c.commands.contains(&command) {
                c.commands.push(command);
            }
        }
    }
    pub fn close(&self) {
        self.state.lock().unwrap_or_else(|p| p.into_inner()).viewer = None;
        self.bus.publish(Event::YouTubeChanged);
    }
}

pub fn valid_id(id: &str) -> bool {
    id.len() == 11
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// All extension files are compiled into the core, so both installers contain the same version.
pub fn prepare(data: &Path, relay: &Path, browser: &str) -> Result<PathBuf, CoreError> {
    if !["chrome", "edge"].contains(&browser) {
        return Err(CoreError::Io("Elige Chrome o Edge.".into()));
    }
    if !relay.is_file() {
        return Err(CoreError::Io(
            "Falta el componente incluido buddy-hook. Reinstala o recompila Buddy.".into(),
        ));
    }
    let dir = data.join("youtube-extension");
    std::fs::create_dir_all(&dir)?;
    for (name, text) in [
        (
            "manifest.json",
            include_str!("../../extensions/youtube/manifest.json"),
        ),
        (
            "background.js",
            include_str!("../../extensions/youtube/background.js"),
        ),
        (
            "content.js",
            include_str!("../../extensions/youtube/content.js"),
        ),
        (
            "popup.html",
            include_str!("../../extensions/youtube/popup.html"),
        ),
        ("browser-video.js", include_str!("../../extensions/youtube/browser-video.js")),
        (
            "popup.js",
            include_str!("../../extensions/youtube/popup.js"),
        ),
    ] {
        std::fs::write(dir.join(name), text)?;
    }
    for (name, bytes) in [
        ("icon-32.png", include_bytes!("../../extensions/youtube/icon-32.png").as_slice()),
        ("icon-64.png", include_bytes!("../../extensions/youtube/icon-64.png").as_slice()),
        ("icon-128.png", include_bytes!("../../extensions/youtube/icon-128.png").as_slice()),
    ] { std::fs::write(dir.join(name), bytes)?; }
    let host = relay.with_file_name(if cfg!(windows) {
        "buddy-youtube-host.exe"
    } else {
        "buddy-youtube-host"
    });
    // Same bundled executable and signature, with a distinct name selecting native-messaging mode.
    if std::fs::read(&host).ok() != Some(std::fs::read(relay)?) {
        std::fs::copy(relay, &host)?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&host, std::fs::Permissions::from_mode(0o755))?;
    }
    let manifest = json!({"name":HOST,"description":"Buddy · YouTube en el notch","path":host,"type":"stdio",
        "allowed_origins":[format!("chrome-extension://{}/", EXTENSION_ID.trim())]});
    #[cfg(target_os = "macos")]
    let destination = {
        let home = std::env::var_os("HOME")
            .ok_or_else(|| CoreError::Io("No se encontró tu carpeta de usuario.".into()))?;
        PathBuf::from(home)
            .join(if browser == "chrome" {
                "Library/Application Support/Google/Chrome/NativeMessagingHosts"
            } else {
                "Library/Application Support/Microsoft Edge/NativeMessagingHosts"
            })
            .join(format!("{HOST}.json"))
    };
    #[cfg(not(target_os = "macos"))]
    let destination = data.join(format!("{HOST}.json"));
    if let Some(parent) = destination.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(
        &destination,
        serde_json::to_vec_pretty(&manifest).map_err(|e| CoreError::Io(e.to_string()))?,
    )?;
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let product = if browser == "chrome" {
            "Google\\Chrome"
        } else {
            "Microsoft\\Edge"
        };
        let status = std::process::Command::new("reg.exe")
            .args([
                "ADD",
                &format!("HKCU\\Software\\{product}\\NativeMessagingHosts\\{HOST}"),
                "/ve",
                "/t",
                "REG_SZ",
                "/d",
            ])
            .arg(&destination)
            .arg("/f")
            .creation_flags(0x08000000)
            .status()?;
        if !status.success() {
            return Err(CoreError::Io(
                "No se pudo registrar la conexión con el navegador.".into(),
            ));
        }
    }
    Ok(dir)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn snapshot(source: &str, id: &str, active: bool) -> Value {
        json!({"sourceId":source,"browser":"Chrome","videos":[{"tabId":7,"videoId":id,"title":"Video de prueba","seconds":42.5,"playing":true,"active":active}]})
    }
    #[test]
    fn rejects_invalid_media_and_never_pauses_before_playback() {
        let y = YouTube::new(Arc::new(EventBus::default()));
        y.enable(true);
        y.receive(&snapshot("one", "invalid", true));
        assert!(y.status().detected.is_none());
        y.receive(&snapshot("one", "abcdefghijk", true));
        let v = y.open("one", "abcdefghijk").unwrap();
        assert_eq!(v.seconds, 42.5);
        assert_eq!(
            serde_json::from_str::<Value>(&y.receive(&snapshot("one", "abcdefghijk", true)))
                .unwrap()["commands"],
            json!([])
        );
        y.started("one", "differentid");
        y.started("one", "abcdefghijk");
        let reply: Value =
            serde_json::from_str(&y.receive(&snapshot("one", "abcdefghijk", true))).unwrap();
        assert_eq!(
            reply["commands"][0],
            json!({"type":"pause","tabId":7,"videoId":"abcdefghijk"})
        );
    }
    #[test]
    fn active_tab_wins_and_viewer_survives_a_new_detection() {
        let y = YouTube::new(Arc::new(EventBus::default()));
        y.enable(true);
        y.receive(&snapshot("one", "abcdefghijk", true));
        y.open("one", "abcdefghijk").unwrap();
        y.receive(&snapshot("two", "lmnopqrstuv", false));
        assert_eq!(y.status().detected.unwrap().source_id, "one");
        y.receive(&snapshot("one", "123456789ab", true));
        assert_eq!(y.status().viewer.unwrap().video_id, "abcdefghijk");
        assert!(y.open("one", "abcdefghijk").is_err());
        y.enable(false);
        assert!(!y.status().connected);
        assert!(y.status().viewer.is_none());
    }
    #[test]
    fn expired_sources_and_nonfinite_positions_are_not_offered() {
        let y = YouTube::new(Arc::new(EventBus::default()));
        y.enable(true);
        y.receive(&snapshot("one", "abcdefghijk", true));
        y.state
            .lock()
            .unwrap()
            .connections
            .get_mut("one")
            .unwrap()
            .seen = Instant::now() - FRESH;
        assert!(!y.status().connected);
        assert!(y.status().detected.is_none());
        let mut v = snapshot("two", "abcdefghijk", true);
        v["videos"][0]["seconds"] = json!(-1);
        y.receive(&v);
        assert!(y.status().detected.is_none());
    }
    #[test]
    fn duration_is_optional_and_toggle_is_scoped_to_the_current_video() {
        let y = YouTube::new(Arc::new(EventBus::default())); y.enable(true);
        let mut payload = snapshot("one", "abcdefghijk", true);
        y.receive(&payload); assert_eq!(y.status().detected.unwrap().duration, None);
        payload["videos"][0]["duration"] = json!(120.0); y.receive(&payload);
        assert_eq!(y.status().detected.unwrap().duration, Some(120.0));
        assert!(y.toggle("one", "lmnopqrstuv").is_err());
        y.toggle("one", "abcdefghijk").unwrap(); y.toggle("one", "abcdefghijk").unwrap();
        let reply: Value = serde_json::from_str(&y.receive(&payload)).unwrap();
        assert_eq!(reply["commands"], json!([{"type":"toggle","tabId":7,"videoId":"abcdefghijk"}]));
        payload["videos"][0]["duration"] = json!(-1); y.receive(&payload);
        assert_eq!(y.status().detected.unwrap().duration, None);
    }
    #[test]
    fn floating_move_preserves_selected_video_and_current_position() {
        let y = YouTube::new(Arc::new(EventBus::default())); y.enable(true);
        let mut payload = snapshot("one", "abcdefghijk", true);
        payload["videos"][0]["caption"] = json!("Subtítulo visible"); y.receive(&payload);
        y.open_at("one", "abcdefghijk", "floating").unwrap();
        assert_eq!(y.status().destination, "floating");
        y.position("one", "wrong_video", 200.0); assert_eq!(y.context().unwrap().seconds, 42.5);
        y.position("one", "abcdefghijk", f64::NAN); assert_eq!(y.context().unwrap().seconds, 42.5);
        y.position("one", "abcdefghijk", 81.0); assert!(y.context().unwrap().caption.is_empty());
        y.receive(&snapshot("one", "lmnopqrstuv", true));
        y.move_viewer("notch").unwrap(); assert_eq!(y.context().unwrap().video_id, "abcdefghijk");
        assert_eq!(y.context().unwrap().seconds, 81.0);
        assert!(y.move_viewer("anything").is_err()); y.close(); assert!(y.move_viewer("floating").is_err());
    }
    #[test]
    fn extension_actions_open_only_a_valid_detected_youtube_video() {
        let y = YouTube::new(Arc::new(EventBus::default())); y.enable(true);
        let mut payload = snapshot("one", "abcdefghijk", true);
        payload["action"] = json!({"type":"open","videoId":"abcdefghijk","destination":"floating"});
        y.receive(&payload); assert_eq!(y.status().destination, "floating");
        assert_eq!(y.status().viewer.unwrap().video_id, "abcdefghijk");
        payload["action"]["videoId"] = json!("lmnopqrstuv"); y.receive(&payload);
        assert_eq!(y.status().viewer.unwrap().video_id, "abcdefghijk");
        y.enable(false); y.receive(&payload); assert!(y.status().viewer.is_none());
    }
    #[test]
    fn browser_media_keeps_credentials_out_and_cannot_be_embedded() {
        let y = YouTube::new(Arc::new(EventBus::default())); y.enable(true);
        let mut payload = snapshot("one", "abcdefghijk", true);
        payload["videos"][0]["service"] = json!("browser");
        payload["videos"][0]["url"] = json!("https://video.example/watch/demo?token=private#state");
        payload["videos"][0]["caption"] = json!("x".repeat(3000)); y.receive(&payload);
        let v = y.context().unwrap(); assert_eq!(v.url, "https://video.example/watch/demo"); assert_eq!(v.caption.len(), 2000);
        assert!(y.open_at("one", "abcdefghijk", "floating").is_err());
        y.browser_pip("one", "abcdefghijk").unwrap();
        let response: Value = serde_json::from_str(&y.receive(&payload)).unwrap(); assert_eq!(response["commands"][0]["type"], "pip");
        payload["videos"][0]["url"] = json!("https://user:secret@video.example/watch"); y.receive(&payload); assert!(y.context().is_none());
    }
    #[cfg(unix)]
    #[test]
    fn browser_messages_round_trip_on_the_private_channel() {
        use std::io::{BufRead, BufReader, Write};
        let dir = tempfile::tempdir().unwrap();
        let hub = Arc::new(crate::sessions::SessionHub::new(
            dir.path().to_owned(),
            Arc::new(EventBus::default()),
        ));
        hub.youtube.enable(true);
        hub.start("").unwrap();
        let mut stream =
            std::os::unix::net::UnixStream::connect(dir.path().join("hooks.sock")).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut payload = snapshot("native-test", "abcdefghijk", true);
        payload["_youtube"] = json!(true);
        writeln!(stream, "{payload}").unwrap();
        let mut reply = String::new();
        BufReader::new(stream).read_line(&mut reply).unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&reply).unwrap()["enabled"],
            true
        );
        assert_eq!(
            hub.youtube.status().detected.unwrap().video_id,
            "abcdefghijk"
        );
    }
}
