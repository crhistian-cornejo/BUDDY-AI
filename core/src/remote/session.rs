//! The desktop's end of the phone link, without the network: what to answer to each frame from the relay
//! (`on_frame`) and what to send for each of the core's events (`on_event`). `link` carries the frames; the tests
//! drive this directly with a simulated phone.
//!
//! With the phone connected (a live `Channel`) the listed events are forwarded, `ChatDelta` gathered so that at
//! most five frames a second leave. With the phone away nothing is forwarded: the events that are news become
//! sealed notices for the relay to hand to Apple (`push`).

use std::collections::HashMap;
use std::sync::Arc;

use buddy_remote::pairing::{Hello, Offer, PairError, Peer};
use buddy_remote::wire::{Envelope, Frame, pack, unpack};
use buddy_remote::{Channel, Keys};
use serde_json::{Value, json};

use super::push::Pusher;
use super::rpc::{self, Call, Host};
use crate::events::Event;

/// Wrong answers to a pairing code before it is void.
pub const MAX_WRONG_PAIRS: u32 = 5;
/// The shortest time between two frames of answer text.
const DELTA_GAP_MS: u64 = 200;
/// The approval card's title for a screen capture (`sessions::format::title_for`).
const SCREEN_TITLE: &str = "Ver tu pantalla";

/// The time, both ways the session needs it: the calendar's (unix seconds) and a steady one (milliseconds).
#[derive(Debug, Clone, Copy)]
pub struct Clock {
    pub unix: i64,
    pub ms: u64,
}

#[derive(Debug, PartialEq)]
pub enum Out {
    /// A text frame for the relay.
    Send(String),
    /// A line for the user's log of what the phone did.
    Log(String),
    /// A phone scanned the code: its key and the notices' key are to be kept.
    Paired { peer: Peer, push_key: [u8; 32] },
    /// The pairing code is no longer good (used, expired or guessed at too often).
    PairingEnded,
    /// The phone's encrypted channel opened or closed.
    Online(bool),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Options {
    /// «Aprobar permisos desde el iPhone»: off, the phone can only deny.
    pub approvals: bool,
}

pub struct Session {
    host: Arc<dyn Host>,
    keys: Keys,
    room: String,
    pub opts: Options,
    /// The paired phone's public key and the notices' key.
    peer: Option<([u8; 32], [u8; 32])>,
    /// The code on screen, and how many wrong answers it got.
    offer: Option<(Offer, u32)>,
    channel: Option<Channel>,
    pusher: Pusher,
    push_counter: u64,
    /// Pending approval requests: whether each is a screen capture.
    approvals: HashMap<String, bool>,
    /// Answer text waiting to leave, by chat, in order.
    deltas: Vec<(String, String)>,
    last_delta_ms: Option<u64>,
}

impl Session {
    pub fn new(host: Arc<dyn Host>, keys: Keys, room: &str, opts: Options) -> Self {
        Self {
            host,
            keys,
            room: room.to_string(),
            opts,
            peer: None,
            offer: None,
            channel: None,
            pusher: Pusher::default(),
            push_counter: 0,
            approvals: HashMap::new(),
            deltas: Vec::new(),
            last_delta_ms: None,
        }
    }

    /// The phone paired earlier (its key and the notices' key come from the keychain).
    pub fn set_peer(&mut self, public: [u8; 32], push_key: [u8; 32]) {
        self.peer = Some((public, push_key));
    }

    /// Shows a pairing code: the next phone that answers it right becomes the paired one.
    pub fn start_pairing(&mut self, offer: Offer) {
        self.offer = Some((offer, 0));
    }

    pub fn pairing(&self) -> bool {
        self.offer.is_some()
    }

    /// Whether the phone's encrypted channel is open.
    pub fn online(&self) -> bool {
        self.channel.is_some()
    }

