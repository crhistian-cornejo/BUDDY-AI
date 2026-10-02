//! The helper process MIKA runs to read Telegram (see lib.rs). One JSON object per line on stdin, one answer per line
//! on stdout, in order:
//!
//! ```text
//! → {"id":1,"cmd":"connect","apiId":123,"apiHash":"…","session":"mtg1|…"}   ← {"id":1,"ok":true,"result":null}
//! → {"id":2,"cmd":"status"}                                                  ← {"id":2,"ok":true,"result":{…}}
//! → {"id":3,"cmd":"sendCode","phone":"+51…"} · signIn{code} · password{password} · signOut
//! → {"id":4,"cmd":"chats","limit":200}
//! → {"id":5,"cmd":"fetch","chats":[{"chat":{…},"after":0}],"limit":20,"mediaDir":"C:\\…"}
//!                     ← {"id":5,"ok":true,"result":{"posts":[…],"last":[{"chatId":…,"last":…}],"errors":[…]}}
//! ```
//!
//! Whenever the session to keep changes, a line `{"event":"session","value":"mtg1|…"}` (or `null` after signing out)
//! comes before the answer; MIKA stores it in the OS vault. Nothing else is ever printed: no logs, no secrets.
//! The helper exits when stdin closes (MIKA quit) or after a long idle spell.

use std::io::Write;
use std::path::PathBuf;
use std::time::Duration;

use mika_telegram::{ChatRef, Telegram};
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, BufReader};

/// MIKA polls every few minutes; without a request for this long nobody needs the connection.
const IDLE_EXIT: Duration = Duration::from_secs(15 * 60);
/// A request line bigger than this is not one of MIKA's.
const MAX_LINE: usize = 1_000_000;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct FetchChat { chat: ChatRef, #[serde(default)] after: i32 }

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    let mut telegram: Option<Telegram> = None;
    let mut last_session: Option<String> = None;
    loop {
        let line = match tokio::time::timeout(IDLE_EXIT, lines.next_line()).await {
            Ok(Ok(Some(line))) => line,
            _ => break, // stdin closed, unreadable, or idle for too long
        };
        if line.len() > MAX_LINE { continue; }
        let Ok(request) = serde_json::from_str::<Value>(&line) else { continue };
        let id = request["id"].clone();
        let cmd = request["cmd"].as_str().unwrap_or("");
        if cmd == "quit" { break; }
        let result = handle(cmd, &request, &mut telegram).await;
        let session = telegram.as_ref().and_then(Telegram::session);
        if session != last_session && (session.is_some() || cmd == "signOut") {
            send(&json!({ "event": "session", "value": session }));
            last_session = session;
        }
        match result {
            Ok(value) => send(&json!({ "id": id, "ok": true, "result": value })),
            Err(error) => send(&json!({ "id": id, "ok": false, "error": error })),
        }
    }
}

async fn handle(cmd: &str, request: &Value, telegram: &mut Option<Telegram>) -> Result<Value, String> {
    if cmd == "connect" {
        let api_id = request["apiId"].as_i64().and_then(|v| i32::try_from(v).ok()).filter(|v| *v > 0).ok_or("Falta el api_id.")?;
        let api_hash = request["apiHash"].as_str().filter(|h| h.len() >= 16 && h.chars().all(|c| c.is_ascii_hexdigit())).ok_or("El api_hash no es válido.")?;
        *telegram = Some(Telegram::connect(api_id, api_hash, request["session"].as_str()));
        return Ok(Value::Null);
    }
    let tg = telegram.as_ref().ok_or("Telegram no está conectado.")?;
    let text = |key: &str| request[key].as_str().unwrap_or("").to_string();
    match cmd {
        "status" => to_value(tg.status().await?),
        "sendCode" => to_value(tg.send_code(&text("phone")).await?),
        "signIn" => to_value(tg.sign_in(&text("code")).await?),
        "password" => to_value(tg.password(&text("password")).await?),
        "signOut" => { tg.sign_out().await?; Ok(Value::Null) }
        "chats" => to_value(tg.chats(request["limit"].as_u64().unwrap_or(200).min(500) as usize).await?),
        "fetch" => {
            let chats: Vec<FetchChat> = serde_json::from_value(request["chats"].clone()).map_err(|_| "Lista de chats no válida.")?;
            let limit = request["limit"].as_u64().unwrap_or(20).clamp(1, 100) as usize;
            let media = PathBuf::from(text("mediaDir"));
            if !media.is_absolute() { return Err("Carpeta de imágenes no válida.".into()); }
            let mut posts = Vec::new();
            let mut last = Vec::new();
            let mut errors = Vec::new();
            for FetchChat { chat, after } in chats.iter().take(50) {
                // A fresh pick only brings its last few posts, not the whole history.
                let limit = if *after == 0 { limit.min(5) } else { limit };
                match tg.fetch(chat, *after, limit, &media).await {
                    Ok((mut found, newest)) => {
                        posts.append(&mut found);
                        last.push(json!({ "chatId": chat.id, "last": newest }));
                    }
                    Err(error) => errors.push(json!({ "chatId": chat.id, "error": error })),
                }
            }
            Ok(json!({ "posts": posts, "last": last, "errors": errors }))
        }
        _ => Err("Orden desconocida.".into()),
    }
}

fn to_value<T: serde::Serialize>(value: T) -> Result<Value, String> {
    serde_json::to_value(value).map_err(|_| "Respuesta no válida.".to_string())
}

fn send(value: &Value) {
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{value}");
    let _ = out.flush();
}
