//! Chats with Buddy: a message goes in, the orchestrated answer comes out as events, and both are saved.
//!
//! One turn: Buddy answers with its provider (Claude by default). If its answer is a hand-off line, nothing of it is
//! shown and the specialist answers instead. When a provider is missing or out of usage before any text arrived,
//! the turn moves to the next installed provider and says so. The mascot follows along: think → work → done/error.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::events::{Event, EventBus};
use crate::orchestrator::{self, Agent, ORCHESTRATOR};
use crate::providers::{Cancel, FailureKind, Provider, ProviderId, TurnEvent, TurnRequest};
use crate::store::{NewMessage, SourceLink, Store};
use crate::CoreError;

pub struct ChatEngine {
    data_dir: PathBuf,
    store: Arc<Mutex<Store>>,
    bus: Arc<EventBus>,
    providers: Vec<Arc<dyn Provider>>,
    running: Mutex<HashMap<String, Cancel>>,
}

/// What one agent's turn produced.
struct Answer {
    text: String,
    /// How much of `text` was already shown (Buddy's is held while it could be a hand-off line).
    shown: usize,
    sources: Vec<SourceLink>,
    provider: ProviderId,
    failure: Option<String>,
}

impl ChatEngine {
    pub fn new(data_dir: PathBuf, store: Arc<Mutex<Store>>, bus: Arc<EventBus>, providers: Vec<Arc<dyn Provider>>) -> Self {
        Self { data_dir, store, bus, providers, running: Mutex::new(HashMap::new()) }
    }

    pub fn agents(&self) -> Vec<Agent> {
        orchestrator::load(&self.data_dir)
    }

    /// Saves the user's message and answers it on a background thread. Returns the chat id (new when `chat_id` is
    /// None). A turn already running in that chat is stopped first.
    pub fn send(self: &Arc<Self>, chat_id: Option<String>, text: String) -> Result<String, CoreError> {
        let text = text.trim().to_string();
        let chat_id = chat_id.filter(|c| !c.is_empty()).unwrap_or_else(new_chat_id);
        self.cancel(&chat_id);
        let previous = self.lock().messages(&chat_id)?;
        {
            let store = self.lock();
            store.ensure_chat(&chat_id, &title_for(&text))?;
            store.add_message(NewMessage {
                chat_id: &chat_id,
                role: "user",
                agent: ORCHESTRATOR,
                provider: None,
                text: &text,
                sources: &[],
                failed: false,
            })?;
        }
        let cancel = Cancel::default();
        self.running.lock().unwrap().insert(chat_id.clone(), cancel.clone());
        let engine = self.clone();
        let id = chat_id.clone();
        std::thread::Builder::new()
            .name("buddy-turn".into())
            .spawn(move || {
                engine.turn(&id, &text, previous.last().map(|m| (m.agent.clone(), m.text.clone())), &cancel);
                let mut running = engine.running.lock().unwrap();
                if running.get(&id).is_some_and(|c| c.same(&cancel)) {
                    running.remove(&id);
                }
            })
            .map_err(|e| CoreError::Io(e.to_string()))?;
        Ok(chat_id)
    }

    pub fn cancel(&self, chat_id: &str) {
        if let Some(cancel) = self.running.lock().unwrap().remove(chat_id) {
            cancel.cancel();
        }
    }