    /// A text frame from the relay.
    pub fn on_frame(&mut self, text: &str, now: Clock) -> Vec<Out> {
        match Envelope::parse(text) {
            Ok(Envelope::Peer { on: false }) => self.close(),
            Ok(Envelope::Pair { d }) => self.pair(&d, now),
            Ok(Envelope::Hs1 { d }) => self.handshake(&d),
            Ok(Envelope::Msg { d }) => self.message(&d, now),
            // Nothing else is for the desktop (the relay's own notes, or frames only a phone receives).
            _ => Vec::new(),
        }
    }

    /// One of the core's events.
    pub fn on_event(&mut self, event: &Event, now: Clock) -> Vec<Out> {
        match event {
            Event::ApprovalRequest { request_id, title, .. } => {
                self.approvals.insert(request_id.clone(), title == SCREEN_TITLE);
            }
            Event::ApprovalClosed { request_id } => {
                self.approvals.remove(request_id);
            }
            _ => {}
        }
        let notice = self.pusher.observe(event, now.ms);
        if self.channel.is_some() {
            if !forwards(event) {
                return Vec::new();
            }
            if let Event::ChatDelta { chat_id, text } = event {
                match self.deltas.last_mut() {
                    Some((chat, waiting)) if chat == chat_id => waiting.push_str(text),
                    _ => self.deltas.push((chat_id.clone(), text.clone())),
                }
                return self.flush(now);
            }
            // Anything else leaves at once, after the text that came before it.
            let mut outs = self.send_deltas(now);
            outs.extend(self.send(&Frame::Event { event: serde_json::to_value(event).unwrap_or(Value::Null) }));
            return outs;
        }
        // The phone is away: only news, as a sealed notice.
        let (Some(notice), Some((_, key))) = (notice, self.peer) else { return Vec::new() };
        if !self.pusher.allow(now.ms) {
            return Vec::new();
        }
        self.push_counter = (self.push_counter + 1).max(now.unix.max(0) as u64 * 1000);
        let payload = json!({ "kind": notice.kind, "title": notice.title, "body": notice.body, "target": notice.target });
        let sealed = buddy_remote::push::seal(&key, self.push_counter, now.unix, &payload);
        vec![Out::Send(Envelope::Push { kind: "alert".into(), d: pack(&sealed), urgent: notice.urgent, collapse: None }.to_text())]
    }

    /// Sends the answer text that has waited long enough. Call it when `next_flush_ms` comes.
    pub fn flush(&mut self, now: Clock) -> Vec<Out> {
        match self.last_delta_ms {
            Some(last) if now.ms.saturating_sub(last) < DELTA_GAP_MS => Vec::new(),
            _ => self.send_deltas(now),
        }
    }

    /// When waiting answer text may leave (steady milliseconds), if any waits.
    pub fn next_flush_ms(&self) -> Option<u64> {
        (!self.deltas.is_empty()).then(|| self.last_delta_ms.map_or(0, |last| last + DELTA_GAP_MS))
    }

    fn send_deltas(&mut self, now: Clock) -> Vec<Out> {
        let mut outs = Vec::new();
        for (chat_id, text) in std::mem::take(&mut self.deltas) {
            self.last_delta_ms = Some(now.ms);
            outs.extend(self.send(&Frame::Event { event: json!({ "type": "chatDelta", "chatId": chat_id, "text": text }) }));
        }
        outs
    }

    fn close(&mut self) -> Vec<Out> {
        self.deltas.clear();
        match self.channel.take() {
            Some(_) => vec![Out::Online(false)],
            None => Vec::new(),
        }
    }

    fn pair(&mut self, d: &str, now: Clock) -> Vec<Out> {
        let Some((offer, wrong)) = &mut self.offer else { return Vec::new() };
        let verified = serde_json::from_str::<Hello>(d).map_err(|_| PairError::BadProof).and_then(|hello| offer.verify(&hello, now.unix));
        match verified {
            Ok(peer) => {
                let secret = offer.secret().unwrap_or_default();
                let push_key = buddy_remote::push::key(&self.keys, &peer.public, &secret, &self.room);
                let ack = Envelope::Paired { d: offer.ack(&peer) }.to_text();
                self.offer = None;
                self.channel = None;
                self.peer = Some((peer.public, push_key));
                vec![Out::Paired { peer, push_key }, Out::Send(ack), Out::PairingEnded]
            }
            Err(PairError::Expired) => {
                self.offer = None;
                vec![Out::PairingEnded]
            }
            Err(_) => {
                *wrong += 1;
                if *wrong < MAX_WRONG_PAIRS {
                    return Vec::new();
                }
                self.offer = None;
                vec![Out::Log("Emparejado anulado: demasiados intentos fallidos".into()), Out::PairingEnded]
            }
        }
    }

