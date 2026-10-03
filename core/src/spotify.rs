//! Spotify's catalogue for the agents: a search inside Spotify (never the web), so "pon X" finds the real item and
//! costs a few lines of text instead of a web search. Playing stays local (AppleScript / the system's media
//! controls, see `media`); this module only searches.
//!
//! It uses the user's own Spotify app (developer.spotify.com, "Client Credentials"): the Client ID lives in the
//! settings, the Client Secret only in the Keychain / Credential Manager. The app token (one hour) stays in memory.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::CoreError;

pub const CLIENT_ID_KEY: &str = "spotify.client_id";
const KEYCHAIN_SERVICE: &str = "io.github.crhistian-cornejo.buddy";
const KEYCHAIN_ACCOUNT: &str = "spotify-client-secret";
const TIMEOUT: Duration = Duration::from_secs(10);
/// Results the agent reads, at most.
const RESULTS: usize = 5;
pub const KINDS: [&str; 4] = ["track", "album", "artist", "playlist"];

#[derive(Default)]
pub struct Spotify {
    token: Mutex<Option<(String, Instant)>>,
    /// The user's own token (playlists), refreshed from the refresh token in the keychain.
    user_token: Mutex<Option<(String, Instant)>>,
}

/// Where Spotify sends the user back after they allow playlists: this machine only. It is the redirect URI the
/// setup steps ask for in the user's Spotify app.
pub const REDIRECT: &str = "http://127.0.0.1:8888";
const REDIRECT_PORT: u16 = 8888;
/// What Buddy asks the user for: to create and fill playlists. Nothing else of the account.
const SCOPES: &str = "playlist-modify-private playlist-modify-public";
const KEYCHAIN_USER: &str = "spotify-user-refresh";
/// Songs a playlist is made with, at most.
const MAX_TRACKS: usize = 60;

/// The user's refresh token (the Keychain / Credential Manager in the app; memory in tests).
pub struct UserSecrets;

impl Secrets for UserSecrets {
    fn get(&self) -> Option<String> {
        keyring::Entry::new(KEYCHAIN_SERVICE, KEYCHAIN_USER).ok()?.get_password().ok().filter(|s| !s.is_empty())
    }
    fn set(&self, secret: &str) -> Result<(), String> {
        keyring::Entry::new(KEYCHAIN_SERVICE, KEYCHAIN_USER).map_err(|e| e.to_string())?.set_password(secret).map_err(|e| e.to_string())
    }
    fn delete(&self) {
        if let Ok(entry) = keyring::Entry::new(KEYCHAIN_SERVICE, KEYCHAIN_USER) {
            let _ = entry.delete_credential();
        }
    }
}

/// Where the Client Secret is kept (the system's store in the app; memory in tests).
pub trait Secrets: Send + Sync {
    fn get(&self) -> Option<String>;
    fn set(&self, secret: &str) -> Result<(), String>;
    fn delete(&self);
}

/// The Keychain (Mac) or Credential Manager (Windows).
pub struct SystemSecrets;

impl SystemSecrets {
    fn entry() -> Option<keyring::Entry> {
        keyring::Entry::new(KEYCHAIN_SERVICE, KEYCHAIN_ACCOUNT).ok()
    }
}

impl Secrets for SystemSecrets {
    fn get(&self) -> Option<String> {
        Self::entry()?.get_password().ok().filter(|s| !s.is_empty())
    }
    fn set(&self, secret: &str) -> Result<(), String> {
        Self::entry().ok_or("No hay llavero disponible.")?.set_password(secret).map_err(|e| e.to_string())
    }
    fn delete(&self) {
        if let Some(entry) = Self::entry() {
            let _ = entry.delete_credential();
        }
    }
}

impl Spotify {
    /// Checks the pair against Spotify (one token request) before keeping anything.
    pub fn connect(&self, client_id: &str, secret: &str, secrets: &dyn Secrets) -> Result<(), String> {
        let (id, secret) = (client_id.trim(), secret.trim());
        if id.is_empty() || secret.is_empty() {
            return Err("Faltan el Client ID o el Client Secret.".into());
        }
        let token = fetch_token(id, secret)?;
        secrets.set(secret)?;
        *self.token.lock().unwrap_or_else(|p| p.into_inner()) = Some(token);
        Ok(())
    }

