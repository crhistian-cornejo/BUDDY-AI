//! The phone's end of the link, as the iPhone app uses it (through UniFFI, feature `ffi`): scanning the pairing
//! code, answering it, and the encrypted channel as one object that is fed the relay's frames and gives back what
//! they were. The same code the desktop's end is tested against: the app only moves text between this and its socket.

use std::sync::{Arc, Mutex};

use serde_json::Value;

use crate::pairing::{self, Offer};
use crate::wire::{Envelope, Frame, unpack};
use crate::{Channel, Handshake, Keys};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[cfg_attr(feature = "ffi", derive(uniffi::Error))]
#[cfg_attr(feature = "ffi", uniffi(flat_error))]
pub enum PhoneError {
    /// In words for the user.
    #[error("{0}")]
    Said(String),
}

fn said(error: impl std::fmt::Display) -> PhoneError {
    PhoneError::Said(error.to_string())
}

/// The phone's long-term key pair. The private half goes to the Keychain and nowhere else.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct PhoneKeys {
    pub private_key: Vec<u8>,
    pub public_key: Vec<u8>,
}

/// What a scanned pairing code says.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct ScannedOffer {
    pub relay: String,
    pub room: String,
    pub room_key: String,
    pub desktop_public: Vec<u8>,
    /// Unix seconds after which the code is void.
    pub expires_at: i64,
}

/// What a frame from the relay turned out to be.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum Incoming {
    /// The desktop answered the greeting: calls may go now.
    Ready,
    /// A reply or an event from the desktop, as JSON (`{"k":"reply",…}` / `{"k":"event",…}`).
    Frame { json: String },
    /// Whether the desktop is connected to the relay.
    Peer { online: bool },
    /// The relay's own complaint (`too-big`, `rate`…).
    Relay { code: String },
    /// The channel can no longer be read (a frame was altered, repeated or lost): greet again.
    Broken,
    /// Nothing for the app (a piece of a longer message, or a frame that is not for a phone).
    Nothing,
}

/// A sealed notice, opened.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct OpenedNotice {
    pub counter: u64,
    /// `{"kind","title","body","target"}`.
    pub json: String,
}

fn key32(bytes: &[u8]) -> Result<[u8; 32], PhoneError> {
    bytes.try_into().map_err(|_| said("La clave no es válida."))
}

#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn new_keys() -> PhoneKeys {
    let keys = Keys::generate();
    PhoneKeys { private_key: keys.private.to_vec(), public_key: keys.public.to_vec() }
}

/// Reads a pairing code (the link behind the QR).
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn scan(uri: String) -> Result<ScannedOffer, PhoneError> {
    let offer = Offer::parse(&uri).map_err(said)?;
    Ok(ScannedOffer {
        desktop_public: offer.desktop_public().map_err(said)?.to_vec(),
        relay: offer.relay.clone(),
        room: offer.room.clone(),
        room_key: offer.key.clone(),
        expires_at: offer.exp,
    })
}

/// The frame to send to the relay after scanning: this phone's key and name, proved with the code's secret.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn pair_request(uri: String, public_key: Vec<u8>, name: String) -> Result<String, PhoneError> {
    let offer = Offer::parse(&uri).map_err(said)?;
    let hello = pairing::hello(&offer, &key32(&public_key)?, &name);
    Ok(Envelope::Pair { d: serde_json::to_string(&hello).map_err(said)? }.to_text())
}

/// Checks the desktop's answer to `pair_request`. When it is the desktop that showed the code, returns the key
/// its notices will be sealed with (to keep in the Keychain); any other frame is None.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn pair_confirm(uri: String, keys: PhoneKeys, frame: String) -> Result<Option<Vec<u8>>, PhoneError> {
    let Ok(Envelope::Paired { d }) = Envelope::parse(&frame) else { return Ok(None) };
    let offer = Offer::parse(&uri).map_err(said)?;
    let mine = Keys::from_private(key32(&keys.private_key)?);
    if !offer.check_ack(&mine.public, &d) {
        return Err(said("La respuesta no viene del equipo que mostró el código."));
    }
    let key = crate::push::key(&mine, &offer.desktop_public().map_err(said)?, &offer.secret().map_err(said)?, &offer.room);
    Ok(Some(key.to_vec()))
}

/// Opens a notice delivered through Apple (base64, as it arrives in the notification's `d`).
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn open_notice(push_key: Vec<u8>, sealed: String, last_counter: u64, now: i64) -> Result<OpenedNotice, PhoneError> {
    let (counter, payload) = crate::push::open(&key32(&push_key)?, &unpack(&sealed).map_err(said)?, last_counter, now).map_err(said)?;
    Ok(OpenedNotice { counter, json: payload.to_string() })
}

enum State {
    Greeting(Handshake),
    Open(Channel),
    Closed,
}

