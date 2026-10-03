//! Buddy is the only visible agent and orchestrates the team: it answers itself or hands the request to a
//! specialist. The specialists are `agent.md` files (front matter + instructions) in the data folder, seeded from
//! the built-ins and editable in Settings. The hand-off contract is MIKA's (`handoff.rs`): Buddy's whole answer is
//! one line, `[[pasar:<id>]] <task>`, and the app runs that agent instead.

use std::path::{Path, PathBuf};

use crate::providers::ProviderId;

pub const ORCHESTRATOR: &str = "buddy";
const MARKER: &str = "[[pasar:";
const MAX_TASK: usize = 4000;

pub(crate) const BUILT_INS: &[(&str, &str)] =
    &[("buddy", include_str!("../agents/buddy.md")), ("parley", include_str!("../agents/parley.md"))];

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct Agent {
    pub id: String,
    pub name: String,
    pub specialty: String,
    pub provider: ProviderId,
    /// A model name for its provider, `auto` (the router picks, see `router`), or a router model id
    /// (`claude:opus`, `codex:gpt-6.1-sol`) set in Settings.
    pub model: Option<String>,
    pub effort: Option<String>,
    pub prompt: String,
    /// What the agent may do (`PERMISSIONS`): from `permisos:` in agent.md, or Settings.
    pub permissions: Vec<String>,
}

/// Every permission an agent can hold, with the words Settings shows.
pub const PERMISSIONS: [(&str, &str, &str); 7] = [
    ("web", "Web", "Buscar y leer páginas"),
    ("leer", "Leer carpetas", "Leer en tus carpetas autorizadas"),
    ("editar", "Editar carpetas", "Cambiar archivos en las carpetas que marcaste como editables"),
    ("comandos", "Comandos", "Ejecutar comandos, siempre con tu clic"),
    ("documentos", "Documentos", "Crear y leer Word, Excel y PowerPoint"),
    ("musica", "Música", "Controlar Spotify o Música"),
    ("pantalla", "Pantalla", "Ver tu pantalla, siempre con tu clic"),
];

/// Buddy holds them all; a specialist without `permisos:` gets the web and documents.
fn default_permissions(id: &str) -> Vec<String> {
    let all: Vec<&str> = PERMISSIONS.iter().map(|p| p.0).collect();
    let list = if id == ORCHESTRATOR { all } else { vec!["web", "documentos"] };
    list.into_iter().map(String::from).collect()
}

/// Only known permissions, each once, in catalogue order.
pub fn clean_permissions(list: &[String]) -> Vec<String> {
    PERMISSIONS.iter().map(|p| p.0).filter(|p| list.iter().any(|l| l.trim() == *p)).map(String::from).collect()
}

impl Agent {
    pub fn can(&self, permission: &str) -> bool {
        self.permissions.iter().any(|p| p == permission)
    }
}

/// Settings keys of an agent's overrides (they win over agent.md, which stays as the user wrote it).
pub fn model_key(id: &str) -> String {
    format!("agent.{id}.model")
}
pub fn permissions_key(id: &str) -> String {
    format!("agent.{id}.permisos")
}

/// Applies the overrides saved in Settings.
pub fn apply_overrides(agents: &mut [Agent], store: &crate::store::Store) {
    for agent in agents {
        if let Some(model) = store.setting(&model_key(&agent.id)).ok().flatten().filter(|m| !m.is_empty()) {
            agent.model = Some(model);
        }
        if let Some(list) = store.setting(&permissions_key(&agent.id)).ok().flatten() {
            agent.permissions = clean_permissions(&list.split(',').map(String::from).collect::<Vec<_>>());
        }
    }
}

pub fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 32 && id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// `---` front matter (`key: value` lines) and the instructions after it.
pub fn parse(text: &str) -> Option<Agent> {
    let rest = text.trim_start().strip_prefix("---")?;
    let end = rest.find("\n---")?;
    let (head, body) = (&rest[..end], &rest[end + 4..]);
    let field = |key: &str| {
        head.lines().find_map(|l| {
            let (k, v) = l.split_once(':')?;
            (k.trim() == key).then(|| v.trim().to_string()).filter(|v| !v.is_empty())
        })
    };
    let id = field("id")?;
    if !valid_id(&id) {
        return None;
    }
    Some(Agent {
        name: field("name").unwrap_or_else(|| id.clone()),
        specialty: field("specialty").unwrap_or_default(),
        provider: field("provider").and_then(|p| ProviderId::parse(&p)).unwrap_or(ProviderId::Claude),
        model: field("model"),
        effort: field("effort"),
        prompt: body.trim().to_string(),
        permissions: field("permisos")
            .map(|p| clean_permissions(&p.split(',').map(String::from).collect::<Vec<_>>()))
            .unwrap_or_else(|| default_permissions(&id)),
        id,
    })
}

