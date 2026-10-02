//! Providers: the user's own subscriptions through their CLIs. Claude (Claude Code, `claude -p` stream-json),
//! Codex (`codex app-server`, JSON-RPC over stdio) and, in phase 3, Gemini through Antigravity (`agy -p`).
//! Ported from MIKA (ClaudeTurn/ClaudeStreamParser, codex_server.rs). A turn runs on its own thread and reports
//! `TurnEvent`s; nothing here knows about chats, windows or the mascot.

pub mod claude;
pub mod codex;
pub mod process;

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum ProviderId {
    Claude,
    Codex,
    Antigravity,
}

impl ProviderId {
    pub fn as_str(self) -> &'static str {
        match self {
            ProviderId::Claude => "claude",
            ProviderId::Codex => "codex",
            ProviderId::Antigravity => "antigravity",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_lowercase().as_str() {
            "claude" => Some(ProviderId::Claude),
            "codex" => Some(ProviderId::Codex),
            "antigravity" | "gemini" => Some(ProviderId::Antigravity),
            _ => None,
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            ProviderId::Claude => "Claude",
            ProviderId::Codex => "Codex",
            ProviderId::Antigravity => "Gemini",
        }
    }
}

/// One turn for one agent: the prompt goes in as data, the agent's instructions as the system prompt.
#[derive(Debug, Clone, Default)]
pub struct TurnRequest {
    pub prompt: String,
    pub system: String,
    /// The agent's own folder: the only place its file tools may look.
    pub workspace: PathBuf,
    /// The provider's conversation to continue (Claude session id, Codex thread id).
    pub resume: Option<String>,
    pub model: Option<String>,
    pub effort: Option<String>,
    /// Files the user attached, already copied into Buddy's attachments folder (the only files a turn may read).
    pub attachments: Vec<PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum FailureKind {
    Auth,
    Limit,
    Missing,
    Other,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Failure {
    pub kind: FailureKind,
    pub message: String,
}

impl Failure {
    pub fn new(message: impl Into<String>) -> Self {
        let message = message.into();
        Self { kind: classify(&message), message }
    }

    /// One short line in Spanish for the chat.
    pub fn summary(&self, provider: ProviderId) -> String {
        let name = provider.display_name();
        match self.kind {
            FailureKind::Auth => format!("Conecta tu cuenta de {name} (inicia sesión en su app o CLI)."),
            FailureKind::Limit => format!("Se acabó el uso de {name} por ahora. {}", first_line(&self.message)),
            FailureKind::Missing => format!("No encuentro {name} instalado en este equipo."),
            FailureKind::Other => format!("{name} no pudo responder: {}", first_line(&self.message)),
        }
    }
}

fn first_line(s: &str) -> String {
    s.lines().next().unwrap_or("").chars().take(160).collect()
}

/// Order matters: a limit message can also mention "login".
pub fn classify(text: &str) -> FailureKind {
    let t = text.to_lowercase();
    if t.contains("limit") || t.contains("quota") || t.contains("usage") && t.contains("reached") {
        FailureKind::Limit
    } else if ["not logged in", "/login", "unauthorized", "authenticat", "sign in", "log in"].iter().any(|k| t.contains(k)) {
        FailureKind::Auth
    } else {
        FailureKind::Other
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum TurnEvent {
    /// The provider's conversation id, to continue it next turn.
    Session(String),
    Delta(String),
    Tool { name: String, summary: String },
    Source { title: String, url: String },
    /// Plan figures the provider reported during the turn (Claude's `rate_limit_info`, Codex's rate limits).
    Usage(serde_json::Value),
    Done,
    Failed(Failure),
}

/// Set from any thread to stop a turn (Stop button, new chat, quitting).
#[derive(Clone, Default, Debug)]
pub struct Cancel(Arc<AtomicBool>);

impl Cancel {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
    pub fn same(&self, other: &Cancel) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

pub trait Provider: Send + Sync {
    fn id(&self) -> ProviderId;
    /// Whether its CLI is installed (no network, no model).
    fn installed(&self) -> bool;
    /// Gets ready for a turn like `request` (start-up done while the user types). Optional.
    fn prewarm(&self, _request: &TurnRequest) {}
    /// Runs one turn to the end, calling `emit` as events arrive. Always ends with `Done` or `Failed`.
    fn run(&self, request: &TurnRequest, cancel: &Cancel, emit: &mut dyn FnMut(TurnEvent));
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct ProviderStatus {
    pub id: ProviderId,
    pub name: String,
    pub installed: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_cli_errors() {
        assert_eq!(classify("Claude AI usage limit reached|1759"), FailureKind::Limit);
        assert_eq!(classify("Invalid API key · Please run /login"), FailureKind::Auth);
        assert_eq!(classify("Not logged in"), FailureKind::Auth);
        assert_eq!(classify("socket hang up"), FailureKind::Other);
    }

    #[test]
    fn ids_round_trip() {
        for id in [ProviderId::Claude, ProviderId::Codex, ProviderId::Antigravity] {
            assert_eq!(ProviderId::parse(id.as_str()), Some(id));
        }
        assert_eq!(ProviderId::parse("Gemini"), Some(ProviderId::Antigravity));
        assert_eq!(ProviderId::parse("gpt"), None);
    }
}
