//! The shapes on the wire.
//!
//! `Envelope`: what the relay sees, one JSON text frame with a `t` field. The relay forwards `pair`, `paired`,
//! `hs1`, `hs2` and `msg` to the other end untouched; the rest is between one end and the relay itself.
//! `Frame`: what travels inside `msg`, encrypted (`Channel`): the phone's calls, their replies, the core's events.

use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::Error;

/// The longest text frame the relay accepts.
pub const MAX_FRAME: usize = 256 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "lowercase")]
pub enum Envelope {
    /// The phone's pairing answer (`pairing::Hello` as JSON, in `d`).
    Pair { d: String },
    /// The desktop's proof back (`Offer::ack`).
    Paired { d: String },
    /// Handshake messages (base64).
    Hs1 { d: String },
    Hs2 { d: String },
    /// One encrypted piece (base64).
    Msg { d: String },
    /// From the relay: whether the other end is connected.
    Peer { on: bool },
    /// Phone → relay: where Apple delivers its notices.
    Token { alert: String, live: Option<String>, sandbox: bool },
    /// Desktop → relay: a sealed notice to hand to Apple.
    Push {
        kind: String,
        d: String,
        urgent: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        collapse: Option<String>,
    },
    /// From the relay: `too-big`, `rate`, `no-apns`, `bad-frame`.
    Error { code: String },
}

impl Envelope {
    pub fn parse(text: &str) -> Result<Self, Error> {
        if text.len() > MAX_FRAME {
            return Err(Error::TooBig);
        }
        serde_json::from_str(text).map_err(|_| Error::Malformed("sobre"))
    }

    pub fn to_text(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }

    pub fn hs1(bytes: &[u8]) -> Self {
        Self::Hs1 { d: pack(bytes) }
    }

    pub fn hs2(bytes: &[u8]) -> Self {
        Self::Hs2 { d: pack(bytes) }
    }

    pub fn msg(piece: &[u8]) -> Self {
        Self::Msg { d: pack(piece) }
    }
}

/// Bytes as an envelope's `d` (standard base64).
pub fn pack(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

pub fn unpack(d: &str) -> Result<Vec<u8>, Error> {
    base64::engine::general_purpose::STANDARD.decode(d).map_err(|_| Error::Malformed("base64"))
}

/// What the two ends say to each other, inside the channel.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "k", rename_all = "lowercase")]
pub enum Frame {
    /// The phone asks the core for something (`id` pairs it with its reply).
    Call {
        id: u32,
        call: String,
        #[serde(default)]
        args: Value,
    },
    /// The core's answer: `ok` with the result, or `err` with the reason, in words for the user.
    Reply {
        id: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ok: Option<Value>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        err: Option<String>,
    },
    /// One of the core's events, as the apps get it.
    Event { event: Value },
}

impl Frame {
    pub fn parse(bytes: &[u8]) -> Result<Self, Error> {
        serde_json::from_slice(bytes).map_err(|_| Error::Malformed("trama"))
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        serde_json::to_vec(self).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn envelopes_have_the_shapes_the_relay_expects() {
        let shape = |e: Envelope| serde_json::from_str::<Value>(&e.to_text()).unwrap();
        assert_eq!(shape(Envelope::msg(b"hola")), json!({ "t": "msg", "d": "aG9sYQ==" }));
        assert_eq!(shape(Envelope::hs1(&[1, 2])), json!({ "t": "hs1", "d": "AQI=" }));
        assert_eq!(shape(Envelope::hs2(&[1, 2])), json!({ "t": "hs2", "d": "AQI=" }));
        assert_eq!(shape(Envelope::Pair { d: "x".into() }), json!({ "t": "pair", "d": "x" }));
        assert_eq!(shape(Envelope::Paired { d: "x".into() }), json!({ "t": "paired", "d": "x" }));
        assert_eq!(shape(Envelope::Peer { on: true }), json!({ "t": "peer", "on": true }));
        assert_eq!(
            shape(Envelope::Token { alert: "ab".into(), live: None, sandbox: true }),
            json!({ "t": "token", "alert": "ab", "live": null, "sandbox": true })
        );
        assert_eq!(
            shape(Envelope::Push { kind: "alert".into(), d: "x".into(), urgent: true, collapse: None }),
            json!({ "t": "push", "kind": "alert", "d": "x", "urgent": true })
        );
        assert_eq!(shape(Envelope::Error { code: "rate".into() }), json!({ "t": "error", "code": "rate" }));
    }

    #[test]
    fn envelopes_round_trip_and_junk_is_refused() {
        for e in [Envelope::msg(b"x"), Envelope::Peer { on: false }, Envelope::Push { kind: "live".into(), d: "{}".into(), urgent: false, collapse: Some("c1".into()) }] {
            assert_eq!(Envelope::parse(&e.to_text()).unwrap(), e);
        }
        assert_eq!(unpack(&pack(&[0, 255, 7])).unwrap(), [0, 255, 7]);
        assert_eq!(Envelope::parse("hola").err(), Some(Error::Malformed("sobre")));
        assert_eq!(Envelope::parse(r#"{"t":"otro","d":"x"}"#).err(), Some(Error::Malformed("sobre")));
        assert_eq!(Envelope::parse(r#"{"t":"msg"}"#).err(), Some(Error::Malformed("sobre")));
        let huge = Envelope::Msg { d: "A".repeat(MAX_FRAME) }.to_text();
        assert_eq!(Envelope::parse(&huge).err(), Some(Error::TooBig));
        assert!(unpack("no es base64 !").is_err());
    }

    #[test]
    fn frames_have_a_kind_and_round_trip() {
        let call = Frame::Call { id: 7, call: "chats".into(), args: json!({ "limit": 20 }) };
        assert_eq!(serde_json::from_slice::<Value>(&call.to_bytes()).unwrap(), json!({ "k": "call", "id": 7, "call": "chats", "args": { "limit": 20 } }));
        assert_eq!(Frame::parse(&call.to_bytes()).unwrap(), call);
        // A call without arguments.
        assert_eq!(Frame::parse(br#"{"k":"call","id":1,"call":"hello"}"#).unwrap(), Frame::Call { id: 1, call: "hello".into(), args: Value::Null });
        let ok = Frame::Reply { id: 7, ok: Some(json!([1])), err: None };
        assert_eq!(serde_json::from_slice::<Value>(&ok.to_bytes()).unwrap(), json!({ "k": "reply", "id": 7, "ok": [1] }));
        let err = Frame::Reply { id: 7, ok: None, err: Some("llamada no permitida".into()) };
        assert_eq!(Frame::parse(&err.to_bytes()).unwrap(), err);
        let event = Frame::Event { event: json!({ "type": "chatDelta", "chatId": "c1", "text": "ho" }) };
        assert_eq!(Frame::parse(&event.to_bytes()).unwrap(), event);
        assert!(Frame::parse(b"{}").is_err());
    }
}
