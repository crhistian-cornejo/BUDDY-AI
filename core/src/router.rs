//! Router: which model answers Buddy's turn. Rules, not a model: they cost no tokens, take microseconds, run on any
//! machine and say *why* (the reason is shown). Four tiers, each mapped in Settings to a model and an effort:
//!
//! - **Ligero**: small talk (greetings, thanks, «ok») → the cheapest model.
//! - **Normal**: everything else.
//! - **A fondo**: analysis, comparisons, plans, long or many-question messages, several documents.
//! - **Código**: code blocks, errors and stack traces, languages and tools, code files attached.
//!
//! The user can also name a model in the message («con opus», «usa gpt») or fix one model in Settings. Specialists
//! keep their own model. (Switching provider when a plan runs out lives in `chat`.)

use std::path::PathBuf;

use crate::providers::ProviderId;
use crate::store::{Store, fold};

/// The old on/off switch of the small-talk router (replaced by the tiers; kept so old settings stay readable).
pub const SETTING: &str = "router.cheap";
/// "auto" or a model id from `MODELS` (that model always).
pub const MODE_KEY: &str = "router.mode";
const TIER_KEY: &str = "router.tier.";

/// The models Buddy can route to: (id, name shown, provider, model name for the CLI).
pub const MODELS: [(&str, &str, ProviderId, &str); 7] = [
    ("claude:haiku", "Haiku 4.5", ProviderId::Claude, "haiku"),
    ("claude:sonnet", "Sonnet 5.5", ProviderId::Claude, "sonnet"),
    ("claude:opus", "Opus 5.5", ProviderId::Claude, "opus"),
    ("codex:gpt-6.1-sol", "GPT-6.1 Sol", ProviderId::Codex, "gpt-6.1-sol"),
    ("codex:gpt-6-luna", "GPT-6 Luna", ProviderId::Codex, "gpt-6-luna"),
    // Gemini through the Antigravity CLI (Google AI Pro): the tier's effort goes as `--effort`.
    ("antigravity:gemini-3.1-pro", "Gemini 3.1 Pro", ProviderId::Antigravity, "gemini-3.1-pro"),
    ("antigravity:gemini-3.8-flash", "Gemini 3.8 Flash", ProviderId::Antigravity, "gemini-3.8-flash"),
];
pub const EFFORTS: [&str; 3] = ["low", "medium", "high"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum Tier {
    Light,
    Normal,
    Deep,
    Code,
    /// Documents, spreadsheets, presentations, files: the «work» agents.
    Work,
    /// Calculations, equations, proofs.
    Math,
}

impl Tier {
    pub const ALL: [Tier; 6] = [Tier::Light, Tier::Normal, Tier::Deep, Tier::Work, Tier::Code, Tier::Math];

    pub fn parse(key: &str) -> Option<Tier> {
        Tier::ALL.into_iter().find(|t| t.key() == key)
    }

    pub fn key(self) -> &'static str {
        match self {
            Tier::Light => "light",
            Tier::Normal => "normal",
            Tier::Deep => "deep",
            Tier::Code => "code",
            Tier::Work => "work",
            Tier::Math => "math",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Tier::Light => "Ligero",
            Tier::Normal => "Normal",
            Tier::Deep => "A fondo",
            Tier::Code => "Código",
            Tier::Work => "Trabajo",
            Tier::Math => "Matemáticas",
        }
    }

    fn default_choice(self) -> (&'static str, &'static str) {
        match self {
            // Chat, writing and web search on Gemini Flash; real work on Claude and GPT; maths on Gemini Pro.
            Tier::Light => ("antigravity:gemini-3.8-flash", "high"),
            Tier::Normal => ("antigravity:gemini-3.8-flash", "high"),
            Tier::Deep => ("claude:opus", "high"),
            Tier::Work => ("claude:opus", "high"),
            Tier::Code => ("codex:gpt-6.1-sol", "high"),
            Tier::Math => ("antigravity:gemini-3.1-pro", "high"),
        }
    }
}

/// A tier's model in Settings.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct TierChoice {
    pub tier: Tier,
    pub label: String,
    pub model: String,
    pub effort: String,
}

/// A model the user can pick.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct ModelOption {
    pub id: String,
    pub name: String,
    pub provider: String,
}

