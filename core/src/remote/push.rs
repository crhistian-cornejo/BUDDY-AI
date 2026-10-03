//! Which events deserve a notice on the phone while it is away, and what the notice says.
//!
//! Only what the user would want to be interrupted for: an answer they asked for from the phone (or one that took
//! long), a coding session that finished or waits for them, a permission request, a plan running out, money
//! movements, the day's briefing. At most `PER_HOUR` an hour, each one cut to `BODY_MAX` characters.

use std::collections::{HashMap, HashSet, VecDeque};

use crate::events::Event;

pub const PER_HOUR: usize = 30;
pub const BODY_MAX: usize = 300;
/// An answer that took this long is announced even when it was not asked from the phone.
const LONG_TURN_MS: u64 = 120_000;
const HOUR_MS: u64 = 3_600_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    /// `chat`, `session`, `approval`, `usage`, `money`, `briefing` or `focus`: what the phone opens on a tap.
    pub kind: &'static str,
    pub title: String,
    pub body: String,
    /// The agent waits for an answer: delivered even through the phone's quiet modes that allow it.
    pub urgent: bool,
    /// The chat, session or request it is about (empty: none).
    pub target: String,
}

struct Turn {
    started_ms: u64,
    from_phone: bool,
    agent_name: String,
    preview: String,
}

#[derive(Default)]
pub struct Pusher {
    sent: VecDeque<u64>,
    turns: HashMap<String, Turn>,
    /// Chats whose next turn was asked from the phone.
    asked: HashSet<String>,
}

impl Pusher {
    /// The phone sent a message to this chat: its answer is announced when it ends.
    pub fn asked_from_phone(&mut self, chat_id: &str) {
        self.asked.insert(chat_id.to_string());
        if let Some(turn) = self.turns.get_mut(chat_id) {
            turn.from_phone = true;
        }
    }

    /// Follows an event and says whether it deserves a notice (the caller sends it only with the phone away).
    pub fn observe(&mut self, event: &Event, now_ms: u64) -> Option<Notice> {
        match event {
            Event::ChatStarted { chat_id, agent_name, .. } => {
                let from_phone = self.asked.contains(chat_id) || self.turns.get(chat_id).is_some_and(|t| t.from_phone);
                // A hand-off starts a second answer in the same turn: the clock keeps running.
                let started_ms = self.turns.get(chat_id).map_or(now_ms, |t| t.started_ms);
                self.turns.insert(chat_id.clone(), Turn { started_ms, from_phone, agent_name: agent_name.clone(), preview: String::new() });
                None
            }
            Event::ChatDelta { chat_id, text } => {
                if let Some(turn) = self.turns.get_mut(chat_id)
                    && turn.preview.chars().count() < BODY_MAX
                {
                    turn.preview.push_str(text);
                }
                None
            }
            Event::ChatDone { chat_id, .. } => self.ended(chat_id, now_ms, None),
            Event::ChatFailed { chat_id, message } => self.ended(chat_id, now_ms, Some(message)),
            Event::SessionUpdate { session_id, agent, project, state, summary, .. } => {
                let name = agent_name(agent);
                let title = match state.as_str() {
                    "waiting" => format!("{name} te espera en {project}"),
                    "done" => format!("{name} terminó en {project}"),
                    "error" => format!("{name} se detuvo con un error en {project}"),
                    _ => return None,
                };
                Some(notice("session", title, summary, false, session_id))
            }
            Event::ApprovalRequest { request_id, agent, project, title, summary, .. } => {
                let place = if project.is_empty() { String::new() } else { format!(" en {project}") };
                Some(notice("approval", format!("{} pide permiso{place}", agent_name(agent)), &format!("{title}: {summary}"), true, request_id))
            }
            Event::UsageLow { provider, label, left_pct } => {
                Some(notice("usage", "Plan casi agotado".into(), &format!("{} · {label}: queda {left_pct} %", agent_name(provider)), false, ""))
            }
            Event::FinanceRecorded { monto, tipo, concepto, comercio, .. } => {
                let what = if comercio.is_empty() { concepto } else { comercio };
                Some(notice("money", format!("Niko anotó {monto}"), &format!("{tipo} · {what}"), false, ""))
            }
            Event::BudgetAlert { categoria, usado_pct } => {
                Some(notice("money", "Presupuesto".into(), &format!("{categoria} va en {usado_pct} % del mes"), false, ""))
            }
            Event::BriefingReady { count, headline } => {
                let title = if *count == 1 { "1 mensajito nuevo".to_string() } else { format!("{count} mensajitos nuevos") };
                Some(notice("briefing", title, headline, false, ""))
            }
            Event::FocusFinished { minutes } => Some(notice("focus", "Concentración terminada".into(), &format!("Pasaron {minutes} minutos."), false, "")),
            _ => None,
        }
    }