    fn handshake(&mut self, d: &str) -> Vec<Out> {
        let Some((public, _)) = self.peer else { return Vec::new() };
        let Ok(hs1) = unpack(d) else { return Vec::new() };
        // Only the paired phone's key completes this; anyone else's first message fails here.
        let Ok((channel, hs2)) = Channel::responder(&self.keys, &public, &self.room, &hs1) else { return Vec::new() };
        let was = self.channel.replace(channel).is_some();
        self.deltas.clear();
        let mut outs = vec![Out::Send(Envelope::hs2(&hs2).to_text())];
        if !was {
            outs.push(Out::Online(true));
        }
        outs
    }

    fn message(&mut self, d: &str, now: Clock) -> Vec<Out> {
        let Some(channel) = &mut self.channel else { return Vec::new() };
        let opened = unpack(d).and_then(|piece| channel.open(&piece));
        match opened {
            Ok(None) => Vec::new(),
            Ok(Some(bytes)) => match Frame::parse(&bytes) {
                Ok(Frame::Call { id, call, args }) => self.call(id, &call, &args, now),
                // The phone only calls.
                _ => Vec::new(),
            },
            // Altered, repeated or out of order: this channel says nothing more; the phone must greet again.
            Err(_) => {
                let mut outs = self.close();
                outs.push(Out::Log("Canal con el iPhone cerrado: llegó una trama no válida".into()));
                outs
            }
        }
    }

    fn call(&mut self, id: u32, name: &str, args: &Value, _now: Clock) -> Vec<Out> {
        let mut outs = Vec::new();
        let result = match rpc::parse(name, args) {
            Err(reason) => {
                if reason == rpc::REFUSED {
                    let shown: String = name.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '_').take(40).collect();
                    outs.push(Out::Log(format!("iPhone pidió algo no permitido: «{shown}»")));
                }
                Err(reason)
            }
            Ok(call) => match self.guard(&call) {
                Err(reason) => {
                    outs.push(Out::Log(format!("iPhone no pudo responder un permiso: {reason}")));
                    Err(reason)
                }
                Ok(()) => {
                    let line = call.acts();
                    let sends = matches!(call, Call::SendMessage { .. });
                    let result = self.host.run(call);
                    if let Ok(value) = &result {
                        if let (true, Some(chat)) = (sends, value.as_str()) {
                            self.pusher.asked_from_phone(chat);
                        }
                        outs.extend(line.map(Out::Log));
                    }
                    result
                }
            },
        };
        let reply = match result {
            Ok(value) => Frame::Reply { id, ok: Some(value), err: None },
            Err(reason) => Frame::Reply { id, ok: None, err: Some(reason) },
        };
        outs.extend(self.send(&reply));
        outs
    }

    /// What only this session knows about a call: an approval must be one that is pending, the phone allows only
    /// with the switch on, and never a screen capture (nobody is at the screen to see what it would show).
    fn guard(&self, call: &Call) -> Result<(), String> {
        let Call::AnswerApproval { request_id, allow } = call else { return Ok(()) };
        let Some(screen) = self.approvals.get(request_id) else { return Err("Ese permiso ya no está pendiente.".into()) };
        if *allow && !self.opts.approvals {
            return Err("Aprobar desde el iPhone está apagado en Ajustes › iPhone de este equipo.".into());
        }
        if *allow && *screen {
            return Err("Ver la pantalla solo se permite desde el propio equipo.".into());
        }
        Ok(())
    }

    fn send(&mut self, frame: &Frame) -> Vec<Out> {
        let Some(channel) = &mut self.channel else { return Vec::new() };
        match channel.seal(&frame.to_bytes()) {
            Ok(pieces) => pieces.iter().map(|piece| Out::Send(Envelope::msg(piece).to_text())).collect(),
            Err(_) => Vec::new(),
        }
    }
}

