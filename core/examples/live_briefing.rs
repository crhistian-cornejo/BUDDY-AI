//! One real briefing run (cheap Claude turn with web search) on a throwaway data folder.
//! `cargo run --example live_briefing`

use std::time::Duration;

use buddy_core::{BuddyCore, Event};

fn main() {
    let dir = tempfile::tempdir().unwrap();
    let core = BuddyCore::open(dir.path()).unwrap();
    let rx = core.events();
    let started = std::time::Instant::now();
    core.briefing_now();
    loop {
        match rx.recv_timeout(Duration::from_secs(170)) {
            Ok(Event::BriefingReady { count, headline }) => {
                println!("{count} mensajitos en {:.1} s; el primero: {headline}", started.elapsed().as_secs_f32());
                break;
            }
            Ok(_) => continue,
            Err(_) => {
                println!("sin mensajitos (nada nuevo o sin respuesta)");
                break;
            }
        }
    }
    for item in core.briefing() {
        println!("- [{}] {} {}", item.topic, item.text, item.url.unwrap_or_default());
    }
    for row in core.token_report(1).unwrap() {
        println!("tokens · {}: {} entrada, {} salida", row.feature, row.input + row.cached, row.output);
    }
}