/// Everything Settings shows: the mode ("auto" or a model id) and each tier's choice.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct RouterConfig {
    pub mode: String,
    pub tiers: Vec<TierChoice>,
    pub models: Vec<ModelOption>,
}

/// What a turn will use, and why (shown to the user).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Route {
    pub tier: Tier,
    pub provider: ProviderId,
    pub model: String,
    pub effort: String,
    pub model_name: String,
    pub reason: String,
}

fn model(id: &str) -> Option<(&'static str, &'static str, ProviderId, &'static str)> {
    MODELS.iter().copied().find(|m| m.0 == id)
}

pub fn options() -> Vec<ModelOption> {
    MODELS.iter().map(|m| ModelOption { id: m.0.into(), name: m.1.into(), provider: m.2.as_str().into() }).collect()
}

/// The saved mode and tiers (defaults for anything missing or unknown).
pub fn config(store: &Store) -> RouterConfig {
    let mode = store.setting(MODE_KEY).ok().flatten().filter(|m| m == "auto" || model(m).is_some()).unwrap_or_else(|| "auto".into());
    let tiers = Tier::ALL
        .iter()
        .map(|&tier| {
            let (model_id, effort) = tier_choice(store, tier);
            TierChoice { tier, label: tier.label().into(), model: model_id, effort }
        })
        .collect();
    RouterConfig { mode, tiers, models: options() }
}

fn tier_choice(store: &Store, tier: Tier) -> (String, String) {
    let saved = store.setting(&format!("{TIER_KEY}{}", tier.key())).ok().flatten().unwrap_or_default();
    let (m, e) = saved.split_once('|').unwrap_or(("", ""));
    let (dm, de) = tier.default_choice();
    let m = if model(m).is_some() { m } else { dm };
    let e = if EFFORTS.contains(&e) { e } else { de };
    (m.to_string(), e.to_string())
}

pub fn set_mode(store: &Store, mode: &str) -> Result<(), String> {
    if mode != "auto" && model(mode).is_none() {
        return Err(format!("Modelo desconocido: {mode}"));
    }
    store.set_setting(MODE_KEY, mode).map_err(|e| e.to_string())
}

pub fn set_tier(store: &Store, tier: Tier, model_id: &str, effort: &str) -> Result<(), String> {
    if model(model_id).is_none() || !EFFORTS.contains(&effort) {
        return Err("Modelo o esfuerzo desconocido.".into());
    }
    store.set_setting(&format!("{TIER_KEY}{}", tier.key()), &format!("{model_id}|{effort}")).map_err(|e| e.to_string())
}

/// The route for Buddy's turn: a model named in the message first, then a fixed model, then the tier's.
pub fn route(store: &Store, text: &str, files: &[PathBuf]) -> Route {
    let (tier, reason) = classify(text, files);
    let build = |id: &str, effort: &str, tier: Tier, reason: String| {
        let (_, name, provider, cli) = model(id).unwrap_or(MODELS[1]);
        Route { tier, provider, model: cli.into(), effort: effort.into(), model_name: name.into(), reason }
    };
    let (tier_model, tier_effort) = tier_choice(store, tier);
    if let Some(id) = named_model(text) {
        let effort = if tier == Tier::Light { "medium".to_string() } else { tier_effort };
        return build(id, &effort, tier, "lo pediste".into());
    }
    let mode = config(store).mode;
    if mode != "auto" {
        return build(&mode, &tier_effort, tier, "modelo fijo en Ajustes".into());
    }
    // Code that also asks for depth gets more thinking, on the code model.
    let effort = if tier == Tier::Code && deep_reason(&fold(text), files).is_some() { "high".to_string() } else { tier_effort };
    build(&tier_model, &effort, tier, reason.into())
}

/// The route for a specialist: with `model: auto` the router decides (never below Normal: a specialist's work is
/// never small talk); with a router model id (`claude:opus`) that model; otherwise `None` (its agent.md model).
pub fn route_agent(store: &Store, model_setting: Option<&str>, text: &str, files: &[PathBuf]) -> Option<Route> {
    let setting = model_setting?;
    if let Some((_, name, provider, cli)) = model(setting) {
        let effort = tier_choice(store, Tier::Normal).1;
        return Some(Route { tier: Tier::Normal, provider, model: cli.into(), effort, model_name: name.into(), reason: "modelo del agente".into() });
    }
    if setting != "auto" {
        return None;
    }
    let mut route = route(store, text, files);
    if route.tier == Tier::Light && route.reason != "lo pediste" {
        let (id, effort) = tier_choice(store, Tier::Normal);
        let (_, name, provider, cli) = model(&id).unwrap_or(MODELS[1]);
        route = Route { tier: Tier::Normal, provider, model: cli.into(), effort, model_name: name.into(), reason: "encargo de Buddy".into() };
    }
    Some(route)
}