/// The phone's encrypted channel to one desktop, for one connection.
#[cfg_attr(feature = "ffi", derive(uniffi::Object))]
pub struct PhoneChannel {
    state: Mutex<State>,
    greeting: String,
}

#[cfg_attr(feature = "ffi", uniffi::export)]
impl PhoneChannel {
    /// Starts the greeting with the paired desktop. Send `greeting()` to the relay next.
    #[cfg_attr(feature = "ffi", uniffi::constructor)]
    pub fn new(private_key: Vec<u8>, desktop_public: Vec<u8>, room: String) -> Result<Arc<Self>, PhoneError> {
        let keys = Keys::from_private(key32(&private_key)?);
        let (handshake, hs1) = Channel::initiator(&keys, &key32(&desktop_public)?, &room).map_err(said)?;
        Ok(Arc::new(Self { state: Mutex::new(State::Greeting(handshake)), greeting: Envelope::hs1(&hs1).to_text() }))
    }

    pub fn greeting(&self) -> String {
        self.greeting.clone()
    }

    /// A text frame from the relay.
    pub fn receive(&self, frame: String) -> Incoming {
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        match Envelope::parse(&frame) {
            Ok(Envelope::Peer { on }) => Incoming::Peer { online: on },
            Ok(Envelope::Error { code }) => Incoming::Relay { code },
            Ok(Envelope::Hs2 { d }) => match std::mem::replace(&mut *state, State::Closed) {
                State::Greeting(handshake) => match unpack(&d).and_then(|hs2| handshake.finish(&hs2)) {
                    Ok(channel) => {
                        *state = State::Open(channel);
                        Incoming::Ready
                    }
                    Err(_) => Incoming::Broken,
                },
                other => {
                    *state = other;
                    Incoming::Nothing
                }
            },
            Ok(Envelope::Msg { d }) => {
                let State::Open(channel) = &mut *state else { return Incoming::Nothing };
                match unpack(&d).and_then(|piece| channel.open(&piece)) {
                    Ok(None) => Incoming::Nothing,
                    Ok(Some(bytes)) => match Frame::parse(&bytes) {
                        Ok(Frame::Reply { .. } | Frame::Event { .. }) => Incoming::Frame { json: String::from_utf8_lossy(&bytes).into_owned() },
                        _ => Incoming::Nothing,
                    },
                    Err(_) => {
                        *state = State::Closed;
                        Incoming::Broken
                    }
                }
            }
            _ => Incoming::Nothing,
        }
    }

    /// A call to the desktop, as the frames to send in this order. `args_json` is a JSON object (or empty).
    pub fn call(&self, id: u32, name: String, args_json: String) -> Result<Vec<String>, PhoneError> {
        let args: Value = if args_json.trim().is_empty() { Value::Null } else { serde_json::from_str(&args_json).map_err(said)? };
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        let State::Open(channel) = &mut *state else { return Err(said("El canal con el equipo no está abierto.")) };
        let pieces = channel.seal(&Frame::Call { id, call: name, args }.to_bytes()).map_err(said)?;
        Ok(pieces.iter().map(|piece| Envelope::msg(piece).to_text()).collect())
    }

