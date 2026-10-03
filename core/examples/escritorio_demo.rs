//! A stand-in desktop for trying the iPhone app without touching the real Buddy: it pairs through a relay and
//! answers the phone's calls with invented chats, a scripted answer that streams, a coding session and a permission
//! request. No model runs, nothing is read from this Mac, and its keys live only in memory.
//!
//!   cargo run -p buddy-core --example escritorio_demo -- http://127.0.0.1:8799 <clave de dueño del relé>
//!
//! It prints the pairing link (what the QR of Ajustes › iPhone encodes) and keeps running until Ctrl-C.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use buddy_core::remote::rpc::{Call, Host};
use buddy_core::remote::{HttpRooms, Remote, Vault};
use buddy_core::{AgentLook, Event, EventBus};
use serde_json::{Value, json};

#[derive(Default)]
struct MemoryVault(Mutex<HashMap<String, String>>);

impl Vault for MemoryVault {
    fn get(&self, name: &str) -> Option<String> {
        self.0.lock().unwrap().get(name).cloned()
    }
    fn set(&self, name: &str, value: &str) -> Result<(), String> {
        self.0.lock().unwrap().insert(name.into(), value.into());
        Ok(())
    }
    fn delete(&self, name: &str) {
        self.0.lock().unwrap().remove(name);
    }
}

fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

struct DemoHost {
    bus: Arc<EventBus>,
    /// The messages of each chat, as the real core lists them.
    chats: Mutex<Vec<(String, String, Vec<Value>)>>,
}

impl DemoHost {
    fn new(bus: Arc<EventBus>) -> Self {
        let message = |id: i64, role: &str, agent: &str, text: &str| {
            json!({ "id": id, "role": role, "agent": agent, "provider": if role == "assistant" { json!("claude") } else { Value::Null }, "text": text,
                "sources": [], "failed": false, "createdAt": now() - 3600 + id * 60, "attachments": [], "model": if role == "assistant" { json!("Sonnet 5.5 · esfuerzo medio") } else { Value::Null }, "took": if role == "assistant" { json!("4.2 s") } else { Value::Null } })
        };
        let chats = vec![
            ("demo-viaje".to_string(), "Plan para el viaje a Cusco".to_string(), vec![
                message(1, "user", "buddy", "Ayúdame a planear tres días en Cusco"),
                message(2, "assistant", "buddy", "Claro. Te propongo esto:\n\n**Día 1** · Centro histórico y San Blas, con calma por la altura.\n**Día 2** · Valle Sagrado: Pisac y Ollantaytambo.\n**Día 3** · Machu Picchu saliendo en el primer tren.\n\n¿Quieres que arme el presupuesto?"),
            ]),
            ("demo-gastos".to_string(), "Gastos del mes".to_string(), vec![
                message(1, "user", "buddy", "¿Cómo voy con el presupuesto de este mes?"),
                message(2, "assistant", "niko", "Vas en 62 % del presupuesto. Comida es la categoría más alta (S/. 480 de S/. 600)."),
            ]),
        ];
        Self { bus, chats: Mutex::new(chats) }
    }

    /// The answer to a message, written a few words at a time, as a model would.
    fn answer(&self, chat_id: String, text: String) {
        let bus = self.bus.clone();
        std::thread::spawn(move || {
            let reply = format!("Esto es una respuesta de demostración a «{text}».\n\nCuando emparejes tu Mac o tu PC de verdad, aquí responderán **Buddy** y sus especialistas con tu suscripción, y verás el texto llegar así, poco a poco.\n\n- Los chats son los de esa máquina.\n- Los permisos se aprueban con Face ID.\n- Con la app cerrada llegan avisos.");
            std::thread::sleep(Duration::from_millis(400));
            bus.publish(Event::ChatStarted { chat_id: chat_id.clone(), agent: "buddy".into(), agent_name: "Buddy".into(), provider: "claude".into() });
            bus.publish(Event::ChatActivity { chat_id: chat_id.clone(), kind: "web".into(), label: "Buscando en la web".into() });
            std::thread::sleep(Duration::from_millis(900));
            for word in reply.split_inclusive(' ') {
                bus.publish(Event::ChatDelta { chat_id: chat_id.clone(), text: word.into() });
                std::thread::sleep(Duration::from_millis(45));
            }
            bus.publish(Event::ChatDone { chat_id, message_id: 99 });
        });
    }
}

