//! Read-only personal Telegram account. Requests are explicit, serialized and bounded; no idle polling.
//! Credentials and MTProto authorization keys live in the OS vault, never in SQLite or logs.
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use buddy_telegram::{ChatRef, Post, Telegram};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use crate::store::Store;

const SERVICE: &str = "io.github.crhistian-cornejo.buddy";
const CONFIG: &str = "telegram-account-api";
const SESSION: &str = "telegram-account-session";
const SELECTED: &str = "telegram.account.selected";
const POSTS: &str = "telegram.account.posts";
const MAX_CHATS: usize = 10;
const MAX_POSTS: usize = 200;
const COVERAGE: &str = "telegram.account.coverage";

#[derive(Serialize, Deserialize, PartialEq)]
struct Credentials { id: i32, hash: String }
fn credentials(input: &Value, current: Option<&str>) -> Result<Credentials, String> {
    let id = input["id"].as_i64().and_then(|v| i32::try_from(v).ok()).filter(|v| *v > 0).ok_or("api_id no válido.")?;
    let supplied = input["hash"].as_str().unwrap_or("").trim();
    let hash = if supplied.is_empty() {
        current.and_then(|s| serde_json::from_str::<Credentials>(s).ok()).map(|c| c.hash).unwrap_or_default()
    } else { supplied.into() };
    if hash.len() != 32 || !hash.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err("api_hash no válido (32 caracteres).".into());
    }
    Ok(Credentials { id, hash })
}
struct Connection { runtime: tokio::runtime::Runtime, telegram: Telegram }

pub struct Account {
    connection: Mutex<Option<Connection>>,
    store: Arc<Mutex<Store>>,
    media: PathBuf,
}

fn entry(name: &str) -> Result<keyring::Entry, String> {
    keyring::Entry::new(SERVICE, name).map_err(|_| "No se pudo abrir el almacén de credenciales.".into())
}
fn secret(name: &str) -> Result<Option<String>, String> {
    match entry(name)?.get_password() {
        Ok(v) => Ok(Some(v)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(_) => Err("No se pudo leer el almacén de credenciales.".into()),
    }
}
fn keep(name: &str, value: &str) -> Result<(), String> {
    entry(name)?.set_password(value).map_err(|_| "No se pudo guardar en el almacén de credenciales.".into())
}
fn forget(name: &str) -> Result<(), String> {
    match entry(name)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(_) => Err("No se pudo borrar la sesión del almacén de credenciales.".into()),
    }
}
fn selection(available: &[ChatRef], ids: &[i64]) -> Result<Vec<ChatRef>, String> {
    if ids.len() > MAX_CHATS { return Err("Elige como máximo 10 grupos.".into()); }
    let mut selected = Vec::new();
    for id in ids {
        let chat = available.iter().find(|c| c.id == *id).ok_or("Ese grupo no está disponible en tu cuenta.")?;
        if !selected.iter().any(|c: &ChatRef| c.id == *id) { selected.push(chat.clone()); }
    }
    Ok(selected)
}
fn merge_posts(old: Vec<Post>, fresh: Vec<Post>, chosen: &[ChatRef]) -> Vec<Post> {
    let mut posts: Vec<Post> = old.into_iter().filter(|p| chosen.iter().any(|c| c.id == p.chat_id)).collect();
    for post in fresh {
        if !chosen.iter().any(|c| c.id == post.chat_id) { continue; }
        posts.retain(|p| (p.chat_id, p.id) != (post.chat_id, post.id));
        posts.push(post);
    }
    posts.sort_by_key(|p| (p.date, p.chat_id, p.id));
    if posts.len() > MAX_POSTS { posts.drain(..posts.len() - MAX_POSTS); }
    posts
}

impl Account {
    pub fn new(store: Arc<Mutex<Store>>, data_dir: PathBuf) -> Self {
        Self { connection: Mutex::new(None), store, media: data_dir.join("telegram-account-media") }
    }
    fn read<T: serde::de::DeserializeOwned + Default>(&self, key: &str) -> T {
        self.store.lock().unwrap_or_else(|p| p.into_inner()).setting(key).ok().flatten()
            .and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
    }
    fn write<T: Serialize>(&self, key: &str, value: &T) -> Result<(), String> {
        let text = serde_json::to_string(value).map_err(|_| "Datos de Telegram no válidos.")?;
        self.store.lock().unwrap_or_else(|p| p.into_inner()).set_setting(key, &text).map_err(|e| e.to_string())
    }
    /// Used only on the UI thread's background task. No arbitrary Telegram method or peer is accepted.
    pub fn request(&self, action: &str, value: &str) -> Result<String, String> {
        self.request_at(action, value, crate::parley::now())
    }

