//! Skills of a named agent and the system prompt that carries them. A skill is a small markdown file the user wrote:
//! `%LOCALAPPDATA%\MIKA\agents\<id>\skills\<slug>.md`, a front matter (`name`, `description`, `enabled`) and a body (the
//! procedure). On every turn the prompt is assembled as: the agent's instructions (the body of its `agent.md`), a short
//! identity line (its current name), then the ENABLED skills under `## Skills`; the caller adds the rest (the Telegram
//! rules, MIKA's team directory, the agent's memory). Skills are the user's own text, trusted like the agent prompt.
//! The slug is the only thing that reaches a file path and it is validated first, so no path can leave the folder.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::named_agents::{self, AgentDefinition};

/// Skills an agent may have.
pub const MAX_SKILLS: usize = 20;
/// Characters of one skill's body.
pub const MAX_BODY: usize = 8_000;
pub const MAX_NAME: usize = 60;
pub const MAX_DESCRIPTION: usize = 240;
/// Characters of all the enabled skills together, as they go into the prompt. What does not fit is left out.
pub const MAX_TOTAL: usize = 20_000;
/// A skill file bigger than this is ignored (the body cap is far below it).
const MAX_FILE_BYTES: u64 = 64_000;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Skill {
    pub slug: String,
    pub name: String,
    pub description: String,
    pub enabled: bool,
    pub body: String,
}

/// What the settings page sends. No `slug`: a new skill (its slug comes from the name); with one: that skill.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SkillInput {
    pub slug: Option<String>,
    pub name: String,
    pub description: String,
    pub body: String,
    /// None: keep what it was (a new skill starts enabled).
    pub enabled: Option<bool>,
}

/// The skills as the settings page shows them, with what the prompt budget leaves out.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillsView {
    pub skills: Vec<Skill>,
    pub max_skills: usize,
    pub max_body: usize,
    pub cap_chars: usize,
    /// Characters the enabled skills that fit take in the prompt.
    pub used_chars: usize,
    /// Enabled skills (slugs) that did not fit under `cap_chars`: they are not sent to the agent.
    pub dropped: Vec<String>,
}

