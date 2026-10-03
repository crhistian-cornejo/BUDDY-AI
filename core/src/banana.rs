//! Banana, the images specialist: makes pictures and edits the ones the user attaches, through the image generators
//! of the user's own subscriptions. Gemini's (Nano Banana, in Antigravity) goes first; ChatGPT's (in Codex) takes
//! over when Gemini cannot. Neither CLI hands the file to Buddy: each leaves it in its own folder, so after the turn
//! the core picks up what appeared there and copies it, untouched (no resizing, no re-encoding), into Buddy's
//! documents folder, where the chat shows it.
//!
//! Live checks (agy 1.2.16, codex 0.160, Oct 2026): Gemini's main agent has no image tool; its `image-generator`
//! subagent does, takes input pictures, and writes `brain/<conversation>/<name>.jpg` (1376×768 for 16:9). Codex's
//! `image_gen` writes `generated_images/<thread>/<name>.png` (1664×936).

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use crate::providers::ProviderId;

pub const AGENT: &str = "banana";
/// The permission that makes an agent an image maker (Settings › Agentes).
pub const PERMISSION: &str = "imagenes";
/// The agent's own words for «no picture came out»: the next generator takes the turn.
pub const NO_IMAGE: &str = "[[sin-imagen]]";

/// How each subscription's CLI makes a picture; added to the agent's instructions for that provider's turn.
pub fn method(provider: ProviderId) -> &'static str {
    match provider {
        ProviderId::Antigravity => "\n\n[Cómo generar aquí] Tú no tienes la herramienta de imágenes: la tiene tu subagente `image-generator`. Encárgale el trabajo con la instrucción completa y, si hay imagen adjunta, su ruta absoluta como imagen de entrada de `generate_image`. Después ESPERA a que termine (usa `schedule` y revisa `manage_subagents`): no termines tu turno hasta que el archivo exista. No busques en la web.",
        ProviderId::Codex => "\n\n[Cómo generar aquí] Usa tu herramienta integrada de generación de imágenes (`image_gen`). Para editar, usa la imagen adjunta como entrada. No ejecutes comandos ni muevas el archivo.",
        _ => "",
    }
}

/// True when the message asks for a picture to be made, or for an attached picture to be changed.
pub fn wants_image(text: &str, files: &[PathBuf]) -> bool {
    let folded = crate::store::fold(text);
    let words: Vec<&str> = folded.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).collect();
    let has = |list: &[&str]| words.iter().any(|w| list.contains(w));
    // Work for other hands: slides and documents (Opus), drawings made of code (the code model).
    const OTHERS: [&str; 16] = [
        "powerpoint", "ppt", "presentacion", "diapositivas", "documento", "word", "excel", "pdf", "svg", "html", "css",
        "codigo", "grafico", "grafica", "tabla", "diagrama",
    ];
    if has(&OTHERS) {
        return false;
    }
    const MAKE: [&str; 19] = [
        "genera", "generame", "generar", "crea", "creame", "crear", "haz", "hazme", "hacer", "dibuja", "dibujame",
        "dibujar", "disena", "diseña", "disename", "ilustra", "ilustrame", "imagina", "pinta",
    ];
    const PICTURE: [&str; 19] = [
        "imagen", "imagenes", "foto", "fotos", "ilustracion", "dibujo", "logo", "logotipo", "wallpaper", "poster",
        "afiche", "banner", "icono", "sticker", "avatar", "portada", "miniatura", "meme", "retrato",
    ];
    if has(&MAKE) && (has(&PICTURE) || folded.contains("fondo de pantalla")) {
        return true;
    }
    const EDIT: [&str; 26] = [
        "edita", "editame", "editar", "editala", "modifica", "modificala", "retoca", "retocala", "quita", "quitale",
        "agrega", "agregale", "anade", "anadele", "ponle", "borra", "borrale", "elimina", "cambia", "cambiale",
        "recorta", "mejora", "mejorala", "restaura", "colorea", "reemplaza",
    ];
    files.iter().any(|f| crate::images::is_image(f)) && has(&EDIT)
}

