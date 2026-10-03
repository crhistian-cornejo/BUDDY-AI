//! Buddy's tools beside Office: the music player and the skills.
//!
//! The music tools ask Buddy over its socket (`_app` + this run's secret from `BUDDY_GATE_TOKEN`); Buddy checks
//! the request and its app presses the button. They are offered only when the secret is in the environment, so a
//! server started by hand has none. `use_skill` reads `<skills>/<name>/SKILL.md` and nothing else.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

pub const MEDIA_CONTROL: &str = "media_control";
pub const MEDIA_PLAY: &str = "media_play";
pub const MEDIA_SEARCH: &str = "media_search";
pub const SPOTIFY_SEARCH: &str = "spotify_search";
pub const NOW_PLAYING: &str = "now_playing";
pub const USE_SKILL: &str = "use_skill";
const MAX_SKILL: u64 = 64 * 1024;

/// What the extra tools need: the skills folder and, for the music, the secret.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Extra {
    pub skills: Option<PathBuf>,
    pub token: Option<String>,
}

impl Extra {
    pub fn names(&self) -> Vec<&'static str> {
        let mut names = Vec::new();
        if self.token.is_some() {
            names.extend([SPOTIFY_SEARCH, MEDIA_PLAY, MEDIA_CONTROL, MEDIA_SEARCH, NOW_PLAYING]);
        }
        if self.skills.is_some() {
            names.push(USE_SKILL);
        }
        names
    }

    pub fn specs(&self) -> Vec<Value> {
        let safe = json!({ "readOnlyHint": false, "destructiveHint": false, "idempotentHint": false, "openWorldHint": false });
        let read = json!({ "readOnlyHint": true, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false });
        self.names()
            .into_iter()
            .map(|name| match name {
                MEDIA_CONTROL => json!({
                    "name": name,
                    "title": "Botones del reproductor",
                    "description": "Pulsa un botón de Spotify o Música en el ordenador del usuario.",
                    "inputSchema": { "type": "object", "properties": {
                        "action": { "type": "string", "enum": ["play", "pause", "toggle", "next", "previous"] }
                    }, "required": ["action"], "additionalProperties": false },
                    "annotations": safe,
                }),
                MEDIA_PLAY => json!({
                    "name": name,
                    "title": "Poner en Spotify",
                    "description": "Reproduce en Spotify una canción, disco, lista, artista o episodio por su enlace \
(https://open.spotify.com/... o spotify:...). Busca el enlace antes; nunca lo inventes.",
                    "inputSchema": { "type": "object", "properties": {
                        "link": { "type": "string", "description": "Enlace de open.spotify.com o URI spotify:" }
                    }, "required": ["link"], "additionalProperties": false },
                    "annotations": safe,
                }),
                SPOTIFY_SEARCH => json!({
                    "name": name,
                    "title": "Buscar en el catálogo de Spotify",
                    "description": "Busca dentro de Spotify (no en la web) y devuelve hasta 5 resultados con su enlace \
spotify:, listos para media_play. Con nuevo=true trae discos salidos en las últimas dos semanas.",
                    "inputSchema": { "type": "object", "properties": {
                        "query": { "type": "string", "description": "Canción, artista, disco o lista (puede ir vacío con nuevo=true)" },
                        "kind": { "type": "string", "enum": ["track", "album", "artist", "playlist"] },
                        "nuevo": { "type": "boolean", "description": "Solo lanzamientos recientes (discos)" }
                    }, "required": ["query"], "additionalProperties": false },
                    "annotations": read,
                }),
                MEDIA_SEARCH => json!({
                    "name": name,
                    "title": "Buscar en Spotify",
                    "description": "Abre en Spotify la búsqueda de lo que pide el usuario, para que elija y le dé a \
reproducir. Úsala solo si no encontraste un enlace de open.spotify.com.",
                    "inputSchema": { "type": "object", "properties": {
                        "query": { "type": "string", "description": "Qué buscar: canción, artista, disco…" }
                    }, "required": ["query"], "additionalProperties": false },
                    "annotations": safe,
                }),
                NOW_PLAYING => json!({
                    "name": name,
                    "title": "Qué suena",
                    "description": "Dice qué suena ahora en Spotify o Música y si está en pausa.",
                    "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false },
                    "annotations": read,
                }),
                _ => json!({
                    "name": name,
                    "title": "Usar una habilidad",
                    "description": "Carga las instrucciones completas de una de las habilidades (skills) de Buddy.",
                    "inputSchema": { "type": "object", "properties": {
                        "name": { "type": "string", "description": "Nombre de la habilidad" }
                    }, "required": ["name"], "additionalProperties": false },
                    "annotations": read,
                }),
            })
            .collect()
    }

    /// Runs one of the extra tools: the text for the agent, or why it failed.
    pub fn run(&self, name: &str, args: &Value) -> Result<String, String> {
        match name {
            USE_SKILL => read_skill(self.skills.as_deref().ok_or("No hay habilidades.")?, args["name"].as_str().unwrap_or("")),
            _ => {
                let token = self.token.as_deref().ok_or("El reproductor no está disponible.")?;
                let request = match name {
                    MEDIA_CONTROL => json!({ "request": "media", "action": args["action"].as_str().unwrap_or("") }),
                    MEDIA_PLAY => json!({ "request": "media", "action": "open", "uri": args["link"].as_str().unwrap_or("") }),
                    MEDIA_SEARCH => json!({ "request": "media", "action": "search", "query": args["query"].as_str().unwrap_or("") }),
                    SPOTIFY_SEARCH => json!({
                        "request": "spotify_search",
                        "query": args["query"].as_str().unwrap_or(""),
                        "kind": args["kind"].as_str().unwrap_or("track"),
                        "new": args["nuevo"] == true,
                    }),
                    _ => json!({ "request": "now_playing" }),
                };
                ask_buddy(request, token)
            }
        }
    }
}