/// `^[a-z0-9][a-z0-9-]{0,39}$`.
pub fn valid_slug(slug: &str) -> bool {
    let mut chars = slug.chars();
    slug.len() <= 40
        && chars.next().is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// A file name from a name: ASCII lowercase, accents dropped, anything else a dash.
pub fn slugify(name: &str) -> String {
    let mut out = String::new();
    for c in name.trim().chars().flat_map(char::to_lowercase) {
        let c = match c { 'á' | 'à' | 'ä' | 'â' => 'a', 'é' | 'è' | 'ë' | 'ê' => 'e', 'í' | 'ì' | 'ï' | 'î' => 'i',
            'ó' | 'ò' | 'ö' | 'ô' => 'o', 'ú' | 'ù' | 'ü' | 'û' => 'u', 'ñ' => 'n', other => other };
        if c.is_ascii_lowercase() || c.is_ascii_digit() { out.push(c); } else if !out.ends_with('-') && !out.is_empty() { out.push('-'); }
    }
    let out: String = out.trim_end_matches('-').chars().take(40).collect();
    let out = out.trim_end_matches('-').to_string();
    if out.is_empty() { "skill".into() } else { out }
}

fn one_line(text: &str, max: usize, what: &str, required: bool) -> Result<String, String> {
    let text = text.trim();
    let count = text.chars().count();
    if required && count == 0 { return Err(format!("Falta {what}.")); }
    if count > max { return Err(format!("{what} pasa de {max} caracteres.", what = capital(what))); }
    if text.chars().any(|c| c.is_control() || c == '"') { return Err(format!("{} no puede tener saltos de línea ni comillas dobles.", capital(what))); }
    Ok(text.to_string())
}

fn capital(text: &str) -> String {
    let mut chars = text.chars();
    chars.next().map(|c| c.to_uppercase().collect::<String>() + chars.as_str()).unwrap_or_default()
}

fn clean_body(body: &str) -> Result<String, String> {
    let body = body.replace("\r\n", "\n");
    let body = body.trim();
    if body.is_empty() { return Err("Falta el procedimiento (el cuerpo de la skill).".into()); }
    let count = body.chars().count();
    if count > MAX_BODY { return Err(format!("El cuerpo pasa de {MAX_BODY} caracteres ({count}).")); }
    Ok(body.to_string())
}

/// The text of the file.
fn render_file(skill: &Skill) -> String {
    format!("---\nname: \"{}\"\ndescription: \"{}\"\nenabled: {}\n---\n{}\n", skill.name, skill.description, skill.enabled, skill.body)
}

fn parse_skill(slug: &str, text: &str) -> Option<Skill> {
    let text = text.replace("\r\n", "\n");
    let mut lines = text.split('\n');
    if lines.next()?.trim() != "---" { return None; }
    let (mut name, mut description, mut enabled) = (String::new(), String::new(), true);
    let mut closed = false;
    for line in lines.by_ref() {
        if line.trim() == "---" { closed = true; break; }
        let Some((key, value)) = line.split_once(':') else { continue };
        let value = value.trim();
        let value = value.strip_prefix('"').and_then(|v| v.strip_suffix('"')).unwrap_or(value);
        match key.trim() {
            "name" => name = value.to_string(),
            "description" => description = value.to_string(),
            "enabled" => enabled = value != "false",
            _ => {}
        }
    }
    if !closed || name.is_empty() { return None; }
    let body = lines.collect::<Vec<_>>().join("\n").trim().to_string();
    Some(Skill { slug: slug.to_string(), name, description, enabled, body })
}

// ── Files ─────────────────────────────────────────────────────────────────────

pub fn skills_dir(agent: &str) -> Result<PathBuf, String> {
    if !named_agents::valid_id(agent) { return Err("Agente no válido.".into()); }
    Ok(named_agents::root().join(agent).join("skills"))
}

fn file_of(dir: &Path, slug: &str) -> Result<PathBuf, String> {
    if !valid_slug(slug) { return Err("Nombre de archivo no válido.".into()); }
    Ok(dir.join(format!("{slug}.md")))
}

/// Every readable skill of the folder, by slug. A damaged, oversized or oddly named file is skipped.
pub fn list_in(dir: &Path) -> Vec<Skill> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut skills: Vec<Skill> = entries.flatten().filter_map(|entry| {
        let path = entry.path();
        let slug = path.file_name()?.to_str()?.strip_suffix(".md")?.to_string();
        let meta = std::fs::symlink_metadata(&path).ok()?;
        if !valid_slug(&slug) || !meta.is_file() || meta.len() > MAX_FILE_BYTES { return None; }
        parse_skill(&slug, &std::fs::read_to_string(&path).ok()?)
    }).collect();
    skills.sort_by(|a, b| a.slug.cmp(&b.slug));
    skills
}

pub fn get_in(dir: &Path, slug: &str) -> Result<Skill, String> {
    let path = file_of(dir, slug)?;
    list_in(dir).into_iter().find(|s| s.slug == slug).filter(|_| path.is_file()).ok_or_else(|| "Esa skill no existe.".to_string())
}

/// Creates or updates a skill. Validates everything before writing; the write is tmp + rename.
pub fn save_in(dir: &Path, input: &SkillInput) -> Result<Skill, String> {
    let name = one_line(&input.name, MAX_NAME, "el nombre", true)?;
    let description = one_line(&input.description, MAX_DESCRIPTION, "la descripción", false)?;
    let body = clean_body(&input.body)?;
    let existing = list_in(dir);
    let slug = match input.slug.as_deref() {
        Some(slug) if valid_slug(slug) => slug.to_string(),
        Some(_) => return Err("Nombre de archivo no válido.".into()),
        None => {
            let base = slugify(&name);
            let mut slug = base.clone();
            let mut n = 2;
            while existing.iter().any(|s| s.slug == slug) {
                let suffix = format!("-{n}");
                slug = format!("{}{suffix}", base.chars().take(40 - suffix.len()).collect::<String>().trim_end_matches('-'));
                n += 1;
            }
            slug
        }
    };
    let previous = existing.iter().find(|s| s.slug == slug);
    if previous.is_none() && existing.len() >= MAX_SKILLS { return Err(format!("Cada agente puede tener hasta {MAX_SKILLS} skills.")); }
    let skill = Skill { slug: slug.clone(), name, description, enabled: input.enabled.or(previous.map(|s| s.enabled)).unwrap_or(true), body };
    write_skill(dir, &skill)?;
    Ok(skill)
}