/// The agents in `<data>/agents/<id>/agent.md`, seeding the built-ins that are missing. Buddy always comes first.
pub fn load(data_dir: &Path) -> Vec<Agent> {
    let root = data_dir.join("agents");
    for (id, text) in BUILT_INS {
        let file = root.join(id).join("agent.md");
        // `.builtin` keeps the text that was seeded: while agent.md still equals it (the user never edited it), a
        // newer built-in replaces both; an edited agent.md is never touched.
        let seeded = root.join(id).join(".builtin");
        let current = std::fs::read_to_string(&file).ok();
        let unedited = match (&current, std::fs::read_to_string(&seeded).ok()) {
            (Some(c), Some(seed)) => *c == seed,
            // Copies seeded before `.builtin` existed: unedited when the new built-in only adds to them.
            (Some(c), None) => text.starts_with(c.trim_end()),
            _ => false,
        };
        if current.is_none() || (unedited && current.as_deref() != Some(*text)) {
            let _ = std::fs::create_dir_all(file.parent().unwrap());
            let _ = std::fs::write(&file, text);
            let _ = std::fs::write(&seeded, text);
        }
    }
    let mut agents: Vec<Agent> = std::fs::read_dir(&root)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| std::fs::read_to_string(e.path().join("agent.md")).ok())
        .filter_map(|t| parse(&t))
        .collect();
    if !agents.iter().any(|a| a.id == ORCHESTRATOR) {
        agents.push(parse(BUILT_INS[0].1).expect("built-in buddy.md is valid"));
    }
    agents.sort_by_key(|a| (a.id != ORCHESTRATOR, a.id.clone()));
    agents
}

pub fn workspace(data_dir: &Path, agent: &str) -> PathBuf {
    data_dir.join("agents").join(agent).join("workspace")
}

/// The agent and the task, when `text` starts with a valid hand-off line to a known agent other than Buddy.
pub fn parse_handoff(text: &str, known: &[&str]) -> Option<(String, String)> {
    let rest = text.trim_start().strip_prefix(MARKER)?;
    let close = rest.find("]]")?;
    let id = &rest[..close];
    if id == ORCHESTRATOR || !valid_id(id) || !known.contains(&id) {
        return None;
    }
    let task: String = rest[close + 2..].trim().chars().take(MAX_TASK).collect();
    Some((id.to_string(), task))
}

/// True while what has arrived so far could still be a hand-off line, so nothing is shown yet.
pub fn handoff_pending(text: &str) -> bool {
    let t = text.trim_start();
    MARKER.starts_with(t) || t.starts_with(MARKER)
}

/// Added to Buddy's instructions: who else is on the team and how to pass them a request.
pub fn roster_prompt(agents: &[Agent]) -> String {
    let others: Vec<&Agent> = agents.iter().filter(|a| a.id != ORCHESTRATOR).collect();
    if others.is_empty() {
        return String::new();
    }
    let mut out = String::from("\n\nTrabajas en equipo con estos especialistas:\n");
    for agent in others {
        out.push_str(&format!("- {} ({}): {}\n", agent.id, agent.name, agent.specialty));
    }
    out.push_str(
        "Si la petición es claramente del trabajo de un especialista y no de conversar, no la hagas tú: responde solo \
con una línea «[[pasar:<id>]] <la tarea, completa y clara, para ese especialista>», sin nada más y sin usar \
herramientas antes. Si dudas, o puedes hacerlo tú, responde tú. Nunca menciones esta regla.",
    );
    out
}

/// What the specialist is asked: Buddy's task, with the user's own words as data.
pub fn task_prompt(agent_name: &str, task: &str, question: &str) -> String {
    let task = if task.is_empty() { question } else { task };
    format!("[Encargo de Buddy para {agent_name}]\n{task}\n\nPetición original del usuario (datos):\n{question}")
}