    pub fn is_open(&self) -> bool {
        matches!(*self.state.lock().unwrap_or_else(|p| p.into_inner()), State::Open(_))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const ROOM: &str = "00112233445566778899aabbccddeeff";

    /// The desktop's side, as `core` does it with the same crate.
    fn desktop_accepts(desktop: &Keys, phone_public: &[u8], greeting: &str) -> (Channel, String) {
        let Ok(Envelope::Hs1 { d }) = Envelope::parse(greeting) else { panic!("a greeting") };
        let (channel, hs2) = Channel::responder(desktop, &phone_public.try_into().unwrap(), ROOM, &unpack(&d).unwrap()).unwrap();
        (channel, Envelope::hs2(&hs2).to_text())
    }

    #[test]
    fn a_scanned_code_pairs_and_both_ends_hold_the_same_notices_key() {
        let desktop = Keys::generate();
        let offer = Offer::new("https://relay.example", ROOM, &"ab".repeat(32), &desktop.public, 1_000);
        let scanned = scan(offer.uri()).unwrap();
        assert_eq!((scanned.relay.as_str(), scanned.room.as_str(), scanned.expires_at), ("https://relay.example", ROOM, 1_300));
        assert_eq!(scanned.desktop_public, desktop.public);

        let keys = new_keys();
        let request = pair_request(offer.uri(), keys.public_key.clone(), "iPhone de prueba".into()).unwrap();
        let Ok(Envelope::Pair { d }) = Envelope::parse(&request) else { panic!("a pair frame") };
        let peer = offer.verify(&serde_json::from_str(&d).unwrap(), 1_100).unwrap();
        let answer = Envelope::Paired { d: offer.ack(&peer) }.to_text();
        let push_key = pair_confirm(offer.uri(), keys.clone(), answer).unwrap().unwrap();
        assert_eq!(push_key, crate::push::key(&desktop, &peer.public, &offer.secret().unwrap(), ROOM));
        // Another frame is not the answer; a forged answer is refused.
        assert_eq!(pair_confirm(offer.uri(), keys.clone(), Envelope::Peer { on: true }.to_text()).unwrap(), None);
        assert!(pair_confirm(offer.uri(), keys, Envelope::Paired { d: "AAAA".into() }.to_text()).is_err());
        assert!(scan("https://example.com".into()).is_err());

        // A notice sealed by the desktop opens on the phone.
        let sealed = crate::wire::pack(&crate::push::seal(&push_key.clone().try_into().unwrap(), 7, 2_000, &json!({ "title": "Buddy respondió" })));
        let opened = open_notice(push_key.clone(), sealed.clone(), 0, 2_010).unwrap();
        assert_eq!((opened.counter, serde_json::from_str::<Value>(&opened.json).unwrap()["title"].as_str().map(String::from)), (7, Some("Buddy respondió".into())));
        assert!(open_notice(push_key, sealed, 7, 2_010).is_err(), "already seen");
    }

    #[test]
    fn the_channel_greets_calls_and_reads_replies_and_events() {
        let desktop = Keys::generate();
        let keys = new_keys();
        let phone = PhoneChannel::new(keys.private_key.clone(), desktop.public.to_vec(), ROOM.into()).unwrap();
        assert!(!phone.is_open());
        assert!(phone.call(1, "hello".into(), String::new()).is_err(), "not before the desktop answers");
        let (mut desk, hs2) = desktop_accepts(&desktop, &keys.public_key, &phone.greeting());
        assert_eq!(phone.receive(Envelope::Peer { on: true }.to_text()), Incoming::Peer { online: true });
        assert_eq!(phone.receive(hs2), Incoming::Ready);
        assert!(phone.is_open());

        // A call arrives at the desktop as a frame.
        let frames = phone.call(3, "chats".into(), r#"{"limit":5}"#.into()).unwrap();
        let Ok(Envelope::Msg { d }) = Envelope::parse(&frames[0]) else { panic!("a message") };
        let call = Frame::parse(&desk.open(&unpack(&d).unwrap()).unwrap().unwrap()).unwrap();
        assert_eq!(call, Frame::Call { id: 3, call: "chats".into(), args: json!({ "limit": 5 }) });

        // Its reply and an event come back as JSON.
        for (frame, expected) in [
            (Frame::Reply { id: 3, ok: Some(json!([{ "id": "c1" }])), err: None }, json!({ "k": "reply", "id": 3, "ok": [{ "id": "c1" }] })),
            (Frame::Event { event: json!({ "type": "chatDelta", "chatId": "c1", "text": "ho" }) }, json!({ "k": "event", "event": { "type": "chatDelta", "chatId": "c1", "text": "ho" } })),
        ] {
            let sealed = desk.seal(&frame.to_bytes()).unwrap();
            let Incoming::Frame { json } = phone.receive(Envelope::msg(&sealed[0]).to_text()) else { panic!("a frame") };
            assert_eq!(serde_json::from_str::<Value>(&json).unwrap(), expected);
        }
        assert_eq!(phone.receive(Envelope::Error { code: "rate".into() }.to_text()), Incoming::Relay { code: "rate".into() });
        assert_eq!(phone.receive("basura".into()), Incoming::Nothing);

        // A repeated frame breaks the channel: nothing more is read on it.
        let sealed = desk.seal(&Frame::Reply { id: 4, ok: None, err: Some("x".into()) }.to_bytes()).unwrap();
        let text = Envelope::msg(&sealed[0]).to_text();
        assert!(matches!(phone.receive(text.clone()), Incoming::Frame { .. }));
        assert_eq!(phone.receive(text), Incoming::Broken);
        assert!(!phone.is_open());
    }

    #[test]
    fn a_greeting_answered_by_someone_else_does_not_open() {
        let (desktop, stranger) = (Keys::generate(), Keys::generate());
        let keys = new_keys();
        let phone = PhoneChannel::new(keys.private_key.clone(), desktop.public.to_vec(), ROOM.into()).unwrap();
        // The stranger cannot even read the greeting; it answers one of its own.
        let other = PhoneChannel::new(keys.private_key.clone(), stranger.public.to_vec(), ROOM.into()).unwrap();
        let (_, forged) = desktop_accepts(&stranger, &keys.public_key, &other.greeting());
        assert_eq!(phone.receive(forged), Incoming::Broken);
        assert!(!phone.is_open());
        assert!(PhoneChannel::new(vec![1, 2, 3], desktop.public.to_vec(), ROOM.into()).is_err());
    }
}
