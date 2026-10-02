//! Typed events from the core to the apps. Apps draw them; nothing polls on a timer while idle.
//!
//! One channel per subscriber: a slow or gone subscriber never blocks the others, and a closed one is dropped
//! on the next publish.

use std::sync::Mutex;
use std::sync::mpsc::{Receiver, Sender, channel};

/// Phase 0 carries only what exists today. Later phases add `TextDelta`, `ToolStatus`, `SessionFinished`,
/// `NeedsApproval`, `UsageLow`, `PickNew`, `BriefingReady`… (see docs/ARCHITECTURE.md).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum Event {
    /// A setting was saved (the key only; values may be private).
    SettingChanged { key: String },
    /// The mascot should show another state (`idle`, `think`, `work`…), by the priority rules of the core.
    MascotState { state: String },
    /// An agent starts answering in a chat (Buddy first; a specialist after a hand-off).
    ChatStarted { chat_id: String, agent: String, agent_name: String, provider: String },
    /// Text of the answer, as it arrives.
    ChatDelta { chat_id: String, text: String },
    /// The agent is using a tool (searching, reading a page).
    ChatTool { chat_id: String, name: String, summary: String },
    ChatSource { chat_id: String, title: String, url: String },
    /// The answer is complete and saved.
    ChatDone { chat_id: String, message_id: i64 },
    ChatFailed { chat_id: String, message: String },
    /// The focus timer started (`running`, ends at unix seconds) or stopped.
    FocusChanged { running: bool, ends_at: i64 },
    /// A focus block of `minutes` ended.
    FocusFinished { minutes: u32 },
    /// A Claude Code / Codex session changed state (from its hooks): `working`, `waiting` (it needs the user),
    /// `done` (the turn finished), `error` (the turn failed) or `ended` (the session closed; it is forgotten).
    SessionUpdate { session_id: String, agent: String, project: String, state: String },
    /// An agent asks permission for a tool (from its PermissionRequest hook). Answer with `answer_approval`; the
    /// agent waits until then, up to ~110 s, and otherwise asks in its own terminal. `can_allow` is false when the
    /// request arrived cut short: only "deny" (or the terminal) is offered then.
    ApprovalRequest {
        request_id: String,
        session_id: String,
        agent: String,
        project: String,
        title: String,
        summary: String,
        detail: String,
        can_allow: bool,
    },
    /// The approval card must go: answered, timed out, or the agent stopped waiting.
    ApprovalClosed { request_id: String },
}

#[derive(Default)]
pub struct EventBus {
    subscribers: Mutex<Vec<Sender<Event>>>,
}

impl EventBus {
    pub fn subscribe(&self) -> Receiver<Event> {
        let (tx, rx) = channel();
        self.lock().push(tx);
        rx
    }

    pub fn publish(&self, event: Event) {
        self.lock().retain(|tx| tx.send(event.clone()).is_ok());
    }

    pub fn subscriber_count(&self) -> usize {
        self.lock().len()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<Sender<Event>>> {
        self.subscribers.lock().unwrap_or_else(|p| p.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_subscriber_gets_every_event_in_order() {
        let bus = EventBus::default();
        let a = bus.subscribe();
        let b = bus.subscribe();
        bus.publish(Event::MascotState { state: "think".into() });
        bus.publish(Event::MascotState { state: "idle".into() });
        for rx in [&a, &b] {
            assert_eq!(rx.try_recv().unwrap(), Event::MascotState { state: "think".into() });
            assert_eq!(rx.try_recv().unwrap(), Event::MascotState { state: "idle".into() });
            assert!(rx.try_recv().is_err());
        }
    }

    #[test]
    fn a_gone_subscriber_is_dropped_without_hurting_the_rest() {
        let bus = EventBus::default();
        let kept = bus.subscribe();
        drop(bus.subscribe());
        assert_eq!(bus.subscriber_count(), 2);
        bus.publish(Event::SettingChanged { key: "x".into() });
        assert_eq!(bus.subscriber_count(), 1);
        assert!(kept.try_recv().is_ok());
    }

    #[test]
    fn serializes_for_the_web_frontend() {
        let json = serde_json::to_string(&Event::MascotState { state: "work".into() }).unwrap();
        assert_eq!(json, r#"{"type":"mascotState","state":"work"}"#);
    }
}