/// The tier and a short reason. Pure: same text, same answer.
pub fn classify(text: &str, files: &[PathBuf]) -> (Tier, &'static str) {
    let t = fold(text);
    if files.is_empty() && is_small_talk(text) {
        return (Tier::Light, "charla corta");
    }
    if let Some(reason) = code_reason(text, &t, files) {
        return (Tier::Code, reason);
    }
    if let Some(reason) = math_reason(text, &t) {
        return (Tier::Math, reason);
    }
    if let Some(reason) = work_reason(&t) {
        return (Tier::Work, reason);
    }
    if let Some(reason) = deep_reason(&t, files) {
        return (Tier::Deep, reason);
    }
    (Tier::Normal, "pregunta normal")
}

/// «con opus», «usa gpt», «con sonnet», «usa gemini»… anywhere in the message.
fn named_model(text: &str) -> Option<&'static str> {
    let t = fold(text);
    let asks = |word: &str| ["con ", "usa ", "usando ", "pasalo a ", "pregunta a ", "with "].iter().any(|p| t.contains(&format!("{p}{word}")));
    if asks("opus") {
        Some("claude:opus")
    } else if asks("sonnet") {
        Some("claude:sonnet")
    } else if asks("haiku") {
        Some("claude:haiku")
    } else if asks("luna") {
        Some("codex:gpt-6-luna")
    } else if asks("gpt") || asks("codex") || asks("sol") || asks("chatgpt") {
        Some("codex:gpt-6.1-sol")
    } else if asks("gemini flash") {
        Some("antigravity:gemini-3.8-flash")
    } else if asks("gemini") || asks("antigravity") {
        Some("antigravity:gemini-3.1-pro")
    } else {
        None
    }
}

const CODE_EXTENSIONS: [&str; 22] = [
    "rs", "swift", "py", "ts", "tsx", "js", "jsx", "java", "kt", "go", "c", "h", "cpp", "cs", "rb", "php", "sql", "sh",
    "toml", "yml", "yaml", "json",
];

fn code_reason(raw: &str, t: &str, files: &[PathBuf]) -> Option<&'static str> {
    if raw.contains("```") {
        return Some("trae código");
    }
    if files.iter().any(|f| f.extension().and_then(|e| e.to_str()).is_some_and(|e| CODE_EXTENSIONS.contains(&e.to_lowercase().as_str()))) {
        return Some("adjunta código");
    }
    let code_lines = raw
        .lines()
        .filter(|l| {
            let l = l.trim();
            l.ends_with(';') || l.ends_with('{') || l == "}" || l.starts_with("fn ") || l.starts_with("def ") || l.starts_with("func ")
                || l.starts_with("import ") || l.starts_with("let ") || l.starts_with("const ") || l.contains("=>")
        })
        .count();
    if code_lines >= 3 {
        return Some("trae código");
    }
    if ["traceback", "stack trace", "panicked at", "exception", "error[e", "segmentation fault", "undefined is not"]
        .iter()
        .any(|w| t.contains(w))
    {
        return Some("un error de programa");
    }
    const WORDS: [&str; 32] = [
        "codigo", "programa", "funcion", "script", "bug", "depura", "debug", "compila", "refactor", "regex", "sql",
        "endpoint", "api rest", "rust", "swift", "swiftui", "python", "typescript", "javascript", "kotlin", "react",
        "tauri", "docker", "git ", "github", "pull request", "test unitario", "unit test", "algoritmo", "clase ", "html",
        "css",
    ];
    WORDS.iter().any(|w| t.contains(w)).then_some("es de programación")
}