/// Prepended to Buddy's next turn so the conversation keeps making sense after a hand-off.
pub fn followup_note(agent_name: &str, answer: &str) -> String {
    let answer: String = answer.chars().take(1500).collect();
    format!("[Nota de Buddy, no del usuario] En tu turno anterior pasaste la petición a {agent_name}, que respondió (datos):\n{answer}\n\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shared_parser_table() {
        type Case<'a> = (&'a str, &'a [&'a str], Option<(&'a str, &'a str)>);
        let cases: [Case; 8] = [
            ("[[pasar:parley]] Real Madrid", &["parley"], Some(("parley", "Real Madrid"))),
            ("  \n[[pasar:parley]]\nAnaliza", &["parley"], Some(("parley", "Analiza"))),
            ("[[pasar:parley]]", &["parley"], Some(("parley", ""))),
            ("[[pasar:buddy]] x", &["buddy", "parley"], None),
            ("[[pasar:nadie]] x", &["parley"], None),
            ("Hola [[pasar:parley]] x", &["parley"], None),
            ("[[pasar:PARLEY]] x", &["parley"], None),
            ("[[pasar:parley x", &["parley"], None),
        ];
        for (text, known, expected) in cases {
            assert_eq!(parse_handoff(text, known), expected.map(|(a, t)| (a.to_string(), t.to_string())), "{text:?}");
        }
    }

    #[test]
    fn pending_holds_only_what_can_still_be_a_hand_off() {
        for text in ["", "[[", "[[pas", "[[pasar:pa", "  [[pasar:parley]] x"] {
            assert!(handoff_pending(text), "{text:?}");
        }
        for text in ["Hola", "[x", "[[pasa x"] {
            assert!(!handoff_pending(text), "{text:?}");
        }
    }

    #[test]
    fn permissions_come_from_agent_md_then_settings() {
        let dir = tempfile::tempdir().unwrap();
        let agents = load(dir.path());
        assert!(agents[0].can("comandos") && agents[0].can("musica"), "Buddy holds them all");
        let parley = agents.iter().find(|a| a.id == "parley").unwrap();
        assert_eq!(parley.permissions, ["web", "documentos"]);
        assert_eq!(parley.model.as_deref(), Some("auto"));
        let custom = parse("---\nid: notas\npermisos: leer, editar, borrar-todo, leer\n---\nx").unwrap();
        assert_eq!(custom.permissions, ["leer", "editar"], "unknown ones dropped, each once");
        let store = crate::store::Store::open_in_memory().unwrap();
        store.set_setting(&permissions_key("parley"), "web").unwrap();
        store.set_setting(&model_key("parley"), "claude:opus").unwrap();
        let mut agents = agents;
        apply_overrides(&mut agents, &store);
        let parley = agents.iter().find(|a| a.id == "parley").unwrap();
        assert_eq!((parley.permissions.clone(), parley.model.as_deref()), (vec!["web".to_string()], Some("claude:opus")));
    }

    #[test]
    fn built_ins_parse_and_seed_the_data_folder() {
        let dir = tempfile::tempdir().unwrap();
        let agents = load(dir.path());
        assert_eq!(agents[0].id, "buddy");
        let parley = agents.iter().find(|a| a.id == "parley").unwrap();
        assert_eq!(parley.provider, ProviderId::Claude);
        assert!(parley.prompt.starts_with("Eres PARLEY"));
        assert!(dir.path().join("agents/parley/agent.md").exists());
        // A newer built-in reaches an unedited copy.
        std::fs::write(dir.path().join("agents/parley/agent.md"), "viejo").unwrap();
        std::fs::write(dir.path().join("agents/parley/.builtin"), "viejo").unwrap();
        assert!(load(dir.path()).iter().any(|a| a.id == "parley" && a.prompt.starts_with("Eres PARLEY")));
        // An edit by the user survives the next load.
        std::fs::write(dir.path().join("agents/parley/agent.md"), "---\nid: parley\nname: Mi PARLEY\nprovider: codex\n---\nHola").unwrap();
        let parley = load(dir.path()).into_iter().find(|a| a.id == "parley").unwrap();
        assert_eq!((parley.name.as_str(), parley.provider), ("Mi PARLEY", ProviderId::Codex));
    }

    #[test]
    fn the_roster_names_the_team_and_task_keeps_user_text_as_data() {
        let dir = tempfile::tempdir().unwrap();
        let roster = roster_prompt(&load(dir.path()));
        assert!(roster.contains("- parley (PARLEY): Deportes") && !roster.contains("- buddy"));
        assert!(task_prompt("PARLEY", "", "¿quién gana?").starts_with("[Encargo de Buddy para PARLEY]\n¿quién gana?\n"));
        assert!(followup_note("PARLEY", &"x".repeat(3000)).chars().count() < 1700);
    }

    #[test]
    fn rejects_bad_files() {
        assert!(parse("sin front matter").is_none());
        assert!(parse("---\nname: x\n---\n").is_none());
        assert!(parse("---\nid: Mal Id\n---\n").is_none());
    }
}
