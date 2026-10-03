//! Chrome/Edge native messaging, carried over Buddy's existing owner-only local channel.
use serde_json::{Value, json};
use std::io::{Read, Write};
const MAX_FRAME: usize = 64 * 1024;

fn frame(reader: &mut impl Read) -> Option<Value> {
    let mut size = [0; 4];
    reader.read_exact(&mut size).ok()?;
    let size = u32::from_ne_bytes(size) as usize;
    if size == 0 || size > MAX_FRAME {
        return None;
    }
    let mut bytes = vec![0; size];
    reader.read_exact(&mut bytes).ok()?;
    serde_json::from_slice::<Value>(&bytes)
        .ok()
        .filter(|v| v.is_object())
}
fn write(writer: &mut impl Write, value: &Value) -> std::io::Result<()> {
    let bytes = serde_json::to_vec(value)?;
    writer.write_all(&(bytes.len() as u32).to_ne_bytes())?;
    writer.write_all(&bytes)?;
    writer.flush()
}
pub fn main(origin: &str) {
    let expected = format!(
        "chrome-extension://{}/",
        include_str!("../../extensions/youtube/extension-id.txt").trim()
    );
    if origin != expected {
        return;
    }
    let source = format!(
        "{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0)
    );
    let (mut input, mut output) = (std::io::stdin().lock(), std::io::stdout().lock());
    while let Some(mut payload) = frame(&mut input) {
        // An extension cannot use this host to access the agent tools / approval channel.
        payload
            .as_object_mut()
            .unwrap()
            .retain(|k, _| ["browser", "videos", "action", "transcript"].contains(&k.as_str()));
        payload["_youtube"] = json!(true);
        payload["sourceId"] = json!(source);
        let raw = format!("{payload}\n");
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(crate::talk(&raw, true));
        });
        let reply = match rx.recv_timeout(std::time::Duration::from_secs(3)) {
            Ok(Some(line)) => {
                serde_json::from_str(&line).unwrap_or(json!({"enabled":false,"commands":[]}))
            }
            Ok(None) => json!({"enabled":false,"commands":[]}),
            Err(_) => break, // a wedged peer cannot accumulate abandoned workers
        };
        if write(&mut output, &reply).is_err() {
            break;
        }
    }
    let raw = format!(
        "{}\n",
        json!({"_youtube":true,"sourceId":source,"disconnect":true})
    );
    let _ = crate::talk(&raw, false);
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn frames_are_bounded_and_truncated_frames_fail() {
        let mut good = vec![];
        write(&mut good, &json!({"videos":[]})).unwrap();
        assert_eq!(frame(&mut &good[..]).unwrap(), json!({"videos":[]}));
        assert!(frame(&mut &good[..good.len() - 1]).is_none());
        assert!(frame(&mut &((MAX_FRAME as u32 + 1).to_ne_bytes())[..]).is_none());
    }
}
