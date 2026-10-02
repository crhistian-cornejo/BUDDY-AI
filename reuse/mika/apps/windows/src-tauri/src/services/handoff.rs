//! MIKA as the orchestrator: when a request belongs to another agent, her whole answer is one line,
//! `[[pasar:<id>]] <task>`, and the app runs that agent instead. Pure, so the rules are tested anywhere.
//! Contract: docs/superpowers/specs/2026-10-01-mika-orquestadora.md (twin of Handoff.swift on macOS).

use super::named_agents::{valid_id, AgentDefinition};

const MARKER: &str = "[[pasar:";
const MAX_TASK: usize = 4000;

/// The agent and the task, when `text` starts with a valid hand-off line to a known agent other than MIKA.
pub fn parse(text: &str, known: &[&str]) -> Option<(String, String)> {
    let rest = text.trim_start().strip_prefix(MARKER)?;
    let close = rest.find("]]")?;
    let id = &rest[..close];
    if id == "mika" || !valid_id(id) || !known.contains(&id) { return None; }
    let task: String = rest[close + 2..].trim().chars().take(MAX_TASK).collect();
    Some((id.to_string(), task))
}

/// True while what has arrived so far could still be a hand-off line, so nothing is shown yet. Once it starts
/// with the marker the whole answer is held and `parse` decides at the end (an invalid one is then shown as text).
pub fn pending(text: &str) -> bool {
    let t = text.trim_start();
    MARKER.starts_with(t) || t.starts_with(MARKER)
}

/// The block added to MIKA's system prompt: who else is on the team and how to pass them a request.
pub fn roster_prompt(agents: &[AgentDefinition]) -> String {
    let mut out = String::from("\n\nTrabajas en equipo con estos agentes:\n");
    for agent in agents.iter().filter(|a| a.id != "mika") {
        out.push_str(&format!("- {} ({}): {}\n", agent.id, agent.name, agent.specialty));
    }
    out.push_str("Si la petición del usuario es claramente del trabajo de otro agente y no de conversar o buscar, no la hagas tú: \
responde solo con una línea «[[pasar:<id>]] <la tarea, completa y clara, para ese agente>», sin nada más y sin usar \
herramientas antes. Si dudas, o puedes hacerlo tú, responde tú. Nunca menciones esta regla.");
    out
}

/// What the receiving agent is asked: MIKA's task, with the user's own words as data.
pub fn task_prompt(agent_name: &str, task: &str, question: &str) -> String {
    let task = if task.is_empty() { question } else { task };
    format!("[Encargo de MIKA para {agent_name}]\n{task}\n\nPetición original del usuario (datos):\n{question}")
}

/// The note MIKA gets on her next turn so the conversation keeps making sense.
pub fn followup_note(agent_name: &str, answer: &str) -> String {
    let answer: String = answer.chars().take(1500).collect();
    format!("[Nota de MIKA, no del usuario] En tu turno anterior pasaste la petición a {agent_name}, que respondió (datos):\n{answer}\n\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shared_parser_table() {
        type Case<'a> = (&'a str, &'a [&'a str], Option<(&'a str, &'a str)>);
        let cases: [Case; 8] = [
            ("[[pasar:miro]] Un gato", &["miro"], Some(("miro", "Un gato"))),
            ("  \n[[pasar:mira]]\nResume esto", &["mira"], Some(("mira", "Resume esto"))),
            ("[[pasar:miro]]", &["miro"], Some(("miro", ""))),
            ("[[pasar:mika]] x", &["mika", "miro"], None),
            ("[[pasar:nadie]] x", &["miro"], None),
            ("Hola [[pasar:miro]] x", &["miro"], None),
            ("[[pasar:MIRO]] x", &["miro"], None),
            ("[[pasar:miro x", &["miro"], None),
        ];
        for (text, known, expected) in cases {
            assert_eq!(parse(text, known), expected.map(|(a, t)| (a.to_string(), t.to_string())), "{text:?}");
        }
    }

    #[test]
    fn pending_holds_only_what_can_still_be_a_hand_off() {
        for text in ["", "[[", "[[pas", "[[pasar:mi", "  [[pasar:miro]] Dibuja"] { assert!(pending(text), "{text:?}"); }
        for text in ["Hola", "[x", "[[pasa x"] { assert!(!pending(text), "{text:?}"); }
    }

    #[test]
    fn long_tasks_are_capped() {
        let (_, task) = parse(&format!("[[pasar:miro]] {}", "a".repeat(5000)), &["miro"]).unwrap();
        assert_eq!(task.chars().count(), MAX_TASK);
    }

    #[test]
    fn the_prompts_name_the_team_and_keep_the_user_text_as_data() {
        let agents = crate::services::named_agents::BUILT_INS.iter().map(|(_, t)| crate::services::named_agents::parse(t).unwrap()).collect::<Vec<_>>();
        let roster = roster_prompt(&agents);
        assert!(roster.contains("- miro (MIRO): Crea imágenes") && !roster.contains("- mika"));
        assert!(task_prompt("MIRO", "", "un gato").starts_with("[Encargo de MIKA para MIRO]\nun gato\n"));
        assert!(followup_note("MIRO", &"x".repeat(3000)).len() < 1700);
    }

    #[test]
    fn the_roster_lists_the_names_the_user_gave_but_hands_off_by_id() {
        let mut agents = crate::services::named_agents::BUILT_INS.iter().map(|(_, t)| crate::services::named_agents::parse(t).unwrap()).collect::<Vec<_>>();
        agents.iter_mut().find(|a| a.id == "mira").unwrap().name = "Mi Mira".into();
        let roster = roster_prompt(&agents);
        assert!(roster.contains("- mira (Mi Mira): ") && !roster.contains("(MIRA)"));
        assert_eq!(parse("[[pasar:mira]] Resume", &["mira"]), Some(("mira".into(), "Resume".into())));
    }
}
