//! The music player as one of Buddy's tools: the agents ask through the relay (`buddy-hook --mcp`), the core checks
//! the request and the app does it — the app already holds the Apple Events permission (Mac) or the system media
//! session (Windows). Nothing here plays anything: it validates and announces `Event::MediaCommand`.

/// What a player says is playing, as the app last saw it.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct NowPlayingInfo {
    pub title: String,
    pub artist: String,
    /// "Spotify", "Música"…
    pub app: String,
    pub playing: bool,
}

/// The buttons an agent may press.
pub const ACTIONS: [&str; 5] = ["play", "pause", "toggle", "next", "previous"];

/// Kinds of Spotify item that can be played by their link.
const KINDS: [&str; 6] = ["track", "album", "playlist", "artist", "episode", "show"];

/// A Spotify link the agent found (`spotify:track:<id>` or `https://open.spotify.com/[intl-xx/]track/<id>?si=…`)
/// as a clean `spotify:<kind>:<id>` URI. Anything else (other sites, other schemes, odd ids) is `None`.
pub fn spotify_uri(input: &str) -> Option<String> {
    let input = input.trim();
    let (kind, id) = if let Some(rest) = input.strip_prefix("spotify:") {
        rest.split_once(':')?
    } else {
        let rest = input.strip_prefix("https://open.spotify.com/")?;
        let path = rest.split(['?', '#']).next()?;
        let mut parts = path.split('/').filter(|p| !p.is_empty());
        let mut kind = parts.next()?;
        if kind.starts_with("intl-") {
            kind = parts.next()?;
        }
        (kind, parts.next()?)
    };
    let valid_id = id.len() == 22 && id.chars().all(|c| c.is_ascii_alphanumeric());
    (KINDS.contains(&kind) && valid_id).then(|| format!("spotify:{kind}:{id}"))
}

/// A search inside Spotify for when no link was found: `spotify:search:<query>` (percent-encoded, at most 100
/// characters). Spotify shows the results; the user picks and presses play.
pub fn search_uri(query: &str) -> Option<String> {
    let query: String = query.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(100).collect();
    if query.is_empty() {
        return None;
    }
    let encoded: String = query
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect();
    Some(format!("spotify:search:{encoded}"))
}

/// One line for the agent about what is playing.
pub fn describe(now: Option<&NowPlayingInfo>) -> String {
    match now {
        Some(n) if !n.title.is_empty() => {
            let state = if n.playing { "sonando" } else { "en pausa" };
            let by = if n.artist.is_empty() { String::new() } else { format!(" de {}", n.artist) };
            format!("«{}»{by} en {} ({state}).", n.title, n.app)
        }
        _ => "No suena nada (Spotify o Música están cerrados o parados).".into(),
    }
}

/// What the agent reads back after asking for `action`.
pub fn done_text(action: &str) -> &'static str {
    match action {
        "play" => "Listo: a sonar.",
        "pause" => "Listo: en pausa.",
        "toggle" => "Listo: reproducir/pausa.",
        "next" => "Listo: siguiente canción.",
        "previous" => "Listo: canción anterior.",
        "search" => "Abrí la búsqueda en Spotify: el usuario elige y le da a reproducir.",
        _ => "Listo: abriendo en Spotify. Comprueba con now_playing en unos segundos.",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_become_clean_uris_and_nothing_else_passes() {
        let id = "7hQJA50XrCWABAu5v6QZ4i";
        assert_eq!(spotify_uri(&format!("spotify:track:{id}")).as_deref(), Some("spotify:track:7hQJA50XrCWABAu5v6QZ4i"));
        assert_eq!(
            spotify_uri(&format!("https://open.spotify.com/intl-es/album/{id}?si=abc")).as_deref(),
            Some("spotify:album:7hQJA50XrCWABAu5v6QZ4i")
        );
        assert_eq!(spotify_uri(&format!("https://open.spotify.com/playlist/{id}")).as_deref(), Some("spotify:playlist:7hQJA50XrCWABAu5v6QZ4i"));
        assert_eq!(spotify_uri(&format!("https://evil.example/track/{id}")), None);
        assert_eq!(spotify_uri(&format!("http://open.spotify.com/track/{id}")), None);
        assert_eq!(spotify_uri("spotify:track:corto"), None);
        assert_eq!(spotify_uri(&format!("spotify:user:{id}")), None);
        assert_eq!(spotify_uri(&format!("spotify:track:{id}\" & do shell script \"x")), None, "never reaches AppleScript");
    }

    #[test]
    fn a_search_is_encoded_and_bounded() {
        assert_eq!(search_uri("  Miranda   Lambert Crisco ").as_deref(), Some("spotify:search:Miranda%20Lambert%20Crisco"));
        assert_eq!(search_uri("a\" & b").as_deref(), Some("spotify:search:a%22%20%26%20b"));
        assert_eq!(search_uri("   "), None);
        assert!(search_uri(&"x".repeat(500)).unwrap().len() < 130);
    }

    #[test]
    fn describes_what_plays() {
        let n = NowPlayingInfo { title: "Creep".into(), artist: "Radiohead".into(), app: "Spotify".into(), playing: true };
        assert_eq!(describe(Some(&n)), "«Creep» de Radiohead en Spotify (sonando).");
        assert!(describe(None).starts_with("No suena nada"));
    }
}