    pub fn forget(&self, secrets: &dyn Secrets) {
        secrets.delete();
        *self.token.lock().unwrap_or_else(|p| p.into_inner()) = None;
        *self.user_token.lock().unwrap_or_else(|p| p.into_inner()) = None;
    }

    /// Starts «permitir playlists»: listens on this machine for Spotify's answer and returns the page the user must
    /// open to allow it. `done` is told how it ended (within five minutes, or it gives up).
    pub fn authorize(self: &std::sync::Arc<Self>, client_id: &str, done: Box<dyn Fn(Result<(), String>) + Send>) -> Result<String, String> {
        let id = client_id.trim().to_string();
        if id.is_empty() {
            return Err("Primero conecta Spotify con tu Client ID y Client Secret.".into());
        }
        let listener = std::net::TcpListener::bind(("127.0.0.1", REDIRECT_PORT))
            .map_err(|_| format!("El puerto {REDIRECT_PORT} está ocupado por otro programa; ciérralo y vuelve a intentarlo."))?;
        let _ = listener.set_nonblocking(true);
        // Ties the answer to this request: another page on the machine cannot inject a code.
        let seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
        let state = format!("{:x}{:x}", seed, std::process::id());
        let url = format!(
            "https://accounts.spotify.com/authorize?response_type=code&client_id={}&scope={}&redirect_uri={}&state={state}",
            encode(&id),
            encode(SCOPES),
            encode(REDIRECT)
        );
        let me = self.clone();
        std::thread::spawn(move || {
            let result = wait_for_code(&listener, &state).and_then(|code| {
                let secret = SystemSecrets.get().ok_or("Falta el Client Secret en el llavero.")?;
                let (access, until, refresh) = exchange(&id, &secret, &[("grant_type", "authorization_code"), ("code", &code), ("redirect_uri", REDIRECT)])?;
                UserSecrets.set(&refresh.ok_or("Spotify no devolvió el permiso duradero.")?)?;
                *me.user_token.lock().unwrap_or_else(|p| p.into_inner()) = Some((access, until));
                Ok(())
            });
            done(result);
        });
        Ok(url)
    }

    /// The user allowed playlists on this machine.
    pub fn can_write(&self) -> bool {
        UserSecrets.get().is_some()
    }

    pub fn forget_user(&self) {
        UserSecrets.delete();
        *self.user_token.lock().unwrap_or_else(|p| p.into_inner()) = None;
    }

    fn user_access(&self, id: &str) -> Result<String, String> {
        let mut cached = self.user_token.lock().unwrap_or_else(|p| p.into_inner());
        if let Some((token, until)) = cached.as_ref()
            && Instant::now() < *until
        {
            return Ok(token.clone());
        }
        let needs = "Para crear playlists falta tu permiso: Ajustes › Conexiones › Spotify › «Permitir crear playlists».";
        let refresh = UserSecrets.get().ok_or(needs)?;
        let secret = SystemSecrets.get().ok_or(needs)?;
        let (access, until, renewed) = exchange(id, &secret, &[("grant_type", "refresh_token"), ("refresh_token", &refresh)])?;
        if let Some(renewed) = renewed {
            let _ = UserSecrets.set(&renewed);
        }
        *cached = Some((access.clone(), until));
        Ok(access)
    }

