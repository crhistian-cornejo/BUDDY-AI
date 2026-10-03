//! A console client that stands in for the phone, to try the phone link without the iPhone app.
//!
//!   cargo run -p buddy-core --example telefono -- '<buddy://pair?d=…>' ['un mensaje para Buddy']
//!
//! Paste the link behind the QR of Ajustes › iPhone (each code is good once, for five minutes). It pairs, greets,
//! asks who is there and for the chats, sends the message if one was given, and prints the events of its answer.

use std::time::Duration;

use buddy_core::remote::link;
use buddy_remote::pairing::{self, Offer};
use buddy_remote::wire::{Envelope, Frame, unpack};
use buddy_remote::{Channel, Keys};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio_tungstenite::tungstenite::Message;

struct Phone {
    socket: link::Socket,
    channel: Option<Channel>,
    next_id: u32,
}

impl Phone {
    /// The next envelope from the relay that `want` accepts (others are printed when they are news).
    async fn next(&mut self, want: impl Fn(&Envelope) -> bool) -> Result<Envelope, String> {
        loop {
            let frame = tokio::time::timeout(Duration::from_secs(60), self.socket.next()).await.map_err(|_| "El equipo no respondió en un minuto.")?;
            let Some(Ok(Message::Text(text))) = frame else { return Err("El relé cerró la conexión.".into()) };
            match Envelope::parse(text.as_str()) {
                Ok(envelope) if want(&envelope) => return Ok(envelope),
                Ok(Envelope::Peer { on }) => println!("· el equipo {} conectado al relé", if on { "está" } else { "no está" }),
                Ok(Envelope::Error { code }) => println!("· el relé dice: {code}"),
                _ => {}
            }
        }
    }

    async fn send(&mut self, envelope: Envelope) -> Result<(), String> {
        self.socket.send(Message::text(envelope.to_text())).await.map_err(|_| "No se pudo enviar al relé.".to_string())
    }

    /// The next decrypted frame from the desktop.
    async fn frame(&mut self) -> Result<Frame, String> {
        loop {
            let Envelope::Msg { d } = self.next(|e| matches!(e, Envelope::Msg { .. })).await? else { unreachable!() };
            let piece = unpack(&d).map_err(|e| e.to_string())?;
            if let Some(bytes) = self.channel.as_mut().ok_or("Sin canal.")?.open(&piece).map_err(|e| e.to_string())? {
                return Frame::parse(&bytes).map_err(|e| e.to_string());
            }
        }
    }

    /// Calls the desktop and waits for its reply, printing the events that arrive meanwhile.
    async fn call(&mut self, name: &str, args: Value) -> Result<Value, String> {
        self.next_id += 1;
        let id = self.next_id;
        let frame = Frame::Call { id, call: name.into(), args };
        let pieces = self.channel.as_mut().ok_or("Sin canal.")?.seal(&frame.to_bytes()).map_err(|e| e.to_string())?;
        for piece in pieces {
            self.send(Envelope::msg(&piece)).await?;
        }
        loop {
            match self.frame().await? {
                Frame::Reply { id: got, ok, err } if got == id => return err.map_or(Ok(ok.unwrap_or(Value::Null)), Err),
                Frame::Event { event } => print_event(&event),
                _ => {}
            }
        }
    }
}

fn print_event(event: &Value) {
    match event["type"].as_str().unwrap_or("") {
        "chatDelta" => {
            use std::io::Write;
            print!("{}", event["text"].as_str().unwrap_or(""));
            let _ = std::io::stdout().flush();
        }
        "chatStarted" => println!("\n— {} responde —", event["agentName"].as_str().unwrap_or("Buddy")),
        "chatTool" => println!("\n  [{}] {}", event["name"].as_str().unwrap_or(""), event["summary"].as_str().unwrap_or("")),
        "mascotState" | "usageChanged" | "chatActivity" => {}
        kind => println!("\n· {kind}"),
    }
}

async fn run() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let uri = args.next().ok_or("Uso: telefono '<buddy://pair?d=…>' ['mensaje']")?;
    let message = args.next();
    let offer = Offer::parse(&uri).map_err(|e| e.to_string())?;
    let keys = Keys::generate();
    let desktop = offer.desktop_public().map_err(|e| e.to_string())?;

    println!("Conectando con {} …", offer.relay);
    let socket = link::open(&offer.relay, &offer.room, &offer.key, "phone").await?;
    let mut phone = Phone { socket, channel: None, next_id: 0 };

    let hello = pairing::hello(&offer, &keys.public, "Teléfono de consola");
    phone.send(Envelope::Pair { d: serde_json::to_string(&hello).map_err(|e| e.to_string())? }).await?;
    let Envelope::Paired { d } = phone.next(|e| matches!(e, Envelope::Paired { .. })).await? else { unreachable!() };
    if !offer.check_ack(&keys.public, &d) {
        return Err("La prueba del equipo no coincide: no es el que mostró el código.".into());
    }
    println!("Emparejado.");

    let (hand, hs1) = Channel::initiator(&keys, &desktop, &offer.room).map_err(|e| e.to_string())?;
    phone.send(Envelope::hs1(&hs1)).await?;
    let Envelope::Hs2 { d } = phone.next(|e| matches!(e, Envelope::Hs2 { .. })).await? else { unreachable!() };
    phone.channel = Some(hand.finish(&unpack(&d).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?);
    println!("Canal cifrado abierto.");

    let who = phone.call("hello", Value::Null).await?;
    println!("Equipo: {} ({}, Buddy {})", who["name"].as_str().unwrap_or("?"), who["platform"].as_str().unwrap_or("?"), who["version"].as_str().unwrap_or("?"));
    let chats = phone.call("chats", json!({ "limit": 5 })).await?;
    for chat in chats.as_array().into_iter().flatten() {
        println!("  · {}", chat["title"].as_str().unwrap_or(""));
    }
    // What is refused stays refused, whoever asks.
    println!("set_setting → {}", phone.call("set_setting", json!({ "key": "commands.enabled", "value": "true" })).await.unwrap_err());

    if let Some(text) = message {
        let chat_id = phone.call("send_message", json!({ "text": text })).await?;
        println!("Mensaje enviado al chat {chat_id}. Esperando la respuesta…");
        loop {
            match phone.frame().await? {
                Frame::Event { event } if event["type"] == "chatDone" || event["type"] == "chatFailed" => {
                    println!("\n— fin ({}) —", event["type"].as_str().unwrap_or(""));
                    break;
                }
                Frame::Event { event } => print_event(&event),
                _ => {}
            }
        }
    }
    Ok(())
}

fn main() {
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().expect("a runtime");
    if let Err(error) = runtime.block_on(run()) {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