/// One request to Buddy; its JSON reply (`ok`, `text`) becomes the tool's result.
fn ask_buddy(mut request: Value, token: &str) -> Result<String, String> {
    request["_app"] = Value::String(token.into());
    let reply = crate::talk(&format!("{request}\n"), true).ok_or("Buddy no responde (¿está abierto?).")?;
    let reply: Value = serde_json::from_str(&reply).map_err(|_| "Respuesta extraña de Buddy.".to_string())?;
    let text = reply["text"].as_str().unwrap_or("").to_string();
    if reply["ok"] == true { Ok(text) } else { Err(text) }
}

/// `<skills>/<name>/SKILL.md`, only for a plain name and only inside the folder (no links out).
fn read_skill(root: &Path, name: &str) -> Result<String, String> {
    let valid = !name.is_empty()
        && name.len() <= 64
        && name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && !name.starts_with('-');
    if !valid {
        return Err(format!("No existe la habilidad «{name}»."));
    }
    let missing = || format!("No existe la habilidad «{name}».");
    let root = root.canonicalize().map_err(|_| missing())?;
    let file = root.join(name).join("SKILL.md").canonicalize().map_err(|_| missing())?;
    if !file.starts_with(&root) || std::fs::metadata(&file).map(|m| m.len() > MAX_SKILL).unwrap_or(true) {
        return Err(missing());
    }
    std::fs::read_to_string(&file).map_err(|_| missing())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn music_tools_only_with_the_secret_and_skills_only_with_a_folder() {
        assert!(Extra::default().names().is_empty());
        let both = Extra { skills: Some("/s".into()), token: Some("t".into()) };
        assert_eq!(both.names(), [SPOTIFY_SEARCH, MEDIA_PLAY, MEDIA_CONTROL, MEDIA_SEARCH, NOW_PLAYING, USE_SKILL]);
        assert_eq!(both.specs().len(), 6);
    }

    #[test]
    fn a_skill_is_read_only_from_its_folder() {
        let tmp = crate::office::test_support::TempDir::new("skills");
        let skills = tmp.0.join("skills");
        std::fs::create_dir_all(skills.join("spotify")).unwrap();
        std::fs::write(skills.join("spotify/SKILL.md"), "---\nname: spotify\n---\npasos").unwrap();
        std::fs::write(tmp.0.join("secreto.md"), "no").unwrap();
        let extra = Extra { skills: Some(skills.clone()), token: None };
        assert!(extra.run(USE_SKILL, &json!({ "name": "spotify" })).unwrap().contains("pasos"));
        assert!(extra.run(USE_SKILL, &json!({ "name": "../secreto" })).is_err());
        assert!(extra.run(USE_SKILL, &json!({ "name": "nadie" })).is_err());
        #[cfg(unix)]
        {
            std::fs::create_dir_all(skills.join("fuera")).unwrap();
            std::os::unix::fs::symlink(tmp.0.join("secreto.md"), skills.join("fuera/SKILL.md")).unwrap();
            assert!(extra.run(USE_SKILL, &json!({ "name": "fuera" })).is_err(), "a link out of the folder is not followed");
        }
        assert!(extra.run(MEDIA_CONTROL, &json!({ "action": "next" })).is_err(), "no secret, no music");
    }
}
