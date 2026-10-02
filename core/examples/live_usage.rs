//! Asks Claude and Codex for their plan figures and prints them: `cargo run --example live_usage`.
use buddy_core::BuddyCore;

fn main() {
    let dir = std::env::temp_dir().join("buddy-live-usage");
    let core = BuddyCore::open(&dir).expect("core");
    core.refresh_usage();
    std::thread::sleep(std::time::Duration::from_secs(25));
    for p in core.usage() {
        for w in p.windows {
            println!("{} · {}: {:.0} % (reinicia {:?})", p.name, w.label, w.used_pct, w.resets_at);
        }
    }
}
