//! One real orchestrated turn in a scratch data folder: `cargo run --example live_chat -- "pregunta"`.
use buddy_core::{BuddyCore, Event};

fn main() {
    let question = std::env::args().nth(1).unwrap_or_else(|| "¿Cómo llega el Real Madrid a su próximo partido? Breve.".into());
    let dir = std::env::temp_dir().join("buddy-live-chat");
    let _ = std::fs::remove_dir_all(&dir);
    let core = BuddyCore::open(&dir).expect("core");
    let rx = core.events();
    if std::env::var("BUDDY_PREWARM").is_ok() {
        core.prewarm();
        std::thread::sleep(std::time::Duration::from_secs(4));
    }
    let start = std::time::Instant::now();
    let mut first = None;
    let chat = core.send_message(None, question, std::env::var("BUDDY_ATTACH").map(|a| vec![a]).unwrap_or_default()).expect("send");
    while let Ok(event) = rx.recv() {
        let end = matches!(&event, Event::MascotState { state } if ["done", "error", "idle"].contains(&state.as_str()));
        match &event {
            Event::ChatDelta { .. } => {
                if first.is_none() {
                    first = Some(start.elapsed());
                    println!("\nprimer texto a los {:?}", start.elapsed());
                }
                print!(".")
            }
            other => println!("\n{:>6} ms {other:?}", start.elapsed().as_millis()),
        }
        if end {
            break;
        }
    }
    for m in core.messages(chat).unwrap() {
        println!("\n[{} · {}] {}", m.role, m.agent, m.text.chars().take(300).collect::<String>());
    }
}