    /// Whether one more notice fits in this hour (and counts it).
    pub fn allow(&mut self, now_ms: u64) -> bool {
        while self.sent.front().is_some_and(|t| now_ms.saturating_sub(*t) >= HOUR_MS) {
            self.sent.pop_front();
        }
        if self.sent.len() >= PER_HOUR {
            return false;
        }
        self.sent.push_back(now_ms);
        true
    }

    fn ended(&mut self, chat_id: &str, now_ms: u64, failure: Option<&String>) -> Option<Notice> {
        self.asked.remove(chat_id);
        let turn = self.turns.remove(chat_id)?;
        if !turn.from_phone && now_ms.saturating_sub(turn.started_ms) < LONG_TURN_MS {
            return None;
        }
        let name = if turn.agent_name.is_empty() { "Buddy" } else { &turn.agent_name };
        Some(match failure {
            None => notice("chat", format!("{name} respondió"), &turn.preview, false, chat_id),
            Some(message) => notice("chat", format!("{name} no pudo responder"), message, false, chat_id),
        })
    }
}

fn notice(kind: &'static str, title: String, body: &str, urgent: bool, target: &str) -> Notice {
    Notice { kind, title: cut(&title, 80), body: cut(body, BODY_MAX), urgent, target: target.to_string() }
}

/// One line of at most `max` characters, with «…» when something was left out.
fn cut(text: &str, max: usize) -> String {
    let line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if line.chars().count() <= max {
        return line;
    }
    let mut out: String = line.chars().take(max - 1).collect();
    out.push('…');
    out
}