fn write_skill(dir: &Path, skill: &Skill) -> Result<(), String> {
    let file = file_of(dir, &skill.slug)?;
    std::fs::create_dir_all(dir).map_err(|_| "No se pudo crear la carpeta de skills.")?;
    let tmp = dir.join(format!("{}.md.tmp", skill.slug));
    std::fs::write(&tmp, render_file(skill)).map_err(|_| "No se pudo guardar la skill.")?;
    std::fs::rename(&tmp, &file).map_err(|_| { let _ = std::fs::remove_file(&tmp); "No se pudo guardar la skill.".to_string() })
}

pub fn delete_in(dir: &Path, slug: &str) -> Result<(), String> {
    let file = file_of(dir, slug)?;
    if !file.is_file() { return Err("Esa skill no existe.".into()); }
    std::fs::remove_file(file).map_err(|_| "No se pudo borrar la skill.".into())
}

pub fn toggle_in(dir: &Path, slug: &str, enabled: bool) -> Result<Skill, String> {
    let mut skill = get_in(dir, slug)?;
    skill.enabled = enabled;
    write_skill(dir, &skill)?;
    Ok(skill)
}

// ── The prompt ────────────────────────────────────────────────────────────────

/// One skill as the agent reads it.
fn block(skill: &Skill) -> String {
    let description = if skill.description.is_empty() { String::new() } else { format!("{}\n\n", skill.description) };
    format!("### {}\n{description}{}\n\n", skill.name, skill.body)
}

/// The enabled skills that fit under [`MAX_TOTAL`], in slug order, and the slugs of the enabled ones that do not.
pub fn select(skills: &[Skill]) -> (Vec<&Skill>, Vec<String>, usize) {
    let (mut used, mut kept, mut dropped) = (0, Vec::new(), Vec::new());
    for skill in skills.iter().filter(|s| s.enabled) {
        let size = block(skill).chars().count();
        if used + size <= MAX_TOTAL { used += size; kept.push(skill); } else { dropped.push(skill.slug.clone()); }
    }
    (kept, dropped, used)
}

/// `## Skills` and the skills that fit; empty when there are none.
pub fn skills_section(skills: &[Skill]) -> String {
    let (kept, _, _) = select(skills);
    if kept.is_empty() { return String::new(); }
    let mut out = String::from("\n\n## Skills\nProcedimientos que el usuario te enseñó. Aplica el que corresponda cuando la petición encaje con su descripción.\n\n");
    for skill in kept { out.push_str(&block(skill)); }
    out.trim_end().to_string()
}

/// The line that tells an agent its own name (the one the user gave it).
pub fn identity_block(name: &str, id: &str) -> String {
    format!("\n\nTu nombre en MIKA es {name} (id {id}); los otros agentes y el usuario te llaman así.")
}

/// Instructions, then identity, then skills.
pub fn compose(instructions: &str, name: &str, id: &str, skills: &[Skill]) -> String {
    format!("{instructions}{}{}", identity_block(name, id), skills_section(skills))
}

/// The system prompt of an agent for its next turn, from what is on disk now. The caller appends what is specific to
/// the turn (Telegram rules, team directory, memory).
pub fn system_prompt(agent: &AgentDefinition) -> String {
    let skills = skills_dir(&agent.id).map(|dir| list_in(&dir)).unwrap_or_default();
    format!("{}{}", compose(&agent.prompt, &agent.name, &agent.id, &skills), super::agent_tools::tools_block(agent))
}

