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
    /// The folders the user authorized (read, or read and edit).
    pub folders: Vec<crate::folders::AuthorizedFolder>,
    /// When set, the agent may ask to run commands; each one waits for the user's click (see `sessions` gate).
    pub gate: Option<Gate>,
    /// When set, the agent can create Word, Excel and PowerPoint files (Buddy's MCP server, `buddy-hook --mcp`).
    pub office: Option<Office>,
}

/// Buddy's Office tools: the relay that serves them and the folder the files go to (never anywhere else).
#[derive(Debug, Clone, PartialEq)]
pub struct Office {
    pub relay: PathBuf,
    pub dir: PathBuf,
    /// `<data>/skills`, read by the `use_skill` tool.
    pub skills: PathBuf,
    /// How the music tools reach the app (none: they are not offered).
    pub link: Option<Link>,
}

/// This run's secret and Buddy's data folder (where the relay finds the socket), handed to the MCP server through
/// its environment, never its command line.
#[derive(Debug, Clone, PartialEq)]
pub struct Link {
    pub token: String,
    pub data_dir: PathBuf,
}

impl Office {
    /// The tool names as Claude Code exposes them (Office, music, skills).
    pub const TOOLS: [&'static str; 9] = [
        "mcp__buddy__create_document",
        "mcp__buddy__create_spreadsheet",
        "mcp__buddy__create_presentation",
        "mcp__buddy__media_control",
        "mcp__buddy__media_play",
        "mcp__buddy__media_search",
        "mcp__buddy__spotify_search",
        "mcp__buddy__now_playing",
        "mcp__buddy__use_skill",
    ];

    pub fn server(&self) -> serde_json::Value {
        serde_json::json!({ "command": self.relay, "args": ["--mcp", "--out", self.dir, "--skills", self.skills] })
    }

    /// The variables the MCP server needs for the music tools.
    pub fn env(&self) -> Vec<(&'static str, String)> {
        match &self.link {
            Some(link) => vec![("BUDDY_GATE_TOKEN", link.token.clone()), ("BUDDY_DATA_DIR", link.data_dir.to_string_lossy().into())],
            None => vec![],
        }
    }
}

impl TurnRequest {
    /// The attached files that are images (already shrunk by `images::prepare`). Every provider hands these over
    /// in its own way: Claude as `image` blocks, Codex as `localImage`, Gemini by path.
    pub fn images(&self) -> Vec<&PathBuf> {
        self.attachments.iter().filter(|p| crate::images::is_image(p)).collect()
    }
}

/// How a turn reaches the command gate: the relay to run as the PreToolUse hook, the secret of this run of the app,
/// and Buddy's data folder (where the relay finds the socket).
#[derive(Debug, Clone, PartialEq)]
pub struct Gate {
    pub relay: PathBuf,
    pub token: String,
    pub data_dir: PathBuf,
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
    /// Account exhaustion only: transient throttling and context limits must not rerun tools elsewhere.
    pub fn is_no_usage(&self) -> bool {
        let text = self.message.to_lowercase();
        [
            "usage limit",
            "usage_limit",
            "usagelimitexceeded",
            "session limit",
            "weekly limit",
            "5-hour limit",
            "quota",
            "out of credits",
            "insufficient credits",
            "credit balance",
            "hit your limit",
            "reached your limit",
        ]
        .iter()
        .any(|fragment| text.contains(fragment))
    }

    pub fn new(message: impl Into<String>) -> Self {
        let message = message.into();
        Self { kind: classify(&message), message }
    }

    /// One short line in Spanish for the chat.
    pub fn summary(&self, provider: ProviderId) -> String {
        let name = provider.display_name();
        match self.kind {
            FailureKind::Auth => {
                format!("Conecta tu cuenta de {name} (inicia sesión en su app o CLI).")
            }
            FailureKind::Limit => format!("Se acabó el uso de {name} por ahora. {}", first_line(&self.message)),
            FailureKind::Missing => format!("No encuentro {name} instalado en este equipo."),
            FailureKind::Other => {
                format!("{name} no pudo responder: {}", first_line(&self.message))
            }
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
    Tool {
        name: String,
        summary: String,
    },
    Source {
        title: String,
        url: String,
    },
    /// Plan figures the provider reported during the turn (Claude's `rate_limit_info`, Codex's rate limits).
    Usage(serde_json::Value),
    /// What the turn spent (for the token meter).
    Tokens(TokenCount),
    Done,
    Failed(Failure),
}

/// Tokens a turn spent, as the provider reported them.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TokenCount {
    pub input: i64,
    pub output: i64,
    /// Input served from the prompt cache (cheaper).
    pub cached: i64,
    /// What it would cost on the API, when the provider says (Claude does; the subscription pays nothing extra).
    pub cost_usd: Option<f64>,
    pub model: Option<String>,
}

impl TokenCount {
    /// Claude Code's `result` event: `usage` and `total_cost_usd`.
    pub fn from_claude_result(result: &serde_json::Value) -> Option<Self> {
        let usage = result.get("usage")?;
        let n = |k: &str| usage[k].as_i64().unwrap_or(0);
        Some(Self {
            input: n("input_tokens") + n("cache_creation_input_tokens"),
            output: n("output_tokens"),
            cached: n("cache_read_input_tokens"),
            cost_usd: result["total_cost_usd"].as_f64(),
            model: result["modelUsage"].as_object().and_then(|m| m.keys().next().cloned()),
        })
    }
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
    /// Whether it can look at images (`TurnRequest::images`). A provider that cannot is skipped for turns that
    /// carry images while another one can take them.
    fn sees_images(&self) -> bool {
        false
    }
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