/// How an agent or provider is called in a notice.
fn agent_name(id: &str) -> &str {
    match id {
        "claude" => "Claude Code",
        "codex" => "Codex",
        "antigravity" => "Gemini",
        "buddy" => "Buddy",
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn started(chat: &str) -> Event {
        Event::ChatStarted { chat_id: chat.into(), agent: "buddy".into(), agent_name: "Buddy".into(), provider: "claude".into() }
    }
    fn delta(chat: &str, text: &str) -> Event {
        Event::ChatDelta { chat_id: chat.into(), text: text.into() }
    }
    fn done(chat: &str) -> Event {
        Event::ChatDone { chat_id: chat.into(), message_id: 1 }
    }

    #[test]
    fn an_answer_asked_from_the_phone_is_announced_with_its_first_words() {
        let mut p = Pusher::default();
        p.asked_from_phone("c1");
        assert_eq!(p.observe(&started("c1"), 0), None);
        assert_eq!(p.observe(&delta("c1", "El informe\nestá "), 10), None);
        assert_eq!(p.observe(&delta("c1", "listo."), 20), None);
        let n = p.observe(&done("c1"), 5_000).unwrap();
        assert_eq!((n.kind, n.title.as_str(), n.body.as_str(), n.urgent, n.target.as_str()), ("chat", "Buddy respondió", "El informe está listo.", false, "c1"));
        // The next answer in that chat, asked at the desk and short, is not.
        p.observe(&started("c1"), 6_000);
        assert_eq!(p.observe(&done("c1"), 7_000), None);
    }

    #[test]
    fn an_answer_asked_at_the_desk_is_announced_only_when_it_took_long() {
        let mut p = Pusher::default();
        p.observe(&started("c1"), 0);
        assert_eq!(p.observe(&done("c1"), 119_000), None);
        p.observe(&started("c2"), 0);
        // A hand-off to a specialist does not restart the clock.
        p.observe(&Event::ChatStarted { chat_id: "c2".into(), agent: "niko".into(), agent_name: "Niko".into(), provider: "claude".into() }, 100_000);
        assert_eq!(p.observe(&done("c2"), 121_000).unwrap().title, "Niko respondió");
        p.observe(&started("c3"), 0);
        let failed = p.observe(&Event::ChatFailed { chat_id: "c3".into(), message: "Sin conexión".into() }, 130_000).unwrap();
        assert_eq!((failed.title.as_str(), failed.body.as_str()), ("Buddy no pudo responder", "Sin conexión"));
    }

    #[test]
    fn sessions_and_permissions_say_who_and_where() {
        let mut p = Pusher::default();
        let session = |state: &str, summary: &str| Event::SessionUpdate {
            session_id: "s1".into(), agent: "codex".into(), project: "buddy".into(), state: state.into(), cwd: "/p".into(), terminal: String::new(), summary: summary.into(),
        };
        assert_eq!(p.observe(&session("working", ""), 0), None);
        assert_eq!(p.observe(&session("ended", ""), 0), None);
        let waiting = p.observe(&session("waiting", "¿Barras o tarjeta?"), 0).unwrap();
        assert_eq!((waiting.title.as_str(), waiting.body.as_str(), waiting.urgent), ("Codex te espera en buddy", "¿Barras o tarjeta?", false));
        assert_eq!(p.observe(&session("done", "Listo"), 0).unwrap().title, "Codex terminó en buddy");
        let ask = p
            .observe(
                &Event::ApprovalRequest {
                    request_id: "9-1".into(), session_id: "s1".into(), agent: "claude".into(), project: "buddy".into(), title: "Ejecutar un comando".into(),
                    summary: "git push".into(), detail: "git push".into(), can_allow: true, always: String::new(),
                },
                0,
            )
            .unwrap();
        assert_eq!((ask.kind, ask.title.as_str(), ask.body.as_str(), ask.urgent, ask.target.as_str()), ("approval", "Claude Code pide permiso en buddy", "Ejecutar un comando: git push", true, "9-1"));
    }

    #[test]
    fn a_notice_is_one_bounded_line() {
        let mut p = Pusher::default();
        let long = Event::BriefingReady { count: 3, headline: format!("Titular\n{}", "x".repeat(500)) };
        let n = p.observe(&long, 0).unwrap();
        assert_eq!(n.title, "3 mensajitos nuevos");
        assert_eq!(n.body.chars().count(), BODY_MAX);
        assert!(n.body.starts_with("Titular xxx") && n.body.ends_with('…'));
    }

    #[test]
    fn events_that_are_not_news_give_no_notice() {
        let mut p = Pusher::default();
        for e in [
            Event::SettingChanged { key: "x".into() }, Event::MascotState { state: "idle".into() }, Event::UsageChanged,
            Event::MediaCommand { action: "play".into(), uri: String::new() }, Event::ScreenshotRequest { path: "/p".into() },
            Event::ApprovalClosed { request_id: "1-1".into() }, Event::TelegramChanged, Event::NikoChanged,
            Event::FocusChanged { running: true, ends_at: 1 }, Event::ChatQueueChanged { chat_id: "c".into() },
        ] {
            assert_eq!(p.observe(&e, 0), None, "{e:?}");
        }
    }

    #[test]
    fn no_more_than_thirty_notices_an_hour() {
        let mut p = Pusher::default();
        for i in 0..PER_HOUR as u64 {
            assert!(p.allow(i * 1_000), "{i}");
        }
        assert!(!p.allow(40_000), "the 31st of the hour");
        assert!(!p.allow(HOUR_MS - 1));
        assert!(p.allow(HOUR_MS), "the first one is an hour old: one more fits");
        assert!(!p.allow(HOUR_MS + 1));
    }
}
