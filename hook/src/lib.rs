//! Native browser bridge reused by the Windows app's executable (no separate download or installer).
mod transport;
#[cfg(windows)]
mod win;
pub mod youtube;
fn talk(payload: &str, waits: bool) -> Option<String> {
    use std::io::{BufRead, Read, Write};
    let mut conn = transport::connect()?;
    conn.write_all(payload.as_bytes()).ok()?;
    conn.flush().ok()?;
    if !waits {
        return None;
    }
    let mut reply = vec![];
    std::io::BufReader::new(conn.take(4097))
        .read_until(b'\n', &mut reply)
        .ok()?;
    (reply.len() <= 4096).then(|| String::from_utf8_lossy(&reply).trim().into())
}
