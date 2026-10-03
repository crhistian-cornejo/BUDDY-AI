//! Skills: folders with a `SKILL.md` (front matter `name` + `description`, then the steps), the same format as
//! Claude Code and Agent Skills, so one written for them can be dropped in `<data>/skills/<name>/`. The agents only
//! see the index (name and description) in their instructions; when a request fits, they load the whole skill with
//! the `use_skill` tool (progressive disclosure: no tokens spent on skills a turn does not need).
//!
//! A skill is instructions, nothing else: it gets no new powers. It can only use the tools Buddy already gives the
//! agent (web search, Office, the music player…); running commands still needs the user's click.

use std::path::{Path, PathBuf};

const BUILT_INS: &[(&str, &str)] = &[("spotify", include_str!("../skills/spotify/SKILL.md"))];
/// Skills listed in an agent's instructions, at most (the rest are ignored, never a prompt flood).
const MAX_LISTED: usize = 30;
const MAX_DESCRIPTION: usize = 300;

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct Skill {
    pub name: String,
    pub description: String,
    /// Shipped with Buddy (kept up to date while the user has not edited it).
    pub built_in: bool,
}

pub fn dir(data_dir: &Path) -> PathBuf {
    data_dir.join("skills")
}

/// Lowercase letters, digits and dashes, like Claude Code's skill names.
pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && !name.starts_with('-')
}

/// `name` and `description` from the front matter (quotes around a value are dropped).
pub fn parse(text: &str) -> Option<(String, String)> {
    let rest = text.trim_start_matches('\u{feff}').trim_start().strip_prefix("---")?;
    let end = rest.find("\n---")?;
    let field = |key: &str| {
        rest[..end].lines().find_map(|l| {
            let (k, v) = l.split_once(':')?;
            let v = v.trim().trim_matches(|c| c == '"' || c == '\'').trim();
            (k.trim() == key && !v.is_empty()).then(|| v.to_string())
        })
    };
    let name = field("name")?;
    valid_name(&name).then(|| (name, field("description").unwrap_or_default()))
}

/// Seeds the built-ins (an edited copy is never touched, as with agents) and lists every valid skill by name.
pub fn list(data_dir: &Path) -> Vec<Skill> {
    let root = dir(data_dir);
    for (name, text) in BUILT_INS {
        let file = root.join(name).join("SKILL.md");
        let seeded = root.join(name).join(".builtin");
        let current = std::fs::read_to_string(&file).ok();
        let unedited = matches!((&current, std::fs::read_to_string(&seeded).ok()), (Some(c), Some(s)) if *c == s);
        if current.is_none() || (unedited && current.as_deref() != Some(*text)) {
            let _ = std::fs::create_dir_all(file.parent().unwrap());
            let _ = std::fs::write(&file, text);
            let _ = std::fs::write(&seeded, text);
        }
    }
    let mut skills: Vec<Skill> = std::fs::read_dir(&root)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let folder = e.file_name().to_string_lossy().to_string();
            let (name, description) = parse(&std::fs::read_to_string(e.path().join("SKILL.md")).ok()?)?;
            // The folder is the skill's address for `use_skill`: it must match the declared name.
            (name == folder).then(|| Skill {
                built_in: BUILT_INS.iter().any(|(n, _)| *n == name),
                description: description.chars().take(MAX_DESCRIPTION).collect(),
                name,
            })
        })
        .collect();
    skills.sort_by(|a, b| a.name.cmp(&b.name));
    skills
}

/// The index for an agent's instructions (empty when there are none).
pub fn prompt_note(skills: &[Skill]) -> String {
    if skills.is_empty() {
        return String::new();
    }
    let mut note = String::from(
        "\n\nHabilidades (skills): si la petición encaja con una, llama primero a la herramienta use_skill con su \
nombre y sigue sus pasos. No las menciones si no hacen falta.\n",
    );
    for s in skills.iter().take(MAX_LISTED) {
        note.push_str(&format!("- {}: {}\n", s.name, s.description));
    }
    note
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_front_matter_and_checks_the_name() {
        let ok = "---\nname: spotify\ndescription: \"Poner música\"\n---\n# Pasos";
        assert_eq!(parse(ok), Some(("spotify".into(), "Poner música".into())));
        assert_eq!(parse("---\nname: ../fuera\n---\nx"), None);
        assert_eq!(parse("sin encabezado"), None);
    }

    #[test]
    fn seeds_the_built_ins_and_lists_only_skills_whose_folder_matches() {
        let tmp = tempfile::tempdir().unwrap();
        let other = dir(tmp.path()).join("notas");
        std::fs::create_dir_all(&other).unwrap();
        std::fs::write(other.join("SKILL.md"), "---\nname: notas\ndescription: Tomar notas\n---\nPasos").unwrap();
        let wrong = dir(tmp.path()).join("otra");
        std::fs::create_dir_all(&wrong).unwrap();
        std::fs::write(wrong.join("SKILL.md"), "---\nname: spotify\n---\nimpostora").unwrap();
        let names: Vec<String> = list(tmp.path()).into_iter().map(|s| s.name).collect();
        assert_eq!(names, ["notas", "spotify"]);
        let note = prompt_note(&list(tmp.path()));
        assert!(note.contains("- notas: Tomar notas") && note.contains("use_skill"));
    }

    #[test]
    fn an_edited_built_in_is_kept() {
        let tmp = tempfile::tempdir().unwrap();
        list(tmp.path());
        let file = dir(tmp.path()).join("spotify/SKILL.md");
        std::fs::write(&file, "---\nname: spotify\ndescription: mía\n---\nmis pasos").unwrap();
        list(tmp.path());
        assert!(std::fs::read_to_string(&file).unwrap().contains("mis pasos"));
    }
}
