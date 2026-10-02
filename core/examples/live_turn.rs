//! Runs one real turn against an installed CLI and prints its events: `cargo run --example live_turn -- claude "hola"`.
use buddy_core::providers::{Cancel, Provider, TurnRequest, claude::Claude, codex::Codex};

fn main() {
    let mut args = std::env::args().skip(1);
    let provider: Box<dyn Provider> = match args.next().as_deref() {
        Some("codex") => Box::new(Codex::new()),
        _ => Box::new(Claude::new()),
    };
    let prompt = args.next().unwrap_or_else(|| "Saluda en una frase corta.".into());
    let request = TurnRequest {
        prompt,
        system: "Eres Buddy, un asistente breve. Responde en español.".into(),
        workspace: std::env::temp_dir().join("buddy-live-turn"),
        effort: Some("low".into()),
        ..Default::default()
    };
    let start = std::time::Instant::now();
    let mut first = None;
    provider.run(&request, &Cancel::default(), &mut |event| {
        if first.is_none() && matches!(event, buddy_core::providers::TurnEvent::Delta(_)) {
            first = Some(start.elapsed());
        }
        println!("{:>6} ms {event:?}", start.elapsed().as_millis());
    });
    println!("first text after {:?}", first);
}
