//! Serves the hooks socket/pipe of a scratch data folder and prints what arrives; every approval is answered with
//! the given word after a second. Pair it with the relay by hand:
//!
//! ```text
//! cargo build -p buddy-hook
//! BUDDY_DATA_DIR=/tmp/buddy-hooks cargo run -p buddy-core --example live_hooks -- target/debug/buddy-hook allow
//! echo '{"hook_event_name":"PermissionRequest","session_id":"s","cwd":"/tmp","tool_name":"Bash","tool_input":{"command":"ls"}}' \
//!   | BUDDY_DATA_DIR=/tmp/buddy-hooks /tmp/buddy-hooks/bin/buddy-hook PermissionRequest
//! ```
use buddy_core::{BuddyCore, Event};

fn main() {
    let mut args = std::env::args().skip(1);
    let relay = args.next().unwrap_or_default();
    let allow = args.next().as_deref() != Some("deny");
    // Empty = `$BUDDY_DATA_DIR`, else the platform default (where the relay looks too).
    let core = std::sync::Arc::new(BuddyCore::open("").expect("core"));
    let rx = core.events();
    core.start_sessions(relay).expect("sessions");
    println!("escuchando en {} — {:?}", core.data_dir(), core.hooks_status());
    while let Ok(event) = rx.recv() {
        println!("{event:?}");
        if let Event::ApprovalRequest { request_id, .. } = event {
            let core = core.clone();
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_secs(1));
                core.answer_approval(request_id, allow);
            });
        }
    }
}
