// Small append-only log at %LOCALAPPDATA%\MIKA\mika.log. Nothing leaves the
// machine, and no secret, token or request body is ever written here.

use std::io::Write;

use windows::Win32::System::SystemInformation::GetLocalTime;

use super::settings;

pub(crate) fn line(message: impl AsRef<str>) {
    let t = unsafe { GetLocalTime() };
    let stamp = format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond
    );
    let dir = settings::local_dir();
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let path = dir.join("mika.log");
    // Keep it from growing forever: start fresh past ~1 MB.
    if std::fs::metadata(&path).map(|m| m.len() > 1_000_000).unwrap_or(false) {
        let _ = std::fs::remove_file(&path);
    }
    if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(file, "{stamp} {}", message.as_ref());
    }
}