/// The home folder both CLIs keep their data in.
pub fn home() -> PathBuf {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from).unwrap_or_default()
}

/// Where each CLI leaves the pictures it makes: one folder per conversation.
fn outboxes(home: &Path) -> [PathBuf; 2] {
    [home.join(".gemini/antigravity-cli/brain"), home.join(".codex/generated_images")]
}

/// The pictures the CLIs made since `since`, copied as they are into `into` (oldest first, at most 4). The same
/// picture left in two conversation folders (a subagent's and its parent's) counts once.
pub fn collect(home: &Path, since: SystemTime, into: &Path) -> Vec<PathBuf> {
    let mut found: Vec<(SystemTime, u64, PathBuf)> = Vec::new();
    for outbox in outboxes(home) {
        for conversation in std::fs::read_dir(&outbox).into_iter().flatten().flatten() {
            for entry in std::fs::read_dir(conversation.path()).into_iter().flatten().flatten() {
                let path = entry.path();
                let Ok(meta) = entry.metadata() else { continue };
                let fresh = meta.modified().is_ok_and(|m| m >= since);
                if meta.is_file() && fresh && crate::images::is_image(&path) && !found.iter().any(|f| f.1 == meta.len()) {
                    found.push((meta.modified().unwrap_or(since), meta.len(), path));
                }
            }
        }
    }
    found.sort();
    let stamp = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap_or_default().as_secs();
    let _ = std::fs::create_dir_all(into);
    found
        .into_iter()
        .take(4)
        .enumerate()
        .filter_map(|(n, (_, _, from))| {
            let ext = from.extension()?.to_string_lossy().to_lowercase();
            let to = into.join(format!("{}-{stamp}{}.{ext}", title(&from), if n == 0 { String::new() } else { format!("-{}", n + 1) }));
            std::fs::copy(&from, &to).ok().map(|_| to)
        })
        .collect()
}

/// Waits for a picture that is still being made (Gemini's subagent can outlive its parent's turn).
pub fn wait(home: &Path, since: SystemTime, into: &Path, at_most: Duration, stop: &dyn Fn() -> bool) -> Vec<PathBuf> {
    let started = std::time::Instant::now();
    loop {
        let made = collect(home, since, into);
        if !made.is_empty() || started.elapsed() >= at_most || stop() {
            return made;
        }
        std::thread::sleep(Duration::from_secs(2));
    }
}

/// A file name from the generator's own: `fox_wizard_hat_1791051060121.jpg` → `fox-wizard-hat`; a bare id → `imagen`.
fn title(path: &Path) -> String {
    let stem = path.file_stem().map(|s| s.to_string_lossy().to_lowercase()).unwrap_or_default();
    let words: Vec<&str> = stem
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty() && !w.chars().any(|c| c.is_ascii_digit()) && *w != "exec" && *w != "media")
        .take(6)
        .collect();
    if words.is_empty() { "imagen".into() } else { words.join("-") }
}