    /// Creates a private playlist in the user's account with `tracks` (each a `spotify:track:` link or «Artista -
    /// Canción», looked up in the catalogue). Says what it made, its link to play it, and what it did not find.
    pub fn create_playlist(&self, client_id: Option<String>, name: &str, description: &str, tracks: &[String]) -> Result<String, String> {
        let id = client_id.filter(|s| !s.is_empty()).ok_or("Spotify no está conectado en Ajustes › Conexiones.")?;
        let name: String = name.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(100).collect();
        if name.is_empty() {
            return Err("Falta el nombre de la playlist.".into());
        }
        if tracks.is_empty() {
            return Err("Faltan las canciones.".into());
        }
        let token = self.user_access(&id)?;
        let (mut uris, mut missing): (Vec<String>, Vec<String>) = (Vec::new(), Vec::new());
        for item in tracks.iter().take(MAX_TRACKS) {
            let item = item.trim();
            let found = match crate::media::spotify_uri(item) {
                Some(uri) if uri.starts_with("spotify:track:") => Some(uri),
                Some(_) => None,
                None => get_json(&token, &item.chars().take(160).collect::<String>(), "track").ok().and_then(|body| body["tracks"]["items"][0]["uri"].as_str().map(String::from)),
            };
            match found {
                Some(uri) if !uris.contains(&uri) => uris.push(uri),
                Some(_) => {}
                None => missing.push(item.chars().take(60).collect()),
            }
        }
        if uris.is_empty() {
            return Err("No encontré ninguna de esas canciones en Spotify.".into());
        }
        let body = serde_json::json!({ "name": name, "description": description.chars().take(280).collect::<String>(), "public": false });
        // The account's own playlists: the current endpoint first, the older one for accounts still on it.
        let created = match api(&token, "POST", "https://api.spotify.com/v1/me/playlists", Some(&body)) {
            Ok(v) => v,
            Err(_) => {
                let me = api(&token, "GET", "https://api.spotify.com/v1/me", None)?;
                let user = me["id"].as_str().ok_or("Spotify no dijo de quién es la cuenta.")?;
                api(&token, "POST", &format!("https://api.spotify.com/v1/users/{}/playlists", encode(user)), Some(&body))?
            }
        };
        let playlist = created["id"].as_str().ok_or("Spotify no devolvió la playlist.")?;
        let items = serde_json::json!({ "uris": uris });
        if api(&token, "POST", &format!("https://api.spotify.com/v1/playlists/{playlist}/items"), Some(&items)).is_err() {
            api(&token, "POST", &format!("https://api.spotify.com/v1/playlists/{playlist}/tracks"), Some(&items))?;
        }
        let mut out = format!("Playlist «{name}» creada con {} canciones → spotify:playlist:{playlist}", uris.len());
        if !missing.is_empty() {
            out.push_str(&format!("\nNo encontré: {}.", missing.join("; ")));
        }
        Ok(out)
    }

    /// Up to five results as short lines with their `spotify:` URI. `new` limits albums to the last two weeks.
    pub fn search(&self, client_id: Option<String>, secrets: &dyn Secrets, query: &str, kind: &str, new: bool) -> Result<String, String> {
        let not_connected = "Spotify no está conectado en Ajustes › Conexiones. Usa media_search para abrir la búsqueda en la app.";
        let id = client_id.filter(|s| !s.is_empty()).ok_or(not_connected)?;
        let kind = if new { "album" } else if KINDS.contains(&kind) { kind } else { "track" };
        let mut q: String = query.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(200).collect();
        if new {
            q = if q.is_empty() { "tag:new".into() } else { format!("{q} tag:new") };
        }
        if q.is_empty() {
            return Err("Falta qué buscar.".into());
        }
        let token = self.token(&id, secrets).ok_or(not_connected)??;
        let body = get_json(&token, &q, kind).inspect_err(|_| {
            // A refused token is dropped, so the next search asks for a fresh one.
            *self.token.lock().unwrap_or_else(|p| p.into_inner()) = None;
        })?;
        let lines = format_results(&body, kind);
        Ok(if lines.is_empty() { format!("Spotify no encontró nada para «{q}».") } else { lines })
    }

    /// The cached app token, or a new one (None: no secret stored).
    fn token(&self, id: &str, secrets: &dyn Secrets) -> Option<Result<String, String>> {
        let mut cached = self.token.lock().unwrap_or_else(|p| p.into_inner());
        if let Some((token, until)) = cached.as_ref()
            && Instant::now() < *until
        {
            return Some(Ok(token.clone()));
        }
        let secret = secrets.get()?;
        Some(fetch_token(id, &secret).map(|(token, until)| {
            *cached = Some((token.clone(), until));
            token
        }))
    }
}

