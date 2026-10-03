// No console window on Windows in release.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    if std::env::current_exe().ok().and_then(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned())).is_some_and(|s| s == "buddy-youtube-host") {
        buddy_hook::youtube::main(&std::env::args().nth(1).unwrap_or_default());
        return;
    }
    buddy_lib::run()
}
