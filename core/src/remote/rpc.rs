//! What the phone may ask the core for: a fixed list. `parse` turns a call's name and arguments into a `Call`, or
//! refuses it; there is no other way in, so anything not listed here (settings, keys, folders, hooks, permissions,
//! deleting chats) cannot be reached from the phone at all.

use serde_json::Value;

/// Said for anything outside the list.
pub const REFUSED: &str = "Esa acción no se puede hacer desde el teléfono.";

const TEXT_MAX: usize = 20_000;
const LIST_MAX: u32 = 200;

#[derive(Debug, Clone, PartialEq)]
pub enum Call {
    Hello,
    Chats { limit: u32 },
    SearchChats { query: String, limit: u32 },
    Messages { chat_id: String },
    ImagePreview { path: String },
    Agents,
    AgentSprite { agent_id: String },
    Sprite { id: String },
    ChatCommands,
    ChatSuggestions,
    QueuedMessages { chat_id: String },
    Sessions,
    Usage,
    Briefing,
    FocusStatus,
    VoiceVocabulary,
    SendMessage { chat_id: Option<String>, text: String },
    CancelChat { chat_id: String },
    Regenerate { chat_id: String },
    RemoveQueued { chat_id: String, message_id: String },
    AnswerApproval { request_id: String, allow: bool },
    FocusStart { minutes: u32 },
    FocusStop,
}

impl Call {
    /// The line for the user's log, for the calls that do something (reading leaves no line).
    pub fn acts(&self) -> Option<String> {
        Some(match self {
            Call::SendMessage { .. } => "iPhone envió un mensaje".into(),
            Call::CancelChat { .. } => "iPhone detuvo una respuesta".into(),
            Call::Regenerate { .. } => "iPhone pidió repetir una respuesta".into(),
            Call::RemoveQueued { .. } => "iPhone quitó un mensaje pendiente".into(),
            Call::AnswerApproval { allow: true, .. } => "iPhone permitió un permiso".into(),
            Call::AnswerApproval { allow: false, .. } => "iPhone rechazó un permiso".into(),
            Call::FocusStart { minutes } => format!("iPhone empezó {minutes} min de concentración"),
            Call::FocusStop => "iPhone detuvo la concentración".into(),
            _ => return None,
        })
    }
}

/// The core, as the phone reaches it: one typed call at a time.
pub trait Host: Send + Sync {
    fn run(&self, call: Call) -> Result<Value, String>;
}

/// A call by name with its arguments checked, or the reason it is refused.
pub fn parse(name: &str, args: &Value) -> Result<Call, String> {
    let limit = || args["limit"].as_u64().map_or(50, |n| n.clamp(1, LIST_MAX as u64) as u32);
    Ok(match name {
        "hello" => Call::Hello,
        "chats" => Call::Chats { limit: limit() },
        "search_chats" => Call::SearchChats { query: text(args, "query", 200)?, limit: limit() },
        "messages" => Call::Messages { chat_id: id(args, "chatId")? },
        "image_preview" => Call::ImagePreview { path: path(args, "path")? },
        "agents" => Call::Agents,
        "agent_sprite" => Call::AgentSprite { agent_id: id(args, "agentId")? },
        "sprite" => Call::Sprite { id: id(args, "id")? },
        "chat_commands" => Call::ChatCommands,
        "chat_suggestions" => Call::ChatSuggestions,
        "queued_messages" => Call::QueuedMessages { chat_id: id(args, "chatId")? },
        "sessions" => Call::Sessions,
        "usage" => Call::Usage,
        "briefing" => Call::Briefing,
        "focus_status" => Call::FocusStatus,
        "voice_vocabulary" => Call::VoiceVocabulary,
        "send_message" => Call::SendMessage {
            chat_id: match &args["chatId"] {
                Value::Null => None,
                _ => Some(id(args, "chatId")?),
            },
            text: text(args, "text", TEXT_MAX)?,
        },
        "cancel_chat" => Call::CancelChat { chat_id: id(args, "chatId")? },
        "regenerate" => Call::Regenerate { chat_id: id(args, "chatId")? },
        "remove_queued" => Call::RemoveQueued { chat_id: id(args, "chatId")?, message_id: id(args, "messageId")? },
        "answer_approval" => Call::AnswerApproval {
            request_id: id(args, "requestId")?,
            allow: args["allow"].as_bool().ok_or("Falta «allow».")?,
        },
        "focus_start" => Call::FocusStart { minutes: args["minutes"].as_u64().filter(|m| (1..=180).contains(m)).ok_or("Minutos entre 1 y 180.")? as u32 },
        "focus_stop" => Call::FocusStop,
        _ => return Err(REFUSED.into()),
    })
}