    fn request_at(&self, action: &str, value: &str, now: i64) -> Result<String, String> {
        let mut held = self.connection.lock().unwrap_or_else(|p| p.into_inner());
        if action == "configure" {
            let input: Value = serde_json::from_str(value).map_err(|_| "Revisa api_id y api_hash.")?;
            let saved = secret(CONFIG)?;
            let credentials = credentials(&input, saved.as_deref())?;
            // Re-entering the same API credentials must not erase an authorized session.
            let config = serde_json::to_string(&credentials).unwrap();
            if saved.as_deref().and_then(|s| serde_json::from_str::<Credentials>(s).ok()).as_ref() != Some(&credentials) {
                forget(SESSION)?;
                *held = None;
                self.write(SELECTED, &Vec::<ChatRef>::new())?;
                self.write(POSTS, &Vec::<Post>::new())?;
                self.prune_media(&[]);
                keep(CONFIG, &config)?;
            }
        }
        let Some(raw) = secret(CONFIG)? else {
            if action != "status" { return Err("Guarda primero api_id y api_hash de my.telegram.org.".into()); }
            return Ok(json!({"configured":false,"authorized":false,"step":"idle","chats":[],"selected":[],"posts":[]}).to_string());
        };
        let config: Credentials = serde_json::from_str(&raw).map_err(|_| "Vuelve a guardar las credenciales de Telegram.")?;
        if held.is_none() {
            let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|_| "No se pudo iniciar Telegram.")?;
            let saved = secret(SESSION)?;
            let telegram = { let _enter = runtime.enter(); Telegram::connect(config.id, &config.hash, saved.as_deref()) };
            *held = Some(Connection { runtime, telegram });
        }
        let connection = held.as_ref().unwrap();
        let result = connection.runtime.block_on(async { tokio::time::timeout(Duration::from_secs(90), async {
            let tg = &connection.telegram;
            match action {
                "status" | "configure" => {},
                "sendCode" => { tg.send_code(value).await?; },
                "signIn" => { tg.sign_in(value).await?; },
                "password" => { tg.password(value).await?; },
                "signOut" => { tg.sign_out().await?; },
                "chats" | "select" | "fetch" | "today" => {},
                _ => return Err("Acción de Telegram no válida.".into()),
            }
            let status = tg.status().await?;
            let mut chosen: Vec<ChatRef> = self.read(SELECTED);
            let mut posts: Vec<Post> = self.read(POSTS);
            let mut chats = Vec::new();
            let mut errors: Vec<String> = Vec::new();
            let mut unavailable = Vec::new();
            if matches!(action, "chats" | "select" | "fetch" | "today") {
                if !status.authorized { return Err("Inicia sesión primero.".into()); }
                // Refresh current membership/protection before every read. Never trust a persisted access hash.
                chats = tg.chats(300).await?;
                if action == "select" {
                    let ids: Vec<i64> = serde_json::from_str(value).map_err(|_| "Selección no válida.")?;
                    chosen = selection(&chats, &ids)?;
                } else {
                    for c in chosen.iter().filter(|c| !chats.iter().any(|a| a.id == c.id)) {
                        let error = format!("{}: grupo no disponible en la lista actual; no se pudo revisar.", c.title);
                        errors.push(error.clone());
                        unavailable.push(json!({"group":c.title,"complete":false,"error":error}));
                    }
                    chosen.retain(|c| chats.iter().any(|a| a.id == c.id));
                    for c in &mut chosen { *c = chats.iter().find(|a| a.id == c.id).unwrap().clone(); }
                }
                posts = merge_posts(posts, vec![], &chosen);
                if matches!(action, "fetch" | "today") {
                    if chosen.is_empty() { return Err("Elige primero al menos un grupo.".into()); }
                    let mut fresh = Vec::new();
                    let mut coverage = unavailable;
                    let (start, _) = crate::parley::day_window(now);
                    for chat in &chosen {
                        match tg.fetch_since(chat, 0, if action == "today" { 200 } else { 20 }, &self.media,
                            (action == "today").then_some(start)).await {
                            Ok((found, _, complete)) => {
                                coverage.push(json!({"group":chat.title,"count":found.len(),"complete":complete,"at":now}));
                                fresh.extend(found);
                            },
                            Err(error) => {
                                coverage.push(json!({"group":chat.title,"complete":false,"error":error,"at":now}));
                                errors.push(format!("{}: {}", chat.title, error));
                            },
                        }
                    }
                    // An actual re-read replaces the snapshot: deletions cannot survive in today's analysis.
                    posts = if action == "today" { fresh } else { merge_posts(posts, fresh, &chosen) };
                    self.write(COVERAGE, &coverage)?;
                }
                self.write(SELECTED, &chosen)?;
                self.write(POSTS, &posts)?;
                self.prune_media(&posts);
            }
            Ok::<Value, String>(json!({"configured":true,"apiId":config.id,"authorized":status.authorized,"step":status.step,
                "name":status.name,"hint":status.hint,"chats":chats.iter().map(|c| json!({"id":c.id,"title":c.title,"kind":c.kind})).collect::<Vec<_>>(),"selected":chosen.iter().map(|c| c.id).collect::<Vec<_>>(),
                "selectedGroups":chosen.iter().map(|c| json!({"id":c.id,"title":c.title})).collect::<Vec<_>>(),
                "posts":posts,"errors":errors,"coverage":self.read::<Vec<Value>>(COVERAGE)}))
        }).await });
        // Preserve migrated/new auth keys even after a rejected code or transient network failure.
        if action == "signOut" {
            let removal = forget(SESSION);
            *held = None;
            self.write(SELECTED, &Vec::<ChatRef>::new())?;
            self.write(POSTS, &Vec::<Post>::new())?;
            let _ = std::fs::remove_dir_all(&self.media);
            removal?;
            result.map_err(|_| "No se pudo confirmar el cierre remoto. Revoca esta sesión en Telegram → Dispositivos.".to_string())??;
            return Ok(json!({"configured":true,"apiId":config.id,"authorized":false,"step":"idle","chats":[],"selected":[],"posts":[]}).to_string());
        } else if let Some(session) = connection.telegram.session() { keep(SESSION, &session)?; }
        result.map_err(|_| "Telegram tardó demasiado. Intenta de nuevo.".to_string())?.map(|v| v.to_string())
    }
    fn prune_media(&self, posts: &[Post]) {
        let keep: std::collections::HashSet<&str> = posts.iter().flat_map(|p| p.photos.iter().map(String::as_str)).collect();
        if let Ok(entries) = std::fs::read_dir(&self.media) {
            for entry in entries.flatten() {
                let name = entry.file_name();
                if !keep.contains(name.to_string_lossy().as_ref()) {
                    let _ = std::fs::remove_file(entry.path());
                }
            }
        }
    }
    /// Always refresh the chosen groups before a PARLEY turn; a stale cache is never substituted for a failed read.
    pub fn today_context(&self, now: i64) -> Result<(String, Vec<String>), String> {
        let raw = self.request_at("today", "", now)?;
        let state: Value = serde_json::from_str(&raw).map_err(|_| "Respuesta de Telegram no válida.")?;
        self.render_today(now, &state)
    }

    fn render_today(&self, now: i64, state: &Value) -> Result<(String, Vec<String>), String> {
        // Use this read's snapshot: a concurrent Settings selection must not substitute a different cache.
        let chosen: Vec<_> = state["selectedGroups"].as_array().into_iter().flatten().collect();
        let posts: Vec<Post> = serde_json::from_value(state["posts"].clone()).map_err(|_| "Mensajes de Telegram no válidos.")?;
        let (start, end) = crate::parley::day_window(now);
        let todays: Vec<_> = posts.iter().filter(|p| p.date >= start && p.date < end && p.date <= now && chosen.iter().any(|c| c["id"].as_i64() == Some(p.chat_id))).collect();
        let mut text = format!("Cuenta personal Telegram · lectura nueva · {} (America/Lima). Grupos seleccionados: {}. Mensajes de hoy: {}.\nCobertura por grupo: {}\nErrores: {}\n", crate::parley::lima_time(now), chosen.iter().filter_map(|c| c["title"].as_str()).collect::<Vec<_>>().join(", "), todays.len(), state["coverage"], state["errors"]);
        let mut files = Vec::new();
        let mut shown = 0;
        let photo_count: usize = todays.iter().map(|p| p.photos.len()).sum();
        for post in &todays {
            if text.len() > 100_000 { break; }
            text.push_str(&format!("\nGrupo: {} · Publicado: {} · Mensaje: {} · Autor: {}\n{}\nEnlaces (datos): {:?}\n", post.chat, crate::parley::lima_time(post.date), post.id, post.sender, post.text, post.links));
            if !post.media.is_empty() { text.push_str(&format!("Medio no descargado: {}. No inventes su contenido.\n", post.media)); }
            for name in &post.photos {
                if files.len() >= 10 { continue; }
                if std::path::Path::new(name).file_name().and_then(|n| n.to_str()) != Some(name) { continue; }
                let path = self.media.join(name);
                if path.canonicalize().ok().zip(self.media.canonicalize().ok()).is_some_and(|(p, root)| p.starts_with(root) && p.is_file()) {
                    text.push_str(&format!("Foto: {}\n", name));
                    files.push(path.to_string_lossy().into_owned());
                }
            }
            shown += 1;
        }
        text.push_str(&format!("\nMensajes incluidos: {shown}/{}. Fotos incluidas: {}/{photo_count} (máximo 10). Si faltan mensajes o fotos, la lectura es parcial: no concluyas que todos los picks terminaron.\n", todays.len(), files.len()));
        Ok((text, files))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn chat(id:i64) -> ChatRef { ChatRef { id, hash:0, title:format!("Grupo {id}"), kind:"group".into() } }
    fn post(chat_id:i64,id:i32) -> Post { Post { chat_id, chat:"grupo".into(), id, date:id as i64, text:format!("pick {id}"), sender:String::new(), links:vec![], photos:vec![], media:String::new(), album:None } }
    #[test] fn editing_the_id_can_keep_the_hash_without_exposing_it() {
        let current = serde_json::to_string(&Credentials { id: 12, hash: "a".repeat(32) }).unwrap();
        let next = credentials(&json!({"id":13,"hash":""}),Some(&current)).unwrap();
        assert_eq!(next.id,13);
        assert_eq!(next.hash,"a".repeat(32));
        assert!(credentials(&json!({"id":13,"hash":""}),None).is_err());
        assert!(credentials(&json!({"id":13,"hash":"mistyped"}),Some(&current)).is_err());
    }
    #[test] fn only_selected_available_groups_are_accepted() {
        assert!(selection(&[chat(1)], &[2]).is_err());
        assert!(selection(&[], &(0..11).collect::<Vec<_>>()).is_err());
        assert_eq!(selection(&[chat(1)], &[1,1]).unwrap().len(),1);
    }
    #[test] fn cache_replaces_edits_removes_unselected_and_is_bounded() {
        let mut edited=post(1,1); edited.text="editado".into();
        let posts=merge_posts(vec![post(1,1),post(2,2)],vec![edited,post(2,3)],&[chat(1)]);
        assert_eq!(posts.len(),1); assert_eq!(posts[0].text,"editado");
        assert_eq!(merge_posts(vec![],(0..300).map(|id|post(1,id)).collect(),&[chat(1)]).len(),200);
    }
    #[test] fn analysis_uses_only_todays_selected_snapshot_and_exposes_incomplete_reads() {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(Mutex::new(Store::open_in_memory().unwrap()));
        let account = Account::new(store, dir.path().into());
        let now = crate::parley::parse_time(&json!("2026-10-03T03:30:00Z")).unwrap();
        let start = crate::parley::day_window(now).0;
        let mut today = post(1,1); today.date = start; today.text = "PICK DE HOY".into(); today.photos = vec!["../../secret.jpg".into()];
        let mut yesterday = post(1,2); yesterday.date = start - 1; yesterday.text = "PICK ANTIGUO".into();
        let mut unselected = post(2,3); unselected.date = now; unselected.text = "OTRO GRUPO".into();
        let mut future = post(1,4); future.date = now + 1; future.text = "MENSAJE FUTURO".into();
        // Deliberately different cached data: analysis must use the response, not the mutable cache.
        account.write(POSTS, &vec![yesterday.clone()]).unwrap();
        let state = json!({"selectedGroups":[{"id":1,"title":"Grupo elegido"}], "posts":[today,yesterday,unselected,future],
            "errors":["Grupo 3: sin acceso"],"coverage":[{"group":"Grupo elegido","complete":false,"count":200}]});
        let (text, files) = account.render_today(now, &state).unwrap();
        assert!(text.contains("PICK DE HOY"));
        for excluded in ["PICK ANTIGUO", "OTRO GRUPO", "MENSAJE FUTURO"] { assert!(!text.contains(excluded)); }
        assert!(text.contains("\"complete\":false"));
        assert!(text.contains("Grupo 3: sin acceso"));
        assert!(text.contains("Fotos incluidas: 0/1"));
        assert!(files.is_empty(), "unsafe or missing media must never be attached");
    }
}
