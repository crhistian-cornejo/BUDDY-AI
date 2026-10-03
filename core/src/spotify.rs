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
        if let Some((token, until)) = cached.as_ref() {
            if Instant::now() < *until {
                return Some(Ok(token.clone()));
            }
        }
        let secret = secrets.get()?;
        Some(fetch_token(id, &secret).map(|(token, until)| {
            *cached = Some((token.clone(), until));
            token
        }))
    }
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
        .query("limit", &RESULTS.to_string())
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