/// Percent-encoding for a URL component.
fn encode(text: &str) -> String {
    text.bytes().map(|b| if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}

/// Waits (up to five minutes) for the browser to come back with `?code=…&state=…`, answers it with a short page,
/// and returns the code.
fn wait_for_code(listener: &std::net::TcpListener, state: &str) -> Result<String, String> {
    use std::io::{BufRead, BufReader, Write};
    let deadline = Instant::now() + Duration::from_secs(300);
    loop {
        match listener.accept() {
            Ok((mut stream, _)) => {
                let _ = stream.set_nonblocking(false);
                let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
                let mut line = String::new();
                let _ = BufReader::new(&stream).read_line(&mut line);
                // «GET /?code=…&state=… HTTP/1.1»
                let query = line.split_whitespace().nth(1).and_then(|path| path.split_once('?')).map(|(_, q)| q.to_string()).unwrap_or_default();
                let field = |name: &str| query.split('&').find_map(|pair| pair.strip_prefix(&format!("{name}=")).map(String::from));
                let outcome = match (field("code"), field("state"), field("error")) {
                    (Some(code), Some(got), _) if got == state && code.len() < 600 && code.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.')) => Some(Ok(code)),
                    (_, _, Some(_)) => Some(Err("No diste el permiso en Spotify.".to_string())),
                    // A favicon request, another program: keep waiting for the real answer.
                    _ => None,
                };
                let page = match &outcome {
                    Some(Ok(_)) => "Listo: Buddy ya puede crear playlists. Puedes cerrar esta pestaña.",
                    Some(Err(_)) => "No se dio el permiso. Puedes cerrar esta pestaña.",
                    None => "Buddy",
                };
                let body = format!("<!doctype html><meta charset=utf-8><title>Buddy</title><body style=\"font:16px system-ui;margin:3em\">{page}</body>");
                let _ = stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes());
                if let Some(outcome) = outcome {
                    return outcome;
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(200)),
            Err(_) => return Err("No se pudo recibir la respuesta de Spotify.".into()),
        }
        if Instant::now() > deadline {
            return Err("No llegó la respuesta de Spotify. Vuelve a intentarlo.".into());
        }
    }
}

/// A token request for the user: the access token, when it ends, and a refresh token when Spotify sends one.
fn exchange(id: &str, secret: &str, form: &[(&str, &str)]) -> Result<(String, Instant, Option<String>), String> {
    use base64::Engine;
    let basic = base64::engine::general_purpose::STANDARD.encode(format!("{id}:{secret}"));
    let mut response = agent()
        .post("https://accounts.spotify.com/api/token")
        .header("Authorization", &format!("Basic {basic}"))
        .send_form(form.iter().copied())
        .map_err(|e| format!("No se pudo hablar con Spotify: {e}"))?;
    let status = response.status().as_u16();
    let body: Value = response.body_mut().read_to_string().ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or(Value::Null);
    if status != 200 {
        return Err(if body["error"] == "invalid_grant" {
            "El permiso de Spotify caducó o se retiró: vuelve a darlo en Ajustes › Conexiones › Spotify.".into()
        } else {
            format!("Spotify respondió {status} al pedir el permiso.")
        });
    }
    let access = body["access_token"].as_str().ok_or("Spotify no devolvió un token.")?.to_string();
    let seconds = body["expires_in"].as_u64().unwrap_or(3600).saturating_sub(60);
    Ok((access, Instant::now() + Duration::from_secs(seconds), body["refresh_token"].as_str().map(String::from)))
}

/// One Web API call with the user's token; its JSON, or why Spotify refused.
fn api(token: &str, method: &str, url: &str, body: Option<&Value>) -> Result<Value, String> {
    let bearer = format!("Bearer {token}");
    let sent = match body {
        Some(body) => agent().post(url).header("Authorization", &bearer).header("Content-Type", "application/json").send(body.to_string()),
        None if method == "GET" => agent().get(url).header("Authorization", &bearer).call(),
        None => agent().post(url).header("Authorization", &bearer).send_empty(),
    };
    let mut response = sent.map_err(|e| format!("No se pudo hablar con Spotify: {e}"))?;
    let status = response.status().as_u16();
    let text = response.body_mut().read_to_string().unwrap_or_default();
    if !(200..300).contains(&status) {
        return Err(match status {
            401 => "Spotify caducó la sesión; vuelve a intentarlo.".into(),
            403 => "Spotify no lo permitió (falta el permiso de playlists, o la cuenta de la app no es Premium).".into(),
            429 => "Spotify pide esperar un poco.".into(),
            _ => format!("Spotify respondió {status}."),
        });
    }
    Ok(serde_json::from_str(&text).unwrap_or(Value::Null))
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder().timeout_global(Some(TIMEOUT)).http_status_as_error(false).build().into()
}

/// Client Credentials: a one-hour app token (kept a minute less).
fn fetch_token(id: &str, secret: &str) -> Result<(String, Instant), String> {
    use base64::Engine;
    let basic = base64::engine::general_purpose::STANDARD.encode(format!("{id}:{secret}"));
    let mut response = agent()
        .post("https://accounts.spotify.com/api/token")
        .header("Authorization", &format!("Basic {basic}"))
        .send_form([("grant_type", "client_credentials")])
        .map_err(|e| format!("No se pudo hablar con Spotify: {e}"))?;
    let status = response.status().as_u16();
    let body: Value = response.body_mut().read_to_string().ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or(Value::Null);
    if status != 200 {
        return Err(match status {
            400 | 401 => "Spotify no aceptó el Client ID o el Client Secret.".into(),
            _ => format!("Spotify respondió {status}."),
        });
    }
    let token = body["access_token"].as_str().ok_or("Spotify no devolvió un token.")?.to_string();
    let seconds = body["expires_in"].as_u64().unwrap_or(3600).saturating_sub(60);
    Ok((token, Instant::now() + Duration::from_secs(seconds)))
}

fn get_json(token: &str, q: &str, kind: &str) -> Result<Value, String> {
    let mut response = agent()
        .get("https://api.spotify.com/v1/search")
        .header("Authorization", &format!("Bearer {token}"))
        .query("q", q)
        .query("type", kind)
        .query("limit", RESULTS.to_string())
        .call()
        .map_err(|e| format!("No se pudo buscar en Spotify: {e}"))?;
    let status = response.status().as_u16();
    if status != 200 {
        return Err(match status {
            401 => "Spotify caducó la sesión; vuelve a intentarlo.".into(),
            403 => "Spotify rechazó la búsqueda (la app de desarrollador necesita Premium en su dueño).".into(),
            429 => "Spotify pide esperar un poco antes de buscar otra vez.".into(),
            _ => format!("Spotify respondió {status}."),
        });
    }
    let text = response.body_mut().read_to_string().map_err(|_| "Respuesta extraña de Spotify.".to_string())?;
    serde_json::from_str(&text).map_err(|_| "Respuesta extraña de Spotify.".to_string())
}

/// `1. «Creep» — Radiohead (Pablo Honey, 1993) → spotify:track:…`, one line per result.
pub fn format_results(body: &Value, kind: &str) -> String {
    let names = |v: &Value| -> String {
        v.as_array().into_iter().flatten().filter_map(|a| a["name"].as_str()).take(3).collect::<Vec<_>>().join(", ")
    };
    let year = |d: &Value| d.as_str().map(|d| d.chars().take(4).collect::<String>()).unwrap_or_default();
    body[format!("{kind}s")]["items"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|i| i.is_object())
        .filter_map(|i| {
            let uri = i["uri"].as_str()?;
            let name = i["name"].as_str()?;
            let detail = match kind {
                "track" => format!(" — {} ({}, {})", names(&i["artists"]), i["album"]["name"].as_str().unwrap_or(""), year(&i["album"]["release_date"])),
                "album" => format!(" — {} ({}, {} canciones)", names(&i["artists"]), i["release_date"].as_str().unwrap_or(""), i["total_tracks"]),
                "playlist" => format!(" — de {}", i["owner"]["display_name"].as_str().unwrap_or("?")),
                _ => String::new(),
            };
            Some(format!("«{name}»{detail} → {uri}"))
        })
        .take(RESULTS)
        .enumerate()
        .map(|(n, line)| format!("{}. {line}", n + 1))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The Client ID kept in the settings (not a secret).
pub fn client_id(store: &crate::store::Store) -> Result<Option<String>, CoreError> {
    Ok(store.setting(CLIENT_ID_KEY)?.filter(|s| !s.trim().is_empty()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    pub struct Memory(pub Mutex<Option<String>>);
    impl Secrets for Memory {
        fn get(&self) -> Option<String> {
            self.0.lock().unwrap().clone()
        }
        fn set(&self, s: &str) -> Result<(), String> {
            *self.0.lock().unwrap() = Some(s.into());
            Ok(())
        }
        fn delete(&self) {
            *self.0.lock().unwrap() = None;
        }
    }

    /// Reaches Spotify over HTTPS: `cargo test -p buddy-core spotify -- --ignored`.
    #[test]
    #[ignore]
    fn live_spotify_refuses_a_made_up_pair() {
        let err = fetch_token("inventado", "inventado").unwrap_err();
        assert!(err.contains("no aceptó"), "{err}");
    }

    #[test]
    fn the_browser_answer_is_taken_only_with_the_right_state() {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let _ = listener.set_nonblocking(true);
        let port = listener.local_addr().unwrap().port();
        let ask = move |path: &str| {
            let mut s = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
            s.write_all(format!("GET {path} HTTP/1.1\r\nHost: x\r\n\r\n").as_bytes()).unwrap();
            let mut page = String::new();
            let _ = s.read_to_string(&mut page);
            page
        };
        let browser = std::thread::spawn(move || {
            ask("/favicon.ico");
            ask("/?code=robado&state=otro");
            ask("/?code=AQB-x_1.z&state=abc123")
        });
        assert_eq!(wait_for_code(&listener, "abc123").unwrap(), "AQB-x_1.z");
        assert!(browser.join().unwrap().contains("ya puede crear playlists"));
        assert_eq!(encode("playlist-modify-private a/b"), "playlist-modify-private%20a%2Fb");
    }

    #[test]
    fn results_are_short_lines_with_their_uri() {
        let body = json!({ "tracks": { "items": [
            { "name": "Creep", "uri": "spotify:track:70LcF31zb1H0PyJoS1Sx1r",
              "artists": [{ "name": "Radiohead" }], "album": { "name": "Pablo Honey", "release_date": "1993-02-22" } },
            null
        ]}});
        assert_eq!(format_results(&body, "track"), "1. «Creep» — Radiohead (Pablo Honey, 1993) → spotify:track:70LcF31zb1H0PyJoS1Sx1r");
        let albums = json!({ "albums": { "items": [{ "name": "Crisco", "uri": "spotify:album:x", "release_date": "2026-10-02",
            "total_tracks": 12, "artists": [{ "name": "Miranda Lambert" }] }]}});
        assert_eq!(format_results(&albums, "album"), "1. «Crisco» — Miranda Lambert (2026-10-02, 12 canciones) → spotify:album:x");
        assert_eq!(format_results(&json!({}), "track"), "");
    }

    #[test]
    fn without_credentials_it_says_how_to_connect_and_never_calls_out() {
        let spotify = Spotify::default();
        let none = Memory(Mutex::new(None));
        let err = spotify.search(None, &none, "Creep", "track", false).unwrap_err();
        assert!(err.contains("Ajustes") && err.contains("media_search"));
        let err = spotify.search(Some("id".into()), &none, "Creep", "track", false).unwrap_err();
        assert!(err.contains("no está conectado"));
        assert!(spotify.connect(" ", "x", &none).is_err());
        assert!(none.get().is_none(), "nothing is kept before Spotify accepts the pair");
    }
}