    pub fn statuses(&self) -> Vec<crate::providers::ProviderStatus> {
        self.providers
            .iter()
            .map(|p| crate::providers::ProviderStatus {
                id: p.id(),
                name: p.id().display_name().into(),
                installed: p.installed(),
            })
            .collect()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Store> {
        self.store.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn emit(&self, event: Event) {
        self.bus.publish(event);
    }

    fn mascot(&self, state: &str) {
        self.emit(Event::MascotState { state: state.into() });
    }

    fn turn(&self, chat_id: &str, question: &str, last: Option<(String, String)>, cancel: &Cancel) {
        let agents = self.agents();
        let buddy = agents.iter().find(|a| a.id == ORCHESTRATOR).cloned().expect("orchestrator::load always has buddy");
        let known: Vec<&str> = agents.iter().map(|a| a.id.as_str()).collect();
        self.mascot("think");

        // Buddy's turn: a specialist's last answer rides along as a note, so the conversation keeps making sense.
        let mut prompt = String::new();
        if let Some((agent, text)) = &last {
            if agent != ORCHESTRATOR {
                let name = agents.iter().find(|a| &a.id == agent).map_or(agent.as_str(), |a| a.name.as_str());
                prompt.push_str(&orchestrator::followup_note(name, text));
            }
        }
        prompt.push_str(question);
        let system = format!("{}{}", buddy.prompt, orchestrator::roster_prompt(&agents));
        let answer = self.run_agent(chat_id, &buddy, &prompt, &system, cancel, true);

        let (agent, answer) = match orchestrator::parse_handoff(&answer.text, &known) {
            Some((id, task)) if answer.failure.is_none() && !cancel.is_cancelled() => {
                let specialist = agents.iter().find(|a| a.id == id).cloned().expect("parse_handoff checks known ids");
                let prompt = orchestrator::task_prompt(&specialist.name, &task, question);
                let answer = self.run_agent(chat_id, &specialist, &prompt, &specialist.prompt, cancel, false);
                (specialist, answer)
            }
            _ => {
                // Held text that turned out not to be a hand-off is shown now.
                if answer.shown < answer.text.len() {
                    self.emit(Event::ChatDelta { chat_id: chat_id.into(), text: answer.text[answer.shown..].to_string() });
                }
                (buddy, answer)
            }
        };

        let failed = answer.failure.is_some();
        let text = match (&answer.failure, answer.text.trim().is_empty()) {
            (Some(failure), true) => failure.clone(),
            _ => answer.text.clone(),
        };
        let saved = self.lock().add_message(NewMessage {
            chat_id,
            role: "assistant",
            agent: &agent.id,
            provider: Some(answer.provider.as_str()),
            text: &text,
            sources: &answer.sources,
            failed,
        });
        match (saved, &answer.failure) {
            (Ok(_), Some(message)) => {
                self.emit(Event::ChatFailed { chat_id: chat_id.into(), message: message.clone() });
                self.mascot("error");
            }
            (Ok(message_id), None) => {
                self.emit(Event::ChatDone { chat_id: chat_id.into(), message_id });
                self.mascot(if cancel.is_cancelled() { "idle" } else { "done" });
            }
            (Err(e), _) => {
                self.emit(Event::ChatFailed { chat_id: chat_id.into(), message: e.to_string() });
                self.mascot("error");
            }
        }
    }

    /// Runs one agent, trying its provider first and the other installed ones after a missing CLI or no usage
    /// (only while nothing has been shown). `hold_handoff`: Buddy's text is held while it could be a hand-off line.
    fn run_agent(&self, chat_id: &str, agent: &Agent, prompt: &str, system: &str, cancel: &Cancel, hold_handoff: bool) -> Answer {
        let mut order: Vec<Arc<dyn Provider>> = self.providers.iter().filter(|p| p.id() == agent.provider).cloned().collect();
        order.extend(self.providers.iter().filter(|p| p.id() != agent.provider && p.installed()).cloned());
        let mut last_failure = None;
        let mut provider_used = agent.provider;
        for (attempt, provider) in order.iter().enumerate() {
            provider_used = provider.id();
            if attempt > 0 {
                self.emit(Event::ChatTool {
                    chat_id: chat_id.into(),
                    name: "Cambio".into(),
                    summary: format!("Sigo con {}", provider.id().display_name()),
                });
            }
            self.emit(Event::ChatStarted {
                chat_id: chat_id.into(),
                agent: agent.id.clone(),
                agent_name: agent.name.clone(),
                provider: provider.id().as_str().into(),
            });
            let same_provider = provider.id() == agent.provider;
            let request = TurnRequest {
                prompt: prompt.into(),
                system: system.into(),
                workspace: orchestrator::workspace(&self.data_dir, &agent.id),
                resume: self.lock().session(chat_id, &agent.id, provider.id().as_str()).ok().flatten(),
                // A model name belongs to its provider; another provider uses its own default.
                model: agent.model.clone().filter(|_| same_provider),
                effort: agent.effort.clone(),
            };
            let mut text = String::new();
            let mut shown = 0usize;
            let mut sources = Vec::new();
            let mut failure = None;
            let mut worked = false;
            provider.run(&request, cancel, &mut |event| match event {
                TurnEvent::Session(id) => {
                    let _ = self.lock().set_session(chat_id, &agent.id, provider.id().as_str(), &id);
                }
                TurnEvent::Delta(delta) => {
                    text.push_str(&delta);
                    if !(hold_handoff && orchestrator::handoff_pending(&text)) {
                        self.emit(Event::ChatDelta { chat_id: chat_id.into(), text: text[shown..].to_string() });
                        shown = text.len();
                    }
                }
                TurnEvent::Tool { name, summary } => {
                    if !worked {
                        worked = true;
                        self.mascot("work");
                    }
                    self.emit(Event::ChatTool { chat_id: chat_id.into(), name, summary });
                }
                TurnEvent::Source { title, url } => {
                    if !sources.iter().any(|s: &SourceLink| s.url == url) {
                        self.emit(Event::ChatSource { chat_id: chat_id.into(), title: title.clone(), url: url.clone() });
                        sources.push(SourceLink { title, url });
                    }
                }
                TurnEvent::Done => {}
                TurnEvent::Failed(f) => failure = Some(f),
            });
            match failure {
                Some(f) if text.is_empty() && matches!(f.kind, FailureKind::Missing | FailureKind::Limit) && !cancel.is_cancelled() => {
                    last_failure = Some(f.summary(provider.id()));
                    continue;
                }
                Some(f) => {
                    return Answer { text, shown, sources, provider: provider.id(), failure: Some(f.summary(provider.id())) };
                }
                None => return Answer { text, shown, sources, provider: provider.id(), failure: None },
            }
        }
        Answer {
            text: String::new(),
            shown: 0,
            sources: Vec::new(),
            provider: provider_used,
            failure: Some(last_failure.unwrap_or_else(|| {
                "No encuentro Claude ni Codex en este equipo. Instala uno e inicia sesión.".into()
            })),
        }
    }
}

/// The first line of the first message, short.
fn title_for(text: &str) -> String {
    let line = text.lines().find(|l| !l.trim().is_empty()).unwrap_or("Chat").trim();
    let mut title: String = line.chars().take(60).collect();
    if line.chars().count() > 60 {
        title.push('…');
    }
    title
}

fn new_chat_id() -> String {
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    format!("c{nanos:x}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::Failure;
    use std::sync::mpsc::Receiver;
    use std::time::Duration;

    /// A provider that plays back a script of events.
    struct Fake {
        id: ProviderId,
        installed: bool,
        script: Mutex<Vec<Vec<TurnEvent>>>,
        prompts: Mutex<Vec<(String, String)>>,
    }

    impl Fake {
        fn new(id: ProviderId, scripts: Vec<Vec<TurnEvent>>) -> Arc<Self> {
            Arc::new(Self { id, installed: true, script: Mutex::new(scripts), prompts: Mutex::default() })
        }
    }

    impl Provider for Fake {
        fn id(&self) -> ProviderId {
            self.id
        }
        fn installed(&self) -> bool {
            self.installed
        }
        fn run(&self, request: &TurnRequest, _: &Cancel, emit: &mut dyn FnMut(TurnEvent)) {
            self.prompts.lock().unwrap().push((request.prompt.clone(), request.system.clone()));
            let mut scripts = self.script.lock().unwrap();
            let events = if scripts.is_empty() { vec![TurnEvent::Done] } else { scripts.remove(0) };
            for e in events {
                emit(e);
            }
        }
    }

    fn engine(providers: Vec<Arc<dyn Provider>>) -> (Arc<ChatEngine>, Receiver<Event>, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let bus = Arc::new(EventBus::default());
        let rx = bus.subscribe();
        let store = Arc::new(Mutex::new(Store::open_in_memory().unwrap()));
        (Arc::new(ChatEngine::new(dir.path().into(), store, bus, providers)), rx, dir)
    }

    /// Events until the turn ends.
    fn until_end(rx: &Receiver<Event>) -> Vec<Event> {
        let mut out = vec![];
        loop {
            let e = rx.recv_timeout(Duration::from_secs(5)).expect("the turn ends");
            let end = matches!(e, Event::MascotState { ref state } if ["done", "error", "idle"].contains(&state.as_str()));
            out.push(e);
            if end {
                return out;
            }
        }
    }

    fn deltas(events: &[Event]) -> String {
        events.iter().filter_map(|e| if let Event::ChatDelta { text, .. } = e { Some(text.as_str()) } else { None }).collect()
    }

    #[test]
    fn answers_stream_and_are_saved() {
        let claude = Fake::new(ProviderId::Claude, vec![vec![
            TurnEvent::Session("s1".into()),
            TurnEvent::Delta("Hola".into()),
            TurnEvent::Delta(", ¿qué tal?".into()),
            TurnEvent::Done,
        ]]);
        let (engine, rx, _dir) = engine(vec![claude.clone()]);
        let chat = engine.send(None, "Hola Buddy".into()).unwrap();
        let events = until_end(&rx);
        assert_eq!(events[0], Event::MascotState { state: "think".into() });
        assert_eq!(deltas(&events), "Hola, ¿qué tal?");
        assert!(events.iter().any(|e| matches!(e, Event::ChatDone { .. })));
        assert_eq!(events.last(), Some(&Event::MascotState { state: "done".into() }));
        let store = engine.lock();
        let messages = store.messages(&chat).unwrap();
        assert_eq!((messages[0].role.as_str(), messages[1].text.as_str()), ("user", "Hola, ¿qué tal?"));
        assert_eq!(store.session(&chat, "buddy", "claude").unwrap().as_deref(), Some("s1"));
        assert_eq!(store.chats(5).unwrap()[0].title, "Hola Buddy");
    }

    #[test]
    fn a_hand_off_is_never_shown_and_the_specialist_answers() {
        let claude = Fake::new(ProviderId::Claude, vec![
            vec![TurnEvent::Delta("[[pasar:".into()), TurnEvent::Delta("parley]] Analiza el clásico".into()), TurnEvent::Done],
            vec![TurnEvent::Delta("Gana el Madrid (confianza media).".into()), TurnEvent::Done],
        ]);
        let (engine, rx, _dir) = engine(vec![claude.clone()]);
        let chat = engine.send(None, "¿Quién gana el clásico?".into()).unwrap();
        let events = until_end(&rx);
        assert_eq!(deltas(&events), "Gana el Madrid (confianza media).");
        let started: Vec<&str> = events
            .iter()
            .filter_map(|e| if let Event::ChatStarted { agent, .. } = e { Some(agent.as_str()) } else { None })
            .collect();
        assert_eq!(started, ["buddy", "parley"]);
        let prompts = claude.prompts.lock().unwrap();
        assert!(prompts[0].1.contains("[[pasar:"), "Buddy knows the team");
        assert!(prompts[1].0.starts_with("[Encargo de Buddy para PARLEY]\nAnaliza el clásico"));
        assert_eq!(engine.lock().messages(&chat).unwrap()[1].agent, "parley");
    }

    #[test]
    fn after_a_hand_off_buddy_gets_a_note() {
        let claude = Fake::new(ProviderId::Claude, vec![
            vec![TurnEvent::Delta("[[pasar:parley]] x".into()), TurnEvent::Done],
            vec![TurnEvent::Delta("Pick: over 2.5".into()), TurnEvent::Done],
            vec![TurnEvent::Delta("De nada".into()), TurnEvent::Done],
        ]);
        let (engine, rx, _dir) = engine(vec![claude.clone()]);
        let chat = engine.send(None, "pick".into()).unwrap();
        until_end(&rx);
        engine.send(Some(chat), "gracias".into()).unwrap();
        until_end(&rx);
        let prompts = claude.prompts.lock().unwrap();
        assert!(prompts[2].0.starts_with("[Nota de Buddy, no del usuario]") && prompts[2].0.ends_with("gracias"));
    }

    #[test]
    fn out_of_usage_moves_to_the_next_provider() {
        let claude = Fake::new(ProviderId::Claude, vec![vec![TurnEvent::Failed(Failure::new("usage limit reached"))]]);
        let codex = Fake::new(ProviderId::Codex, vec![vec![TurnEvent::Delta("Aquí Codex".into()), TurnEvent::Done]]);
        let (engine, rx, _dir) = engine(vec![claude, codex]);
        let chat = engine.send(None, "hola".into()).unwrap();
        let events = until_end(&rx);
        assert_eq!(deltas(&events), "Aquí Codex");
        assert!(events.iter().any(|e| matches!(e, Event::ChatTool { name, .. } if name == "Cambio")));
        assert_eq!(engine.lock().messages(&chat).unwrap()[1].provider.as_deref(), Some("codex"));
    }

    #[test]
    fn other_failures_are_reported_and_saved() {
        let claude = Fake::new(ProviderId::Claude, vec![vec![TurnEvent::Failed(Failure::new("Not logged in"))]]);
        let (engine, rx, _dir) = engine(vec![claude]);
        let chat = engine.send(None, "hola".into()).unwrap();
        let events = until_end(&rx);
        assert!(events.iter().any(|e| matches!(e, Event::ChatFailed { message, .. } if message.contains("Conecta tu cuenta"))));
        assert_eq!(events.last(), Some(&Event::MascotState { state: "error".into() }));
        assert!(engine.lock().messages(&chat).unwrap()[1].failed);
    }

    #[test]
    fn an_answer_that_only_looks_like_a_hand_off_is_shown() {
        let claude = Fake::new(ProviderId::Claude, vec![vec![TurnEvent::Delta("[[pasar:nadie]] hola".into()), TurnEvent::Done]]);
        let (engine, rx, _dir) = engine(vec![claude]);
        engine.send(None, "x".into()).unwrap();
        assert_eq!(deltas(&until_end(&rx)), "[[pasar:nadie]] hola");
    }

    #[test]
    fn titles_are_short() {
        assert_eq!(title_for("\n  Hola  \nmundo"), "Hola");
        assert_eq!(title_for(&"a".repeat(80)).chars().count(), 61);
    }
}