/// An identifier (a chat, an agent, a request): letters, digits, `-` and `_` only. It may end up in a path
/// (`adjuntos/<chat>`), so nothing that could leave its folder is ever accepted.
fn id(args: &Value, key: &str) -> Result<String, String> {
    args[key]
        .as_str()
        .filter(|s| (1..=80).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'))
        .map(str::to_string)
        .ok_or_else(|| format!("«{key}» no es un identificador válido."))
}

fn text(args: &Value, key: &str, max: usize) -> Result<String, String> {
    args[key]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty() && s.chars().count() <= max)
        .map(str::to_string)
        .ok_or_else(|| format!("«{key}» falta o es demasiado largo."))
}

/// A file's path as the core gave it in a message (the core itself keeps previews inside Buddy's folders).
fn path(args: &Value, key: &str) -> Result<String, String> {
    args[key]
        .as_str()
        .filter(|s| !s.is_empty() && s.len() <= 1024 && !s.contains('\0') && !s.split(['/', '\\']).any(|part| part == ".."))
        .map(str::to_string)
        .ok_or_else(|| format!("«{key}» no es una ruta válida."))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn listed_calls_parse_with_their_arguments() {
        assert_eq!(parse("hello", &Value::Null), Ok(Call::Hello));
        assert_eq!(parse("chats", &json!({})), Ok(Call::Chats { limit: 50 }));
        assert_eq!(parse("chats", &json!({ "limit": 9999 })), Ok(Call::Chats { limit: 200 }));
        assert_eq!(parse("messages", &json!({ "chatId": "c18f3a_b-2" })), Ok(Call::Messages { chat_id: "c18f3a_b-2".into() }));
        assert_eq!(
            parse("send_message", &json!({ "text": "  hola  " })),
            Ok(Call::SendMessage { chat_id: None, text: "hola".into() })
        );
        assert_eq!(
            parse("send_message", &json!({ "chatId": "telegram-parley", "text": "hola" })),
            Ok(Call::SendMessage { chat_id: Some("telegram-parley".into()), text: "hola".into() })
        );
        assert_eq!(
            parse("answer_approval", &json!({ "requestId": "812-3", "allow": false })),
            Ok(Call::AnswerApproval { request_id: "812-3".into(), allow: false })
        );
        assert_eq!(parse("focus_start", &json!({ "minutes": 25 })), Ok(Call::FocusStart { minutes: 25 }));
    }

    #[test]
    fn nothing_outside_the_list_is_reachable() {
        for name in [
            "set_setting", "setting", "delete_chat", "add_folder", "remove_folder", "hooks_write", "set_agent_permissions",
            "answer_approval_always", "telegram_connect", "spotify_connect", "set_connector_key", "screenshot_taken",
            "remote_forget", "", "Hello", "hello ", "../hello",
        ] {
            assert_eq!(parse(name, &json!({})), Err(REFUSED.into()), "{name:?}");
        }
    }

    #[test]
    fn an_identifier_can_never_leave_its_folder() {
        for bad in ["..", "../x", "a/b", "a\\b", "c1/../../etc", "", " c1", "c1\n", "c\u{0}1", "ñ", &"a".repeat(81)] {
            assert!(parse("messages", &json!({ "chatId": bad })).is_err(), "{bad:?}");
            assert!(parse("send_message", &json!({ "chatId": bad, "text": "x" })).is_err(), "{bad:?}");
        }
        assert!(parse("messages", &json!({ "chatId": 7 })).is_err());
        assert!(parse("messages", &json!({})).is_err());
        assert!(parse("image_preview", &json!({ "path": "/d/adjuntos/c1/../../secreto.png" })).is_err());
        assert!(parse("image_preview", &json!({ "path": "/d/adjuntos/c1/foto.png" })).is_ok());
    }

    #[test]
    fn texts_and_numbers_are_bounded() {
        assert!(parse("send_message", &json!({ "text": "   " })).is_err());
        assert!(parse("send_message", &json!({ "text": "x".repeat(20_001) })).is_err());
        assert!(parse("search_chats", &json!({ "query": "x".repeat(201) })).is_err());
        assert!(parse("focus_start", &json!({ "minutes": 0 })).is_err());
        assert!(parse("focus_start", &json!({ "minutes": 181 })).is_err());
        assert!(parse("answer_approval", &json!({ "requestId": "1-2" })).is_err(), "allow is required");
    }

    #[test]
    fn only_calls_that_act_leave_a_line_in_the_log() {
        assert_eq!(Call::Chats { limit: 1 }.acts(), None);
        assert_eq!(Call::SendMessage { chat_id: None, text: "x".into() }.acts().as_deref(), Some("iPhone envió un mensaje"));
        assert_eq!(Call::AnswerApproval { request_id: "1-1".into(), allow: true }.acts().as_deref(), Some("iPhone permitió un permiso"));
    }
}
