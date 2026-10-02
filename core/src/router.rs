//! Router: the cheapest model that does the job. Small talk (greetings, thanks, «ok») goes to the provider's light
//! model; everything else uses the agent's own. Specialists are never downgraded, and attachments always get the
//! agent's model. A setting turns it off. (Switching provider when a plan runs out lives in `chat`.)

use crate::providers::ProviderId;
use crate::store::fold;

/// Setting key; "false" turns the router off.
pub const SETTING: &str = "router.cheap";

/// Model and effort to use instead of the agent's, when the message is light.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Light {
    pub model: Option<String>,
    pub effort: Option<String>,
}

pub fn light_for(provider: ProviderId) -> Light {
    match provider {
        ProviderId::Claude => Light { model: Some("haiku".into()), effort: None },
        _ => Light { model: None, effort: Some("low".into()) },
    }
}

/// True for small talk: short, no question to research, made of greetings or acknowledgements.
pub fn is_small_talk(text: &str) -> bool {
    let t = fold(text.trim());
    if t.is_empty() || t.chars().count() > 40 || t.contains('\n') || t.chars().any(|c| c.is_ascii_digit()) {
        return false;
    }
    const WORDS: &[&str] = &[
        "hola", "holi", "buenas", "buenos dias", "buenas tardes", "buenas noches", "hey", "que tal", "como estas",
        "como vas", "gracias", "muchas gracias", "ok", "okey", "vale", "listo", "perfecto", "genial", "excelente",
        "chevere", "bacan", "jaja", "jeje", "adios", "chao", "nos vemos", "hasta luego", "bien", "de nada", "si", "no",
        "hello", "hi", "thanks", "thank you", "bye",
    ];
    let words: Vec<&str> = t
        .split(|c: char| !c.is_alphanumeric() && c != ' ')
        .flat_map(|chunk| chunk.split_whitespace())
        .collect();
    if words.is_empty() {
        return false;
    }
    let joined = words.join(" ");
    // Every word belongs to a greeting/acknowledgement (allowing "buddy" and "bro"-like fillers).
    words.iter().all(|w| {
        ["buddy", "bro", "brooo", "amigo", "mi", "y", "tu", "muy", "todo", "a", "pues"].contains(w)
            || w.starts_with("jaj")
            || w.starts_with("broo")
            || WORDS.iter().any(|g| g.split(' ').any(|p| p == *w))
    }) && WORDS.iter().any(|g| joined.contains(g))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_talk_is_recognised() {
        for t in ["hola", "Hola mi broooo", "¡Gracias!", "ok", "buenas tardes buddy", "jajaja", "¿Qué tal?", "chao"] {
            assert!(is_small_talk(t), "{t:?}");
        }
    }

    #[test]
    fn real_requests_keep_the_good_model() {
        for t in [
            "hola, ¿me explicas qué es SwiftUI?",
            "dame 3 tips de Rust",
            "¿quién gana el clásico?",
            "ok ahora hazlo en Python",
            "gracias, y el pronóstico para mañana",
            "",
            "hola\nrevisa esto",
        ] {
            assert!(!is_small_talk(t), "{t:?}");
        }
    }

    #[test]
    fn light_models_per_provider() {
        assert_eq!(light_for(ProviderId::Claude).model.as_deref(), Some("haiku"));
        assert_eq!(light_for(ProviderId::Codex).effort.as_deref(), Some("low"));
    }
}