/// What the user reads under the picture: the agent's last paragraph, without paths, markers or waiting notes.
pub fn final_words(text: &str, made: usize) -> String {
    let last = text
        .split("\n\n")
        .map(|p| {
            p.lines()
                .filter(|l| ![".gemini", ".codex", "/brain/", "generated_images", "file://"].iter().any(|s| l.contains(s)))
                .collect::<Vec<_>>()
                .join("\n")
                .replace(NO_IMAGE, "")
                .trim()
                .to_string()
        })
        .filter(|p| !p.is_empty())
        .last()
        .unwrap_or_default();
    match (made, last.is_empty()) {
        (0, true) => "No salió ninguna imagen. Inténtalo otra vez o dime qué cambio.".into(),
        (0, false) => last,
        (1, true) => "Aquí está tu imagen.".into(),
        (_, true) => "Aquí están tus imágenes.".into(),
        // With the picture made, only the closing line: what comes before it are waiting notes.
        (_, false) => last.lines().last().unwrap_or_default().trim().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_picture_request_is_told_from_other_work() {
        let none: &[PathBuf] = &[];
        let photo = [PathBuf::from("/x/foto.jpg")];
        for yes in ["genera una imagen de un zorro tomando café", "hazme un logo para mi cafetería", "Crea un fondo de pantalla minimalista", "dibújame una ilustración de Lima de noche"] {
            assert!(wants_image(yes, none), "{yes}");
        }
        for yes in ["quítale el fondo", "ponle un sombrero al perro", "edita esta foto: más luz", "cambia el color del polo a azul"] {
            assert!(wants_image(yes, &photo), "{yes}");
        }
        for no in ["¿qué dice esta imagen?", "analiza esta foto", "responde este correo"] {
            assert!(!wants_image(no, &photo), "{no}");
        }
        for no in ["crea un powerpoint con imágenes de Lima", "haz un diagrama en svg", "quita el fondo", "crea una playlist de rock", "hazme un resumen", "quiero saber qué foto usar", "crea un fondo de emergencia"] {
            assert!(!wants_image(no, none), "{no}");
        }
    }

    #[test]
    fn new_pictures_are_copied_untouched_and_once() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let brain = home.join(".gemini/antigravity-cli/brain");
        let old = brain.join("antes");
        std::fs::create_dir_all(&old).unwrap();
        std::fs::write(old.join("vieja.jpg"), b"vieja").unwrap();
        std::thread::sleep(Duration::from_millis(1100));
        let since = SystemTime::now();
        let (main, sub) = (brain.join("c1"), brain.join("c2"));
        std::fs::create_dir_all(main.join(".tempmediaStorage")).unwrap();
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(main.join("fox_wizard_hat_1791051060121.jpg"), b"la imagen").unwrap();
        std::fs::write(sub.join("fox_wizard_hat_1791051060999.jpg"), b"la imagen").unwrap();
        std::fs::write(main.join(".tempmediaStorage/media_1.jpg"), b"lo que miro").unwrap();
        std::fs::write(main.join("notas.md"), b"texto").unwrap();
        let codex = home.join(".codex/generated_images/t1");
        std::fs::create_dir_all(&codex).unwrap();
        std::fs::write(codex.join("exec-f5887723.png"), b"otra imagen mas").unwrap();

        let out = dir.path().join("documentos");
        let made = collect(&home, since, &out);
        assert_eq!(made.len(), 2, "{made:?}");
        let names: Vec<String> = made.iter().map(|p| p.file_name().unwrap().to_string_lossy().into_owned()).collect();
        assert!(names.iter().any(|n| n.starts_with("fox-wizard-hat-") && n.ends_with(".jpg")), "{names:?}");
        assert!(names.iter().any(|n| n.starts_with("imagen-") && n.ends_with(".png")), "{names:?}");
        let jpg = made.iter().find(|p| p.extension().unwrap() == "jpg").unwrap();
        assert_eq!(std::fs::read(jpg).unwrap(), b"la imagen", "the same bytes: no resizing, no re-encoding");
    }

    #[test]
    fn only_the_last_words_reach_the_user() {
        let text = "He enviado la solicitud al subagente.\n\nSigo esperando.\n\nAñadí un sombrero morado al zorro.\n/Users/x/.gemini/antigravity-cli/brain/c1/fox.jpg";
        assert_eq!(final_words(text, 1), "Añadí un sombrero morado al zorro.");
        assert_eq!(final_words("Comprobando el archivo.\nHe creado un gato astronauta.", 1), "He creado un gato astronauta.");
        assert_eq!(final_words("", 1), "Aquí está tu imagen.");
        assert_eq!(final_words("[[sin-imagen]] sin cuota hoy", 0), "sin cuota hoy");
    }
}