fn math_reason(raw: &str, t: &str) -> Option<&'static str> {
    const WORDS: [&str; 22] = [
        "matematic", "ecuacion", "integral", "derivada", "limite de", "calcula", "resuelve", "despeja", "algebra",
        "geometria", "trigonometr", "probabilidad", "estadistic", "matriz", "vector", "logaritmo", "teorema",
        "demostracion", "factoriza", "raiz cuadrada", "porcentaje de", "interes compuesto",
    ];
    if WORDS.iter().any(|w| t.contains(w)) {
        return Some("es de matemáticas");
    }
    // A formula: digits with operators and an equals sign, or LaTeX.
    let ops = raw.chars().filter(|c| "+-*/^=".contains(*c)).count();
    let digits = raw.chars().filter(char::is_ascii_digit).count();
    ((raw.contains('=') && ops >= 2 && digits >= 2) || raw.contains("\\frac") || raw.contains("\\int")).then_some("es de matemáticas")
}

fn work_reason(t: &str) -> Option<&'static str> {
    const WORDS: [&str; 18] = [
        "documento", "word", "docx", "excel", "hoja de calculo", "xlsx", "presentacion", "powerpoint", "ppt",
        "diapositiva", "informe", "reporte", "carta formal", "curriculum", "organiza mis archivos", "renombra",
        "plantilla", "acta",
    ];
    WORDS.iter().any(|w| t.contains(w)).then_some("es un trabajo de documentos")
}