impl Host for DemoHost {
    fn run(&self, call: Call) -> Result<Value, String> {
        Ok(match call {
            Call::Hello => json!({ "name": "Mac de demostración", "platform": "macos", "version": env!("CARGO_PKG_VERSION") }),
            Call::Chats { .. } | Call::SearchChats { .. } => json!(self.chats.lock().unwrap().iter().map(|(id, title, messages)| {
                json!({ "id": id, "title": title, "updatedAt": now() - 1800, "preview": messages.last().and_then(|m| m["text"].as_str()).unwrap_or("").chars().take(120).collect::<String>() })
            }).collect::<Vec<_>>()),
            Call::Messages { chat_id } => json!(self.chats.lock().unwrap().iter().find(|(id, ..)| *id == chat_id).map(|(_, _, m)| m.clone()).unwrap_or_default()),
            Call::Agents => json!([
                { "id": "buddy", "name": "Buddy", "specialty": "Tu asistente", "provider": "claude" },
                { "id": "niko", "name": "Niko", "specialty": "Finanzas personales", "provider": "claude" },
                { "id": "parley", "name": "PARLEY", "specialty": "Deportes y estadísticas", "provider": "claude" },
            ]),
            Call::AgentSprite { agent_id } => {
                let sprite = buddy_core::look::sprite(&AgentLook::buddy(), &agent_id, "Buddy").map_err(|e| e.to_string())?;
                serde_json::to_value(sprite).map_err(|e| e.to_string())?
            }
            Call::Sprite { id } => serde_json::to_value(buddy_core::pixel::builtin_sprite(&id).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?,
            Call::Sessions => json!([
                { "sessionId": "s-demo-1", "agent": "claude", "project": "buddy", "state": "working", "updatedAt": now() - 40 },
                { "sessionId": "s-demo-2", "agent": "codex", "project": "sitio-web", "state": "done", "updatedAt": now() - 900 },
            ]),
            Call::Usage => json!([
                { "provider": "claude", "name": "Claude", "windows": [{ "label": "5 h", "usedPct": 38.0, "resetsAt": now() + 7200, "seenAt": now() }, { "label": "semana", "usedPct": 61.0, "resetsAt": now() + 250000, "seenAt": now() }] },
                { "provider": "codex", "name": "Codex", "windows": [{ "label": "5 h", "usedPct": 12.0, "resetsAt": now() + 9000, "seenAt": now() }] },
                { "provider": "antigravity", "name": "Gemini", "windows": [{ "label": "día", "usedPct": 24.0, "resetsAt": now() + 40000, "seenAt": now() }] },
            ]),
            Call::Briefing => json!([
                { "topic": "Ingeniería", "text": "Se publicó una guía nueva sobre túneles en roca blanda.", "url": null, "at": now() - 5000 },
                { "topic": "Deportes", "text": "Hoy juega la selección a las 20:30.", "url": null, "at": now() - 7000 },
            ]),
            Call::FocusStatus | Call::FocusStop => json!({ "running": false, "startedAt": 0, "endsAt": 0, "minutes": 0 }),
            Call::FocusStart { minutes } => json!({ "running": true, "startedAt": now(), "endsAt": now() + minutes as i64 * 60, "minutes": minutes }),
            Call::ChatCommands | Call::QueuedMessages { .. } => json!([]),
            Call::ChatSuggestions => json!(["¿Qué tengo pendiente hoy?", "Resume mis gastos de la semana"]),
            Call::VoiceVocabulary => json!(["Buddy", "Niko", "PARLEY"]),
            Call::ImagePreview { .. } => Value::Null,
            Call::SendMessage { chat_id, text } => {
                let id = chat_id.unwrap_or_else(|| format!("demo-{}", now()));
                let mut chats = self.chats.lock().unwrap();
                if !chats.iter().any(|(existing, ..)| *existing == id) {
                    chats.insert(0, (id.clone(), text.chars().take(40).collect(), Vec::new()));
                }
                drop(chats);
                self.answer(id.clone(), text);
                json!(id)
            }
            Call::AnswerApproval { request_id, allow } => {
                println!("· el teléfono {} el permiso {request_id}", if allow { "permitió" } else { "rechazó" });
                self.bus.publish(Event::ApprovalClosed { request_id });
                Value::Null
            }
            Call::CancelChat { .. } | Call::Regenerate { .. } | Call::RemoveQueued { .. } => Value::Null,
        })
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let (Some(relay), Some(owner)) = (args.next(), args.next()) else {
        eprintln!("Uso: escritorio_demo <dirección del relé> <clave de dueño>");
        std::process::exit(2);
    };
    let dir = std::env::temp_dir().join(format!("buddy-demo-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("a temporary folder");
    let store = Arc::new(Mutex::new(buddy_core::store::Store::open(&dir.join("demo.sqlite")).expect("a store")));
    let bus = Arc::new(EventBus::default());
    let remote = Remote::new(store, bus.clone(), Box::new(MemoryVault::default()), Box::new(HttpRooms), Arc::new(DemoHost::new(bus.clone())));
    let started = remote.set_relay(&relay, &owner).and_then(|_| remote.pair());
    let offer = match started {
        Ok(offer) => offer,
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(1);
        }
    };
    println!("ENLACE {}", offer.uri);
    println!("Vale cinco minutos y sirve una vez. Esperando al teléfono…");
    let mut asked = false;
    loop {
        std::thread::sleep(Duration::from_secs(2));
        let status = remote.status();
        // Once the phone is in, a coding agent asks for a permission: the card to try with Face ID.
        if status.online && !asked {
            asked = true;
            let bus = bus.clone();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_secs(12));
                bus.publish(Event::ApprovalRequest {
                    request_id: "7-1".into(), session_id: "s-demo-1".into(), agent: "claude".into(), project: "buddy".into(),
                    title: "Ejecutar un comando".into(), summary: "git push origin main".into(),
                    detail: "git push origin main\n\nDescripción: Subir los cambios de hoy".into(), can_allow: true, always: String::new(),
                });
            });
        }
        if !status.online {
            asked = false;
        }
    }
}