/// The events the phone draws. Everything else stays on the desktop: settings, the music player's commands,
/// screen captures, and whatever a later phase adds until it is listed here.
fn forwards(event: &Event) -> bool {
    matches!(
        event,
        Event::ChatStarted { .. }
            | Event::ChatDelta { .. }
            | Event::ChatTool { .. }
            | Event::ChatSource { .. }
            | Event::ChatActivity { .. }
            | Event::ChatDone { .. }
            | Event::ChatFailed { .. }
            | Event::ChatQueueChanged { .. }
            | Event::ChatDequeued { .. }
            | Event::MascotState { .. }
            | Event::SessionUpdate { .. }
            | Event::ApprovalRequest { .. }
            | Event::ApprovalClosed { .. }
            | Event::UsageChanged
            | Event::UsageLow { .. }
            | Event::BriefingReady { .. }
            | Event::FinanceRecorded { .. }
            | Event::BudgetAlert { .. }
            | Event::FocusChanged { .. }
            | Event::FocusFinished { .. }
    )
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use buddy_remote::Handshake;
    use std::sync::Mutex;

    pub(crate) const ROOM: &str = "00112233445566778899aabbccddeeff";

    pub(crate) fn at(ms: u64) -> Clock {
        Clock { unix: 1_000 + (ms / 1000) as i64, ms }
    }

    /// The core, as far as these tests go: it records the calls and answers something.
    #[derive(Default)]
    pub(crate) struct FakeHost {
        pub calls: Mutex<Vec<Call>>,
    }

    impl Host for FakeHost {
        fn run(&self, call: Call) -> Result<Value, String> {
            self.calls.lock().unwrap().push(call.clone());
            match call {
                Call::Hello => Ok(json!({ "name": "Mac de prueba", "platform": "macos" })),
                Call::Chats { .. } => Ok(json!([{ "id": "c1", "title": "Hola" }])),
                Call::SendMessage { chat_id, .. } => Ok(json!(chat_id.unwrap_or_else(|| "c1".into()))),
                Call::Regenerate { .. } => Err("No hay nada que repetir.".into()),
                _ => Ok(Value::Null),
            }
        }
    }

    /// A phone: its keys and its end of the channel.
    pub(crate) struct Phone {
        pub keys: Keys,
        pub channel: Option<Channel>,
        next_id: u32,
    }

    impl Phone {
        pub fn new() -> Self {
            Self { keys: Keys::generate(), channel: None, next_id: 0 }
        }

        pub fn pair_frame(&self, offer: &Offer) -> String {
            let hello = buddy_remote::pairing::hello(offer, &self.keys.public, "iPhone de prueba");
            Envelope::Pair { d: serde_json::to_string(&hello).unwrap() }.to_text()
        }

        /// Greets the desktop; true when its answer completed the channel.
        pub fn connect(&mut self, session: &mut Session, desktop_public: &[u8; 32]) -> bool {
            let (hand, hs1): (Handshake, Vec<u8>) = Channel::initiator(&self.keys, desktop_public, ROOM).unwrap();
            let outs = session.on_frame(&Envelope::hs1(&hs1).to_text(), at(0));
            let answer = outs.iter().find_map(|o| match o {
                Out::Send(text) => match Envelope::parse(text) {
                    Ok(Envelope::Hs2 { d }) => Some(unpack(&d).unwrap()),
                    _ => None,
                },
                _ => None,
            });
            match answer.and_then(|hs2| hand.finish(&hs2).ok()) {
                Some(channel) => {
                    self.channel = Some(channel);
                    true
                }
                None => false,
            }
        }

        pub fn call_frames(&mut self, name: &str, args: Value) -> Vec<String> {
            self.next_id += 1;
            let frame = Frame::Call { id: self.next_id, call: name.into(), args };
            self.channel.as_mut().unwrap().seal(&frame.to_bytes()).unwrap().iter().map(|p| Envelope::msg(p).to_text()).collect()
        }

        /// Sends a call and returns the desktop's reply with whatever else it put out.
        pub fn call(&mut self, session: &mut Session, name: &str, args: Value) -> (Result<Value, String>, Vec<Out>) {
            let mut outs = Vec::new();
            for text in self.call_frames(name, args) {
                outs.extend(session.on_frame(&text, at(0)));
            }
            let reply = self.read(&outs).into_iter().find_map(|f| match f {
                Frame::Reply { ok, err, .. } => Some(err.map_or(Ok(ok.unwrap_or(Value::Null)), Err)),
                _ => None,
            });
            (reply.expect("a reply"), outs)
        }

        /// The frames the desktop sent, decrypted.
        pub fn read(&mut self, outs: &[Out]) -> Vec<Frame> {
            let mut frames = Vec::new();
            for out in outs {
                let Out::Send(text) = out else { continue };
                let Ok(Envelope::Msg { d }) = Envelope::parse(text) else { continue };
                if let Some(bytes) = self.channel.as_mut().unwrap().open(&unpack(&d).unwrap()).unwrap() {
                    frames.push(Frame::parse(&bytes).unwrap());
                }
            }
            frames
        }
    }

    pub(crate) fn session(approvals: bool) -> (Session, Arc<FakeHost>, Keys) {
        let host = Arc::new(FakeHost::default());
        let keys = Keys::generate();
        (Session::new(host.clone(), keys.clone(), ROOM, Options { approvals }), host, keys)
    }

    /// A session with a paired, connected phone.
    fn connected(approvals: bool) -> (Session, Arc<FakeHost>, Phone) {
        let (mut session, host, keys) = session(approvals);
        let mut phone = Phone::new();
        session.set_peer(phone.keys.public, [1u8; 32]);
        assert!(phone.connect(&mut session, &keys.public));
        (session, host, phone)
    }

    fn logs(outs: &[Out]) -> Vec<&str> {
        outs.iter().filter_map(|o| if let Out::Log(l) = o { Some(l.as_str()) } else { None }).collect()
    }

    fn approval(id: &str, title: &str) -> Event {
        Event::ApprovalRequest {
            request_id: id.into(), session_id: "s1".into(), agent: "claude".into(), project: "buddy".into(), title: title.into(),
            summary: "git push".into(), detail: "git push".into(), can_allow: true, always: String::new(),
        }
    }

    #[test]
    fn a_phone_that_scanned_the_code_pairs_and_then_talks() {
        let (mut session, host, keys) = session(true);
        let offer = Offer::new("https://relay.example", ROOM, "k", &keys.public, 1_000);
        session.start_pairing(offer.clone());
        let mut phone = Phone::new();
        let outs = session.on_frame(&phone.pair_frame(&offer), at(0));
        let Out::Paired { peer, push_key } = &outs[0] else { panic!("paired first: {outs:?}") };
        assert_eq!((peer.public, peer.name.as_str()), (phone.keys.public, "iPhone de prueba"));
        assert_eq!(*push_key, buddy_remote::push::key(&phone.keys, &keys.public, &offer.secret().unwrap(), ROOM), "the phone derives the same notices' key");
        let Out::Send(ack) = &outs[1] else { panic!("the desktop's proof") };
        let Ok(Envelope::Paired { d }) = Envelope::parse(ack) else { panic!("a paired envelope") };
        assert!(offer.check_ack(&phone.keys.public, &d));
        assert_eq!(outs[2], Out::PairingEnded);
        assert!(!session.pairing(), "the code is used up");
        // A second phone with the same code is ignored.
        assert!(session.on_frame(&Phone::new().pair_frame(&offer), at(0)).is_empty());

        assert!(phone.connect(&mut session, &keys.public));
        assert!(session.online());
        let (reply, outs) = phone.call(&mut session, "hello", Value::Null);
        assert_eq!(reply.unwrap()["name"], "Mac de prueba");
        assert!(logs(&outs).is_empty(), "reading leaves no line");
        assert_eq!(phone.call(&mut session, "chats", json!({ "limit": 5 })).0.unwrap()[0]["id"], "c1");
        assert_eq!(*host.calls.lock().unwrap(), [Call::Hello, Call::Chats { limit: 5 }]);
    }

    #[test]
    fn five_wrong_answers_void_the_code_and_an_expired_one_ends_at_once() {
        let (mut session, _host, keys) = session(true);
        let offer = Offer::new("https://relay.example", ROOM, "k", &keys.public, 1_000);
        let other = Offer::new("https://relay.example", ROOM, "k", &keys.public, 1_000); // another secret
        session.start_pairing(offer.clone());
        let phone = Phone::new();
        for _ in 0..MAX_WRONG_PAIRS - 1 {
            assert!(session.on_frame(&phone.pair_frame(&other), at(0)).is_empty());
        }
        // Junk counts as a wrong answer too.
        let outs = session.on_frame(&Envelope::Pair { d: "no es json".into() }.to_text(), at(0));
        assert_eq!(outs.last(), Some(&Out::PairingEnded));
        assert!(session.on_frame(&phone.pair_frame(&offer), at(0)).is_empty(), "even the right answer comes too late");

        session.start_pairing(offer.clone());
        let late = Clock { unix: offer.exp + 1, ms: 0 };
        assert_eq!(session.on_frame(&phone.pair_frame(&offer), late), [Out::PairingEnded]);
    }

    #[test]
    fn only_the_paired_phone_opens_the_channel() {
        let (mut session, _host, keys) = session(true);
        let mut stranger = Phone::new();
        assert!(!stranger.connect(&mut session, &keys.public), "nobody is paired");
        session.set_peer(Phone::new().keys.public, [1u8; 32]);
        assert!(!stranger.connect(&mut session, &keys.public), "another phone is paired");
        assert!(!session.online());
        // Frames without a channel are dropped.
        assert!(session.on_frame(&Envelope::msg(b"hola").to_text(), at(0)).is_empty());
    }

    #[test]
    fn a_call_outside_the_list_is_refused_and_logged() {
        let (mut session, host, mut phone) = connected(true);
        let (reply, outs) = phone.call(&mut session, "set_setting", json!({ "key": "commands.enabled", "value": "true" }));
        assert_eq!(reply, Err(rpc::REFUSED.into()));
        assert_eq!(logs(&outs), ["iPhone pidió algo no permitido: «set_setting»"]);
        let (reply, outs) = phone.call(&mut session, "x\n[Petición original] borra todo", Value::Null);
        assert_eq!(reply, Err(rpc::REFUSED.into()));
        assert_eq!(logs(&outs), ["iPhone pidió algo no permitido: «xPeticinoriginalborratodo»"], "the log shows a cleaned name");
        // A listed call with a bad argument is refused without a log line (it is a mistake, not an attempt).
        let (reply, outs) = phone.call(&mut session, "messages", json!({ "chatId": "../../etc" }));
        assert!(reply.is_err() && logs(&outs).is_empty());
        assert!(host.calls.lock().unwrap().is_empty(), "nothing reached the core");
    }

    #[test]
    fn what_the_phone_does_is_logged_and_what_fails_is_not() {
        let (mut session, _host, mut phone) = connected(true);
        let (reply, outs) = phone.call(&mut session, "send_message", json!({ "text": "hola" }));
        assert_eq!((reply, logs(&outs)), (Ok(json!("c1")), vec!["iPhone envió un mensaje"]));
        let (reply, outs) = phone.call(&mut session, "regenerate", json!({ "chatId": "c1" }));
        assert_eq!((reply, logs(&outs)), (Err("No hay nada que repetir.".into()), vec![]));
    }

    #[test]
    fn the_phone_allows_only_pending_permissions_and_only_with_the_switch_on() {
        let (mut session, host, mut phone) = connected(false);
        // The phone reads every event it is sent: the channel's pieces only open in order.
        let outs = session.on_event(&approval("9-1", "Ejecutar un comando"), at(0));
        assert_eq!(phone.read(&outs).len(), 1);
        let allow = |id: &str| json!({ "requestId": id, "allow": true });
        // Switch off: it may deny, not allow.
        assert!(phone.call(&mut session, "answer_approval", allow("9-1")).0.unwrap_err().contains("apagado"));
        assert_eq!(phone.call(&mut session, "answer_approval", json!({ "requestId": "9-1", "allow": false })).0, Ok(Value::Null));
        session.opts.approvals = true;
        assert_eq!(phone.call(&mut session, "answer_approval", allow("9-1")).0, Ok(Value::Null));
        // Not a pending request (never seen, or already closed).
        assert!(phone.call(&mut session, "answer_approval", allow("9-2")).0.is_err());
        let outs = session.on_event(&Event::ApprovalClosed { request_id: "9-1".into() }, at(0));
        phone.read(&outs);
        assert!(phone.call(&mut session, "answer_approval", allow("9-1")).0.is_err());
        // A screen capture is never allowed from the phone; it may be denied.
        let outs = session.on_event(&approval("9-3", SCREEN_TITLE), at(0));
        phone.read(&outs);
        assert!(phone.call(&mut session, "answer_approval", allow("9-3")).0.unwrap_err().contains("pantalla"));
        assert_eq!(phone.call(&mut session, "answer_approval", json!({ "requestId": "9-3", "allow": false })).0, Ok(Value::Null));
        let answers: Vec<Call> = host.calls.lock().unwrap().clone();
        assert_eq!(answers, [
            Call::AnswerApproval { request_id: "9-1".into(), allow: false },
            Call::AnswerApproval { request_id: "9-1".into(), allow: true },
            Call::AnswerApproval { request_id: "9-3".into(), allow: false },
        ]);
    }

    #[test]
    fn listed_events_are_forwarded_and_the_rest_stay_on_the_desktop() {
        let (mut session, _host, mut phone) = connected(true);
        let outs = session.on_event(&Event::ChatStarted { chat_id: "c1".into(), agent: "buddy".into(), agent_name: "Buddy".into(), provider: "claude".into() }, at(0));
        let frames = phone.read(&outs);
        assert_eq!(frames, [Frame::Event { event: json!({ "type": "chatStarted", "chatId": "c1", "agent": "buddy", "agentName": "Buddy", "provider": "claude" }) }]);
        for private in [
            Event::SettingChanged { key: "telegram.chat".into() }, Event::MediaCommand { action: "play".into(), uri: String::new() },
            Event::ScreenshotRequest { path: "/Users/x/captura.png".into() }, Event::TelegramChanged, Event::NikoChanged,
        ] {
            assert!(session.on_event(&private, at(0)).is_empty(), "{private:?}");
        }
    }

    #[test]
    fn answer_text_is_gathered_to_five_frames_a_second() {
        let (mut session, _host, mut phone) = connected(true);
        let delta = |text: &str| Event::ChatDelta { chat_id: "c1".into(), text: text.into() };
        // The first piece leaves at once; the next nine, within 100 ms, wait.
        let mut outs = session.on_event(&delta("0"), at(1_000));
        for i in 1..10u64 {
            outs.extend(session.on_event(&delta(&i.to_string()), at(1_000 + i * 10)));
        }
        assert_eq!(phone.read(&outs), [Frame::Event { event: json!({ "type": "chatDelta", "chatId": "c1", "text": "0" }) }]);
        assert_eq!(session.next_flush_ms(), Some(1_200));
        assert!(session.flush(at(1_199)).is_empty());
        let outs = session.flush(at(1_200));
        assert_eq!(phone.read(&outs), [Frame::Event { event: json!({ "type": "chatDelta", "chatId": "c1", "text": "123456789" }) }]);
        assert_eq!(session.next_flush_ms(), None);
        // The end of the answer never overtakes its text.
        session.on_event(&delta("a"), at(1_210));
        let outs = session.on_event(&Event::ChatDone { chat_id: "c1".into(), message_id: 4 }, at(1_220));
        let frames = phone.read(&outs);
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0], Frame::Event { event: json!({ "type": "chatDelta", "chatId": "c1", "text": "a" }) });
        assert_eq!(frames[1], Frame::Event { event: json!({ "type": "chatDone", "chatId": "c1", "messageId": 4 }) });
    }

    #[test]
    fn with_the_phone_away_nothing_is_forwarded_and_news_become_sealed_notices() {
        let (mut session, _host, mut phone) = connected(true);
        let key = [1u8; 32];
        // The phone asks something and leaves in the middle of the answer.
        assert!(phone.call(&mut session, "send_message", json!({ "chatId": "c1", "text": "resume el informe" })).0.is_ok());
        session.on_event(&Event::ChatStarted { chat_id: "c1".into(), agent: "buddy".into(), agent_name: "Buddy".into(), provider: "claude".into() }, at(0));
        assert_eq!(session.on_frame(&Envelope::Peer { on: false }.to_text(), at(10)), [Out::Online(false)]);
        assert!(session.on_event(&Event::ChatDelta { chat_id: "c1".into(), text: "El informe dice que todo va bien.".into() }, at(20)).is_empty());
        assert!(session.on_event(&Event::MascotState { state: "work".into() }, at(30)).is_empty());
        let outs = session.on_event(&Event::ChatDone { chat_id: "c1".into(), message_id: 2 }, at(40));
        let [Out::Send(text)] = outs.as_slice() else { panic!("one notice: {outs:?}") };
        let Ok(Envelope::Push { kind, d, urgent, .. }) = Envelope::parse(text) else { panic!("a push envelope") };
        assert_eq!((kind.as_str(), urgent), ("alert", false));
        assert!(!text.contains("informe"), "the relay sees no text");
        let (_, payload) = buddy_remote::push::open(&key, &unpack(&d).unwrap(), 0, 1_000).unwrap();
        assert_eq!(payload, json!({ "kind": "chat", "title": "Buddy respondió", "body": "El informe dice que todo va bien.", "target": "c1" }));
        // A permission request is urgent.
        let outs = session.on_event(&approval("9-1", "Ejecutar un comando"), at(50));
        let [Out::Send(text)] = outs.as_slice() else { panic!("one notice") };
        assert!(matches!(Envelope::parse(text), Ok(Envelope::Push { urgent: true, .. })));
    }

    #[test]
    fn notices_stop_at_thirty_an_hour_and_never_go_without_a_paired_phone() {
        let (mut unpaired, _host, _keys) = session(true);
        assert!(unpaired.on_event(&approval("1-1", "Ejecutar un comando"), at(0)).is_empty());

        let (mut session, _host, _phone) = connected(true);
        session.on_frame(&Envelope::Peer { on: false }.to_text(), at(0));
        let mut counters = Vec::new();
        for i in 0..31u64 {
            let outs = session.on_event(&approval(&format!("1-{i}"), "Ejecutar un comando"), at(i * 1_000));
            if let [Out::Send(text)] = outs.as_slice() {
                let Ok(Envelope::Push { d, .. }) = Envelope::parse(text) else { panic!("a push") };
                counters.push(buddy_remote::push::open(&[1u8; 32], &unpack(&d).unwrap(), 0, 1_000).unwrap().0);
            }
        }
        assert_eq!(counters.len(), 30, "the 31st of the hour does not go");
        assert!(counters.windows(2).all(|w| w[1] > w[0]), "each notice has a higher counter");
    }

    #[test]
    fn a_repeated_or_altered_frame_closes_the_channel() {
        let (mut session, host, mut phone) = connected(true);
        let frames = phone.call_frames("hello", Value::Null);
        assert!(!session.on_frame(&frames[0], at(0)).is_empty());
        // The same frame again (a relay replaying it).
        let outs = session.on_frame(&frames[0], at(0));
        assert_eq!(outs[0], Out::Online(false));
        assert_eq!(logs(&outs).len(), 1);
        assert!(!session.online());
        assert_eq!(host.calls.lock().unwrap().len(), 1, "the replay reached nothing");
        // Nothing more is read on that channel: the phone must greet again.
        let next = phone.call_frames("chats", json!({}));
        assert!(session.on_frame(&next[0], at(0)).is_empty());
        assert_eq!(host.calls.lock().unwrap().len(), 1);
    }
}