fn deep_reason(t: &str, files: &[PathBuf]) -> Option<&'static str> {
    const CUES: [&str; 24] = [
        "a fondo", "en detalle", "detallad", "analiza", "analisis", "investiga", "compara", "comparacion", "estrategia",
        "arquitectura", "disena", "plan de", "planifica", "paso a paso", "razona", "piensa bien", "demuestra", "evalua",
        "pros y contras", "ventajas y desventajas", "investigacion", "ensayo", "profund", "think hard",
    ];
    if CUES.iter().any(|c| t.contains(c)) {
        return Some("pide análisis");
    }
    if t.chars().count() > 700 {
        return Some("mensaje largo");
    }
    if t.matches('?').count() >= 3 {
        return Some("varias preguntas");
    }
    let documents = files.iter().filter(|f| !crate::images::is_image(f)).count();
    (documents >= 2).then_some("varios documentos")
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

    fn store() -> (Store, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        (Store::open(&dir.path().join("t.sqlite")).unwrap(), dir)
    }

    #[test]
    fn small_talk_is_recognised() {
        for t in ["hola", "Hola mi broooo", "¡Gracias!", "ok", "buenas tardes buddy", "jajaja", "¿Qué tal?", "chao"] {
            assert!(is_small_talk(t), "{t:?}");
        }
    }

    #[test]
    fn real_requests_are_not_small_talk() {
        for t in ["hola, ¿me explicas qué es SwiftUI?", "dame 3 tips de Rust", "¿quién gana el clásico?", "ok ahora hazlo en Python", "", "hola\nrevisa esto"] {
            assert!(!is_small_talk(t), "{t:?}");
        }
    }

    #[test]
    fn messages_land_in_their_tier() {
        let none: &[PathBuf] = &[];
        let cases = [
            ("gracias!", Tier::Light),
            ("¿qué tiempo hace en Lima?", Tier::Normal),
            ("pon algo de Radiohead", Tier::Normal),
            ("Analiza los pros y contras de mudarme a Madrid", Tier::Deep),
            ("Compara el iPhone 18 con el Pixel 11", Tier::Deep),
            ("¿por qué falla esta función en Rust?", Tier::Code),
            ("```swift\nlet x = 1\n```", Tier::Code),
            ("thread 'main' panicked at src/main.rs:3", Tier::Code),
            ("Resuelve la ecuación 3x + 2 = 11", Tier::Math),
            ("¿cuánto es 15^2 + 3*4 = ?", Tier::Math),
            ("Hazme un informe en Word sobre la IA", Tier::Work),
            ("Prepara una presentación de 8 diapositivas", Tier::Work),
        ];
        for (text, tier) in cases {
            assert_eq!(classify(text, none).0, tier, "{text:?}");
        }
        assert_eq!(classify("revisa esto", &[PathBuf::from("/a/main.rs")]).0, Tier::Code);
        assert_eq!(classify("resúmelos", &[PathBuf::from("/a/x.pdf"), PathBuf::from("/a/y.docx")]).0, Tier::Deep);
        assert_eq!(classify("hola", &[PathBuf::from("/a/foto.png")]).0, Tier::Normal, "an attachment is never small talk");
    }

    #[test]
    fn defaults_name_wins_and_fixed_mode() {
        let (s, _d) = store();
        let none: &[PathBuf] = &[];
        let r = route(&s, "hola", none);
        assert_eq!((r.provider, r.model.as_str(), r.effort.as_str()), (ProviderId::Antigravity, "gemini-3.8-flash", "high"));
        let r = route(&s, "analiza a fondo esta estrategia", none);
        assert_eq!((r.model.as_str(), r.effort.as_str(), r.model_name.as_str()), ("opus", "high", "Opus 5.5"));
        let r = route(&s, "arregla este bug de typescript", none);
        assert_eq!((r.provider, r.model.as_str(), r.effort.as_str()), (ProviderId::Codex, "gpt-6.1-sol", "high"));
        let r = route(&s, "hazme una presentación sobre ventas", none);
        assert_eq!((r.tier, r.model.as_str()), (Tier::Work, "opus"));
        let r = route(&s, "resuelve la integral de x^2", none);
        assert_eq!((r.tier, r.model.as_str(), r.effort.as_str()), (Tier::Math, "gemini-3.1-pro", "high"));
        let r = route(&s, "explícame la fotosíntesis con opus", none);
        assert_eq!((r.model.as_str(), r.reason.as_str()), ("opus", "lo pediste"));
        set_mode(&s, "codex:gpt-6.1-sol").unwrap();
        let r = route(&s, "hola", none);
        assert_eq!((r.provider, r.reason.as_str()), (ProviderId::Codex, "modelo fijo en Ajustes"));
        assert!(set_mode(&s, "claude:mythos").is_err());
    }

    #[test]
    fn gemini_can_be_named_or_picked() {
        let (s, _d) = store();
        let none: &[PathBuf] = &[];
        let r = route(&s, "Usa gemini: ¿qué es un agujero negro?", none);
        assert_eq!((r.provider, r.model.as_str(), r.reason.as_str()), (ProviderId::Antigravity, "gemini-3.1-pro", "lo pediste"));
        let r = route(&s, "resúmelo con gemini flash", none);
        assert_eq!((r.model.as_str(), r.model_name.as_str()), ("gemini-3.8-flash", "Gemini 3.8 Flash"));
        set_tier(&s, Tier::Normal, "antigravity:gemini-3.8-flash", "low").unwrap();
        let r = route(&s, "¿qué tiempo hace en Lima?", none);
        assert_eq!((r.provider, r.effort.as_str()), (ProviderId::Antigravity, "low"));
        assert!(options().iter().any(|m| m.provider == "antigravity"));
        assert_eq!(route(&s, "hola", none).provider, ProviderId::Antigravity, "small talk on Gemini Flash");
    }

    #[test]
    fn specialists_route_from_normal_up_or_keep_their_model() {
        let (s, _d) = store();
        let none: &[PathBuf] = &[];
        let r = route_agent(&s, Some("auto"), "gracias", none).unwrap();
        assert_eq!((r.tier, r.model.as_str()), (Tier::Normal, "gemini-3.8-flash"), "a hand-off is never small talk");
        let r = route_agent(&s, Some("auto"), "Analiza a fondo el Clásico de mañana", none).unwrap();
        assert_eq!(r.model, "opus");
        let r = route_agent(&s, Some("codex:gpt-6.1-sol"), "lo que sea", none).unwrap();
        assert_eq!((r.provider, r.model.as_str()), (ProviderId::Codex, "gpt-6.1-sol"));
        assert!(route_agent(&s, Some("sonnet"), "x", none).is_none(), "a plain agent.md model stays as it is");
        assert!(route_agent(&s, None, "x", none).is_none());
    }

    #[test]
    fn tiers_are_saved_and_bad_values_fall_back() {
        let (s, _d) = store();
        set_tier(&s, Tier::Deep, "codex:gpt-6.1-sol", "high").unwrap();
        assert!(set_tier(&s, Tier::Deep, "claude:opus", "ultra").is_err());
        let c = config(&s);
        assert_eq!(c.mode, "auto");
        let deep = c.tiers.iter().find(|t| t.tier == Tier::Deep).unwrap();
        assert_eq!((deep.model.as_str(), deep.effort.as_str()), ("codex:gpt-6.1-sol", "high"));
        s.set_setting("router.tier.code", "basura").unwrap();
        let code = config(&s).tiers.into_iter().find(|t| t.tier == Tier::Code).unwrap();
        assert_eq!(code.model, "codex:gpt-6.1-sol");
    }
}