pub fn view_in(dir: &Path) -> SkillsView {
    let skills = list_in(dir);
    let (_, dropped, used_chars) = select(&skills);
    SkillsView { skills, max_skills: MAX_SKILLS, max_body: MAX_BODY, cap_chars: MAX_TOTAL, used_chars, dropped }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn temp() -> PathBuf {
        static N: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!("mika-skills-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn input(name: &str, body: &str) -> SkillInput { SkillInput { name: name.into(), body: body.into(), ..Default::default() } }

    #[test]
    fn slugs_are_strict() {
        for good in ["a", "0", "informe-semanal", &"a".repeat(40)] { assert!(valid_slug(good), "{good}"); }
        for bad in ["", "-a", "A", "a_b", "a.b", "a/b", "a\\b", "..", "../x", "a b", "ñ", &"a".repeat(41), "con:ads", "a\n"] { assert!(!valid_slug(bad), "{bad:?}"); }
        assert_eq!(slugify("Informe Semanal"), "informe-semanal");
        assert_eq!(slugify("  ¡Cómo está el año! "), "como-esta-el-ano");
        assert_eq!(slugify("???"), "skill");
        assert!(valid_slug(&slugify(&"palabra ".repeat(20))));
    }

    #[test]
    fn a_skill_round_trips_and_its_file_is_atomic() {
        let dir = temp();
        let saved = save_in(&dir, &SkillInput { description: "Resume en 3 puntos".into(), ..input("Resumen corto", "Lee y resume.\n\nSin adornos.") }).unwrap();
        assert_eq!((saved.slug.as_str(), saved.enabled), ("resumen-corto", true));
        let text = std::fs::read_to_string(dir.join("resumen-corto.md")).unwrap();
        assert!(text.starts_with("---\nname: \"Resumen corto\"\ndescription: \"Resume en 3 puntos\"\nenabled: true\n---\n"));
        assert!(!dir.join("resumen-corto.md.tmp").exists());
        assert_eq!(get_in(&dir, "resumen-corto").unwrap(), saved);
        // Same name again: a new slug, never an overwrite.
        assert_eq!(save_in(&dir, &input("Resumen corto", "Otra")).unwrap().slug, "resumen-corto-2");
        // Editing keeps the slug and the toggle.
        toggle_in(&dir, "resumen-corto", false).unwrap();
        let edited = save_in(&dir, &SkillInput { slug: Some("resumen-corto".into()), ..input("Resumen corto", "Nuevo texto") }).unwrap();
        assert!(!edited.enabled && edited.body == "Nuevo texto");
        delete_in(&dir, "resumen-corto").unwrap();
        assert!(get_in(&dir, "resumen-corto").is_err() && delete_in(&dir, "resumen-corto").is_err());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn validation_and_caps() {
        let dir = temp();
        assert!(save_in(&dir, &input("", "x")).is_err());
        assert!(save_in(&dir, &input("a\nb", "x")).is_err());
        assert!(save_in(&dir, &input("a\"b", "x")).is_err());
        assert!(save_in(&dir, &input(&"n".repeat(61), "x")).is_err());
        assert!(save_in(&dir, &SkillInput { description: "d".repeat(241), ..input("ok", "x") }).is_err());
        assert!(save_in(&dir, &SkillInput { description: "dos\nlíneas".into(), ..input("ok", "x") }).is_err());
        assert!(save_in(&dir, &input("ok", "   ")).is_err());
        assert!(save_in(&dir, &input("ok", &"x".repeat(MAX_BODY + 1))).is_err());
        assert!(save_in(&dir, &input("ok", &"x".repeat(MAX_BODY))).is_ok());
        // Nothing was written for the refused ones.
        assert_eq!(list_in(&dir).len(), 1);
        for i in 1..MAX_SKILLS { save_in(&dir, &input(&format!("skill {i}"), "x")).unwrap(); }
        assert_eq!(list_in(&dir).len(), MAX_SKILLS);
        assert!(save_in(&dir, &input("una más", "x")).unwrap_err().contains("20"));
        // Updating an existing one is still fine at the limit.
        assert!(save_in(&dir, &SkillInput { slug: Some("ok".into()), ..input("ok", "cambiado") }).is_ok());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_slug_can_never_leave_the_folder() {
        let dir = temp();
        for slug in ["../x", "..", "a/b", "a\\b", "C:\\x", "", "/etc/passwd", "a\0b"] {
            assert!(get_in(&dir, slug).is_err(), "{slug:?}");
            assert!(delete_in(&dir, slug).is_err(), "{slug:?}");
            assert!(toggle_in(&dir, slug, true).is_err(), "{slug:?}");
            assert!(save_in(&dir, &SkillInput { slug: Some(slug.into()), ..input("x", "y") }).is_err(), "{slug:?}");
        }
        assert!(!dir.exists() || list_in(&dir).is_empty());
        // Agent ids too.
        for id in ["..", "../mika", "a/b", ""] { assert!(skills_dir(id).is_err(), "{id:?}"); }
    }

    #[test]
    fn foreign_files_are_ignored() {
        let dir = temp();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("Bad Name.md"), "---\nname: \"x\"\n---\ncuerpo").unwrap();
        std::fs::write(dir.join("ok.txt"), "---\nname: \"x\"\n---\ncuerpo").unwrap();
        std::fs::write(dir.join("sin-front.md"), "solo texto").unwrap();
        std::fs::write(dir.join("huge.md"), format!("---\nname: \"x\"\n---\n{}", "a".repeat(70_000))).unwrap();
        std::fs::write(dir.join("fine.md.tmp"), "---\nname: \"x\"\n---\ncuerpo").unwrap();
        std::fs::write(dir.join("fine.md"), "---\r\nname: \"Fine\"\r\nenabled: false\r\n---\r\ncuerpo\r\nlinea").unwrap();
        let skills = list_in(&dir);
        assert_eq!(skills.len(), 1);
        assert_eq!((skills[0].slug.as_str(), skills[0].enabled, skills[0].body.as_str()), ("fine", false, "cuerpo\nlinea"));
        let _ = std::fs::remove_dir_all(dir);
    }

    fn skill(slug: &str, enabled: bool, body: &str) -> Skill {
        Skill { slug: slug.into(), name: slug.to_uppercase(), description: format!("desc {slug}"), enabled, body: body.into() }
    }

    #[test]
    fn the_prompt_is_instructions_then_identity_then_enabled_skills() {
        let skills = vec![skill("a", true, "Haz A."), skill("b", false, "NO LO ENVÍES"), skill("c", true, "Haz C.")];
        let prompt = compose("Eres Mi Mira.", "Mi Mira", "mira", &skills);
        let identity = prompt.find("Tu nombre en MIKA es Mi Mira (id mira); los otros agentes y el usuario te llaman así.").unwrap();
        let section = prompt.find("## Skills").unwrap();
        assert!(prompt.starts_with("Eres Mi Mira.") && identity > 0 && identity < section);
        assert!(prompt.find("### A").unwrap() < prompt.find("### C").unwrap());
        assert!(prompt.contains("desc a\n\nHaz A.") && !prompt.contains("NO LO ENVÍES") && !prompt.contains("### B"));
        // No skills, no section.
        assert!(!compose("x", "X", "x", &[skill("a", false, "z")]).contains("## Skills"));
        assert!(!compose("x", "X", "x", &[]).contains("Skills"));
    }

    #[test]
    fn skills_over_the_budget_are_dropped_whole() {
        let big = "x".repeat(MAX_BODY);
        let skills: Vec<Skill> = ["a", "b", "c", "d"].iter().map(|s| skill(s, true, &big)).collect();
        let (kept, dropped, used) = select(&skills);
        assert_eq!(kept.len(), 2, "two bodies of 8,000 fit under 20,000, three do not");
        assert_eq!(dropped, ["c", "d"]);
        assert!(used <= MAX_TOTAL);
        let section = skills_section(&skills);
        assert!(section.contains("### A") && section.contains("### B") && !section.contains("### C"));
        // Disabled ones cost nothing and are not "dropped".
        let mixed = vec![skill("a", false, &big), skill("b", true, &big)];
        assert!(select(&mixed).1.is_empty());
    }

    #[test]
    fn view_reports_what_does_not_fit() {
        let dir = temp();
        for n in 0..4 { save_in(&dir, &input(&format!("grande {n}"), &"y".repeat(MAX_BODY))).unwrap(); }
        let view = view_in(&dir);
        assert_eq!((view.skills.len(), view.dropped.len(), view.cap_chars), (4, 2, MAX_TOTAL));
        assert!(view.used_chars <= MAX_TOTAL);
        let _ = std::fs::remove_dir_all(dir);
    }
}
