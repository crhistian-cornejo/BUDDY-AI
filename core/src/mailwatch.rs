//! Niko's mail watch: Gmail over IMAP IDLE, so a charge or a payment is noticed seconds after its mail arrives,
//! with no model turn while nothing arrives.
//!
//! What it may do, and nothing else: one TLS connection to `imap.gmail.com:993` (certificates checked against the
//! bundled web roots), `LOGIN` with the user's Google app password (kept in the Keychain / Credential Manager, never
//! in a file or a log), `EXAMINE INBOX` (read-only: the server refuses changes), `IDLE`, and `UID FETCH` with
//! `BODY.PEEK` of the sender, the subject and the first bytes of the text of new messages. No attachments, no links
//! followed, nothing stored but the last UID. A mail's text is data: it goes to `filter` here and then to a model
//! turn without tools (`Niko::mail_arrived`).

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::store::Store;
use crate::{events::Event, events::EventBus, log};

pub const HOST: &str = "imap.gmail.com";
const PORT: u16 = 993;
/// The Gmail address being watched (empty: off).
pub const EMAIL_KEY: &str = "niko.mail.address";
/// The next UID to read: the watch starts at «now», never at the backlog.
const NEXT_UID_KEY: &str = "niko.mail.next_uid";
const MAILBOX_KEY: &str = "niko.mail.mailbox";
const KEYCHAIN_SERVICE: &str = "io.github.crhistian-cornejo.buddy";
const KEYCHAIN_ACCOUNT: &str = "niko-imap";
/// Gmail ends an IDLE after 29 minutes; a new one starts before that.
const IDLE_FOR: Duration = Duration::from_secs(20 * 60);
/// The socket wakes this often to see whether the watch was stopped.
const POLL: Duration = Duration::from_secs(30);
/// The first bytes of a message's text that are read (a receipt's amount and merchant are at the top).
const BODY_BYTES: usize = 16_000;
/// What a model sees of one mail, at most.
const BODY_CHARS: usize = 2_500;
/// More new mails than this at once (after a long sleep): only the latest are looked at.
const MAX_BATCH: usize = 25;

/// One new mail, already reduced to text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mail {
    /// Gmail's message id in hexadecimal (the one in `https://mail.google.com/…#all/<id>` and in `gmail:<id>`).
    pub id: String,
    /// Gmail's thread id in hexadecimal: what its web address opens.
    pub thread: String,
    pub from: String,
    pub subject: String,
    pub body: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct MailWatchStatus {
    /// The address being watched (empty: not set up).
    pub email: String,
    pub connected: bool,
    /// Why it is not connected, in the user's words (empty: fine or not set up).
    pub error: String,
}

/// What a batch of money mails is handed to (Niko).
pub type Handler = Box<dyn Fn(&[Mail]) + Send + Sync>;

pub struct MailWatch {
    store: Arc<Mutex<Store>>,
    bus: Arc<EventBus>,
    handler: Mutex<Option<Handler>>,
    state: Mutex<(bool, String)>,
    /// Bumped on every (re)start and stop: an older thread sees it and leaves.
    generation: AtomicU64,
    stopped: AtomicBool,
}

impl MailWatch {
    pub fn new(store: Arc<Mutex<Store>>, bus: Arc<EventBus>) -> Self {
        Self { store, bus, handler: Mutex::new(None), state: Mutex::new((false, String::new())), generation: AtomicU64::new(0), stopped: AtomicBool::new(false) }
    }

    pub fn set_handler(&self, handler: Handler) {
        *self.handler.lock().unwrap() = Some(handler);
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Store> {
        self.store.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn email(&self) -> String {
        self.lock().setting(EMAIL_KEY).ok().flatten().unwrap_or_default()
    }

    pub fn status(&self) -> MailWatchStatus {
        let (connected, error) = self.state.lock().unwrap().clone();
        MailWatchStatus { email: self.email(), connected, error }
    }

    fn set_state(&self, connected: bool, error: &str) {
        let changed = {
            let mut state = self.state.lock().unwrap();
            let next = (connected, error.to_string());
            let changed = *state != next;
            *state = next;
            changed
        };
        if changed {
            self.bus.publish(Event::NikoChanged);
        }
    }

    /// Checks the address and the app password against Gmail, saves them (the password only in the system's
    /// keychain) and starts watching.
    pub fn connect(self: &Arc<Self>, email: &str, password: &str) -> Result<(), String> {
        let email = email.trim().to_lowercase();
        // Google shows the app password in groups of four: the spaces are not part of it.
        let password: String = password.chars().filter(|c| !c.is_whitespace()).collect();
        if !valid_address(&email) {
            return Err("Escribe tu dirección de Gmail completa.".into());
        }
        if password.len() < 8 || !password.is_ascii() {
            return Err("Pega la contraseña de aplicación de 16 letras que te da Google (no tu contraseña normal).".into());
        }
        let mut imap = Imap::open()?;
        imap.login(&email, &password)?;
        let _ = imap.command("LOGOUT");
        keyring::Entry::new(KEYCHAIN_SERVICE, KEYCHAIN_ACCOUNT)
            .and_then(|e| e.set_password(&password))
            .map_err(|e| format!("No se pudo guardar la contraseña en el llavero: {e}"))?;
        {
            let store = self.lock();
            let _ = store.set_setting(EMAIL_KEY, &email);
            let _ = store.set_setting(NEXT_UID_KEY, "");
        }
        self.start();
        Ok(())
    }

    /// Stops watching and forgets the password.
    pub fn disconnect(&self) {
        self.generation.fetch_add(1, Ordering::SeqCst);
        if let Ok(entry) = keyring::Entry::new(KEYCHAIN_SERVICE, KEYCHAIN_ACCOUNT) {
            let _ = entry.delete_credential();
        }
        let _ = self.lock().set_setting(EMAIL_KEY, "");
        self.set_state(false, "");
    }

    pub fn shutdown(&self) {
        self.stopped.store(true, Ordering::SeqCst);
        self.generation.fetch_add(1, Ordering::SeqCst);
    }

    /// Starts the watch when an address is saved (nothing runs otherwise).
    pub fn start(self: &Arc<Self>) {
        let email = self.email();
        if email.is_empty() {
            return;
        }
        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        let watch = self.clone();
        let _ = std::thread::Builder::new().name("niko-mail".into()).spawn(move || watch.run(email, generation));
    }

    fn alive(&self, generation: u64) -> bool {
        !self.stopped.load(Ordering::SeqCst) && self.generation.load(Ordering::SeqCst) == generation
    }

    fn run(&self, email: String, generation: u64) {
        let mut failures = 0u32;
        while self.alive(generation) {
            let password = keyring::Entry::new(KEYCHAIN_SERVICE, KEYCHAIN_ACCOUNT).and_then(|e| e.get_password()).unwrap_or_default();
            if password.is_empty() {
                self.set_state(false, "Falta la contraseña de aplicación. Vuelve a conectar el correo.");
                return;
            }
            let started = Instant::now();
            let result = self.session(&email, &password, generation);
            drop(password);
            if !self.alive(generation) {
                return;
            }
            let error = result.err().unwrap_or_else(|| "Gmail cerró la conexión.".into());
            // A session that lasted is a hiccup of the network; one that never got going backs off.
            failures = if started.elapsed() > Duration::from_secs(120) { 1 } else { failures + 1 };
            log::line(format!("niko · correo: {error}"));
            if error.contains("contraseña") {
                // A rejected password does not get better by retrying: the user reconnects.
                self.set_state(false, &error);
                return;
            }
            self.set_state(false, &error);
            let wait = Duration::from_secs((15u64 << failures.min(6)).min(15 * 60));
            let until = Instant::now() + wait;
            while Instant::now() < until && self.alive(generation) {
                std::thread::sleep(Duration::from_secs(1));
            }
        }
    }

    fn session(&self, email: &str, password: &str, generation: u64) -> Result<(), String> {
        let mut imap = Imap::open()?;
        imap.login(email, password)?;
        // «Todos» rather than the inbox: a filter or a category that skips the inbox must not hide a charge.
        let mailbox = imap.all_mail().unwrap_or_else(|| "INBOX".into());
        let lines = imap.command(&format!("EXAMINE \"{}\"", mailbox.replace('\\', "\\\\").replace('"', "\\\"")))?;
        let server_next = lines.iter().find_map(|l| number_after(&l.text(), "UIDNEXT ")).unwrap_or(1);
        // UIDs belong to a mailbox: a saved one for another mailbox means nothing here.
        let same_mailbox = self.lock().setting(MAILBOX_KEY).ok().flatten().as_deref() == Some(mailbox.as_str());
        let _ = self.lock().set_setting(MAILBOX_KEY, &mailbox);
        let saved = self.lock().setting(NEXT_UID_KEY).ok().flatten().and_then(|v| v.parse::<u64>().ok()).filter(|_| same_mailbox);
        // First time (or after a reconnect of the account): start at what arrives from now on.
        let mut next = saved.filter(|n| *n <= server_next).unwrap_or(server_next);
        // Saved at once: what arrives while Buddy is closed is read when it opens again.
        let _ = self.lock().set_setting(NEXT_UID_KEY, &next.to_string());
        self.set_state(true, "");
        log::line(format!("niko · correo: vigilando «{mailbox}»"));
        while self.alive(generation) {
            let mut mails = imap.fetch_from(next)?;
            if let Some(highest) = mails.iter().map(|(uid, _)| *uid).max() {
                next = highest + 1;
                let _ = self.lock().set_setting(NEXT_UID_KEY, &next.to_string());
            }
            if mails.len() > MAX_BATCH {
                mails.drain(..mails.len() - MAX_BATCH);
            }
            let seen = mails.len();
            // BUDDY_DEBUG_MAIL: the log names each new mail and whether the filter let it through (never its text).
            if std::env::var_os("BUDDY_DEBUG_MAIL").is_some() {
                for (uid, m) in &mails {
                    log::line(format!("niko · correo [{uid}] {} «{}» de {}", if is_money(m) { "dinero" } else { "no" }, m.subject.chars().take(70).collect::<String>(), m.from.chars().take(50).collect::<String>()));
                }
            }
            let money: Vec<Mail> = mails.into_iter().map(|(_, m)| m).filter(is_money).collect();
            if seen > 0 {
                log::line(format!("niko · correo: {seen} correo(s) nuevo(s), {} con dinero", money.len()));
            }
            if !money.is_empty() {
                if let Some(handler) = self.handler.lock().unwrap().as_ref() {
                    handler(&money);
                }
            }
            imap.idle(|| self.alive(generation))?;
        }
        let _ = imap.command("LOGOUT");
        Ok(())
    }
}

fn valid_address(email: &str) -> bool {
    let Some((user, domain)) = email.split_once('@') else { return false };
    !user.is_empty()
        && domain.contains('.')
        && email.len() <= 120
        && email.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '@' | '.' | '_' | '-' | '+'))
}

fn number_after(text: &str, marker: &str) -> Option<u64> {
    let rest = &text[text.find(marker)? + marker.len()..];
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    digits.parse().ok()
}

// MARK: IMAP

/// One response line: its text pieces and the literals (`{n}` + n bytes) between them.
#[derive(Debug, Default)]
struct Line {
    pieces: Vec<String>,
    literals: Vec<Vec<u8>>,
}

impl Line {
    fn text(&self) -> String {
        self.pieces.join(" ")
    }
}

struct Imap {
    stream: BufReader<rustls::StreamOwned<rustls::ClientConnection, TcpStream>>,
    tag: u32,
    /// Bytes of a line whose end has not arrived yet (a read may time out in the middle).
    pending: Vec<u8>,
}

impl Imap {
    fn open() -> Result<Self, String> {
        let offline = || "No hay conexión con Gmail. Revisa tu internet.".to_string();
        let address = (HOST, PORT).to_socket_addrs().map_err(|_| offline())?.next().ok_or_else(offline)?;
        let tcp = TcpStream::connect_timeout(&address, Duration::from_secs(20)).map_err(|_| offline())?;
        let _ = tcp.set_read_timeout(Some(POLL));
        let _ = tcp.set_write_timeout(Some(Duration::from_secs(30)));
        let roots = rustls::RootCertStore { roots: webpki_roots::TLS_SERVER_ROOTS.to_vec() };
        let config = rustls::ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_safe_default_protocol_versions()
            .map_err(|e| format!("TLS: {e}"))?
            .with_root_certificates(roots)
            .with_no_client_auth();
        let name = rustls::pki_types::ServerName::try_from(HOST).map_err(|e| e.to_string())?;
        let tls = rustls::ClientConnection::new(Arc::new(config), name).map_err(|e| format!("TLS: {e}"))?;
        let mut imap = Self { stream: BufReader::new(rustls::StreamOwned::new(tls, tcp)), tag: 0, pending: Vec::new() };
        let greeting = imap.read_line(Duration::from_secs(30))?.ok_or("Gmail no respondió.")?;
        if !greeting.text().starts_with("* OK") {
            return Err("Gmail no aceptó la conexión.".into());
        }
        Ok(imap)
    }

    /// One raw line (without CRLF), or `None` when nothing came within `wait`.
    fn read_raw(&mut self, wait: Duration) -> Result<Option<Vec<u8>>, String> {
        let deadline = Instant::now() + wait;
        loop {
            match self.stream.read_until(b'\n', &mut self.pending) {
                Ok(0) => return Err("Gmail cerró la conexión.".into()),
                Ok(_) if self.pending.ends_with(b"\n") => {
                    let mut line = std::mem::take(&mut self.pending);
                    while matches!(line.last(), Some(b'\n' | b'\r')) {
                        line.pop();
                    }
                    return Ok(Some(line));
                }
                Ok(_) => {}
                Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {
                    if Instant::now() >= deadline {
                        return Ok(None);
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                Err(_) => return Err("Se cortó la conexión con Gmail.".into()),
            }
            if self.pending.len() > 4 * BODY_BYTES {
                return Err("Gmail envió una respuesta demasiado larga.".into());
            }
        }
    }

    /// A whole response line with its literals, or `None` when nothing came within `wait`.
    fn read_line(&mut self, wait: Duration) -> Result<Option<Line>, String> {
        let mut line = Line::default();
        let mut wait = wait;
        loop {
            let Some(raw) = self.read_raw(wait)? else {
                return if line.pieces.is_empty() { Ok(None) } else { Err("Gmail dejó una respuesta a medias.".into()) };
            };
            // Inside a line the rest must follow: give it time.
            wait = Duration::from_secs(60);
            let text = String::from_utf8_lossy(&raw).into_owned();
            let size = text.strip_suffix('}').and_then(|t| t.rsplit_once('{')).and_then(|(_, n)| n.parse::<usize>().ok());
            line.pieces.push(text);
            let Some(size) = size else { return Ok(Some(line)) };
            if size > 4 * BODY_BYTES {
                return Err("Gmail envió un bloque demasiado grande.".into());
            }
            let mut literal = vec![0u8; size];
            let mut filled = 0;
            let deadline = Instant::now() + Duration::from_secs(60);
            while filled < size {
                match std::io::Read::read(&mut self.stream, &mut literal[filled..]) {
                    Ok(0) => return Err("Gmail cerró la conexión.".into()),
                    Ok(n) => filled += n,
                    Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut | std::io::ErrorKind::Interrupted) => {
                        if Instant::now() >= deadline {
                            return Err("Gmail tardó demasiado en responder.".into());
                        }
                    }
                    Err(_) => return Err("Se cortó la conexión con Gmail.".into()),
                }
            }
            line.literals.push(literal);
        }
    }

    fn send(&mut self, text: &str) -> Result<(), String> {
        let stream = self.stream.get_mut();
        stream.write_all(text.as_bytes()).and_then(|_| stream.flush()).map_err(|_| "Se cortó la conexión con Gmail.".to_string())
    }

    /// Sends a command and returns the untagged lines of its answer; `Err` with the server's reason on NO / BAD.
    fn command(&mut self, command: &str) -> Result<Vec<Line>, String> {
        self.tag += 1;
        let tag = format!("B{}", self.tag);
        self.send(&format!("{tag} {command}\r\n"))?;
        let mut lines = Vec::new();
        loop {
            let line = self.read_line(Duration::from_secs(60))?.ok_or("Gmail tardó demasiado en responder.")?;
            let text = line.text();
            if let Some(rest) = text.strip_prefix(&format!("{tag} ")) {
                return if rest.starts_with("OK") { Ok(lines) } else { Err(rest.chars().take(160).collect()) };
            }
            lines.push(line);
        }
    }

    fn login(&mut self, email: &str, password: &str) -> Result<(), String> {
        let quote = |s: &str| format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""));
        self.command(&format!("LOGIN {} {}", quote(email), quote(password))).map(|_| ()).map_err(|reason| {
            let reason = reason.to_lowercase();
            if reason.contains("application-specific") || reason.contains("invalid credentials") || reason.contains("authenticationfailed") {
                "Gmail no aceptó esa contraseña. Usa una contraseña de aplicación (Cuenta de Google › Seguridad › Verificación en dos pasos › Contraseñas de aplicaciones).".into()
            } else if reason.contains("imap") && reason.contains("disabled") {
                "IMAP está desactivado en esta cuenta (Gmail › Configuración › Reenvío y correo POP/IMAP, o el administrador de tu empresa).".into()
            } else {
                "Gmail no permitió entrar con esa cuenta. Revisa la dirección y la contraseña de aplicación.".into()
            }
        })
    }

    /// Gmail's «all mail» mailbox, whatever it is called in the account's language (the one flagged `\All`).
    fn all_mail(&mut self) -> Option<String> {
        let lines = self.command("LIST \"\" \"*\"").ok()?;
        lines.iter().map(Line::text).find(|l| l.contains("\\All")).and_then(|l| {
            // «* LIST (\HasNoChildren \All) "/" "[Gmail]/Todos"»
            let name = l.rsplit_once(" \"/\" ")?.1.trim();
            Some(name.trim_matches('"').replace("\\\"", "\"").replace("\\\\", "\\"))
        })
    }

    /// New messages from `next` on: UID and what `Mail` needs. Nothing is marked as read (`BODY.PEEK`).
    fn fetch_from(&mut self, next: u64) -> Result<Vec<(u64, Mail)>, String> {
        let lines = self.command(&format!(
            "UID FETCH {next}:* (UID X-GM-MSGID X-GM-THRID BODY.PEEK[HEADER.FIELDS (FROM SUBJECT CONTENT-TYPE CONTENT-TRANSFER-ENCODING)] BODY.PEEK[TEXT]<0.{BODY_BYTES}>)"
        ))?;
        let mut mails = Vec::new();
        for line in lines {
            let text = line.text();
            if !text.contains(" FETCH ") {
                continue;
            }
            // `n:*` always answers with the last message, even an old one.
            let (Some(uid), Some(gmail)) = (number_after(&text, "UID "), number_after(&text, "X-GM-MSGID ")) else { continue };
            if uid < next {
                continue;
            }
            let mut header: &[u8] = &[];
            let mut body: &[u8] = &[];
            // Each literal follows the piece that names it: «… BODY[HEADER.FIELDS (…)] {n}» or «… BODY[TEXT]<0> {n}».
            for (piece, literal) in line.pieces.iter().zip(&line.literals) {
                if piece.rfind("BODY[TEXT]") > piece.rfind("HEADER.FIELDS") {
                    body = literal;
                } else {
                    header = literal;
                }
            }
            let thread = number_after(&text, "X-GM-THRID ").unwrap_or(gmail);
            mails.push((uid, read_mail(format!("{gmail:x}"), format!("{thread:x}"), header, body)));
        }
        mails.sort_by_key(|(uid, _)| *uid);
        Ok(mails)
    }

    /// Waits in IDLE until the server says something arrived, the watch stops, or `IDLE_FOR` passes.
    fn idle(&mut self, alive: impl Fn() -> bool) -> Result<(), String> {
        self.tag += 1;
        let tag = format!("B{}", self.tag);
        self.send(&format!("{tag} IDLE\r\n"))?;
        let started = Instant::now();
        let mut done = false;
        loop {
            match self.read_line(POLL)? {
                Some(line) => {
                    let text = line.text();
                    if let Some(rest) = text.strip_prefix(&format!("{tag} ")) {
                        return if rest.starts_with("OK") { Ok(()) } else { Err("Gmail rechazó la espera de correo nuevo.".into()) };
                    }
                    if text.ends_with(" EXISTS") && !done {
                        self.send("DONE\r\n")?;
                        done = true;
                    }
                }
                None if done => return Err("Gmail tardó demasiado en responder.".into()),
                None => {}
            }
            if !done && (!alive() || started.elapsed() > IDLE_FOR) {
                self.send("DONE\r\n")?;
                done = true;
            }
        }
    }
}

// MARK: Reading a mail

fn read_mail(id: String, thread: String, header: &[u8], body: &[u8]) -> Mail {
    let header = String::from_utf8_lossy(header).replace("\r\n ", " ").replace("\r\n\t", " ");
    let field = |name: &str| {
        header
            .lines()
            .find(|l| l.to_ascii_lowercase().starts_with(&format!("{name}:")))
            .map(|l| l[name.len() + 1..].trim().to_string())
            .unwrap_or_default()
    };
    let text = part_text(&field("content-type"), &field("content-transfer-encoding"), body, 0);
    Mail {
        id,
        thread,
        from: clean(&decode_words(&field("from")), 160),
        subject: clean(&decode_words(&field("subject")), 240),
        body: clean(&text, BODY_CHARS),
    }
}

/// The readable text of a MIME part: `text/plain` first, else `text/html` without its tags.
fn part_text(content_type: &str, encoding: &str, body: &[u8], depth: u8) -> String {
    let kind = content_type.to_ascii_lowercase();
    if kind.starts_with("multipart/") && depth < 4 {
        let Some(boundary) = parameter(content_type, "boundary") else { return String::new() };
        let marker = format!("--{boundary}");
        let raw = String::from_utf8_lossy(body);
        let mut html = String::new();
        for part in raw.split(marker.as_str()).skip(1) {
            let part = part.trim_start_matches(['\r', '\n']);
            let (head, content) = part.split_once("\r\n\r\n").or_else(|| part.split_once("\n\n")).unwrap_or((part, ""));
            let head = head.replace("\r\n ", " ").replace("\r\n\t", " ");
            let of = |name: &str| {
                head.lines().find(|l| l.to_ascii_lowercase().starts_with(&format!("{name}:"))).map(|l| l[name.len() + 1..].trim().to_string()).unwrap_or_default()
            };
            let kind = of("content-type");
            let text = part_text(&kind, &of("content-transfer-encoding"), content.as_bytes(), depth + 1);
            if text.trim().is_empty() {
                continue;
            }
            if kind.to_ascii_lowercase().starts_with("text/html") {
                if html.is_empty() {
                    html = text;
                }
            } else {
                return text;
            }
        }
        return html;
    }
    if !(kind.is_empty() || kind.starts_with("text/")) {
        return String::new();
    }
    let bytes = match encoding.trim().to_ascii_lowercase().as_str() {
        "base64" => {
            use base64::Engine;
            let compact: Vec<u8> = body.iter().copied().filter(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'/')).collect();
            // The fetch may cut the text in the middle of a group: one loose character cannot be decoded.
            let whole = &compact[..compact.len() - usize::from(compact.len() % 4 == 1)];
            base64::engine::general_purpose::STANDARD_NO_PAD.decode(whole).unwrap_or_default()
        }
        "quoted-printable" => quoted_printable(body),
        _ => body.to_vec(),
    };
    let charset = parameter(content_type, "charset").unwrap_or_default().to_ascii_lowercase();
    let text = if charset.starts_with("iso-8859") || charset.starts_with("windows-125") || charset == "latin1" {
        bytes.iter().map(|b| *b as char).collect()
    } else {
        String::from_utf8_lossy(&bytes).into_owned()
    };
    if kind.starts_with("text/html") || text.contains("<html") || text.contains("<table") || text.contains("<div") { strip_html(&text) } else { text }
}

fn parameter(header: &str, name: &str) -> Option<String> {
    let lower = header.to_ascii_lowercase();
    let at = lower.find(&format!("{name}="))? + name.len() + 1;
    let rest = &header[at..];
    Some(match rest.strip_prefix('"') {
        Some(quoted) => quoted.split('"').next().unwrap_or_default().to_string(),
        None => rest.split([';', ' ', '\r', '\n']).next().unwrap_or_default().to_string(),
    })
}

fn quoted_printable(body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(body.len());
    let mut i = 0;
    while i < body.len() {
        if body[i] == b'=' {
            if body[i + 1..].starts_with(b"\r\n") {
                i += 3;
                continue;
            }
            if body[i + 1..].starts_with(b"\n") {
                i += 2;
                continue;
            }
            let hex = body.get(i + 1..i + 3).and_then(|h| std::str::from_utf8(h).ok()).and_then(|h| u8::from_str_radix(h, 16).ok());
            if let Some(byte) = hex {
                out.push(byte);
                i += 3;
                continue;
            }
        }
        out.push(body[i]);
        i += 1;
    }
    out
}

/// RFC 2047 words in a header («=?UTF-8?B?…?=», «=?iso-8859-1?Q?…?=»).
fn decode_words(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find("=?") {
        let after = &rest[start + 2..];
        let mut parts = after.splitn(3, '?');
        let (Some(charset), Some(kind), Some(tail)) = (parts.next(), parts.next(), parts.next()) else { break };
        let Some(end) = tail.find("?=") else { break };
        let bytes = if kind.eq_ignore_ascii_case("b") {
            use base64::Engine;
            base64::engine::general_purpose::STANDARD_NO_PAD.decode(tail[..end].trim_end_matches('=')).unwrap_or_default()
        } else {
            quoted_printable(tail[..end].replace('_', " ").as_bytes())
        };
        let word: String = if charset.to_ascii_lowercase().starts_with("utf") { String::from_utf8_lossy(&bytes).into_owned() } else { bytes.iter().map(|b| *b as char).collect() };
        // White space between two encoded words is not part of the text.
        let before = &rest[..start];
        if !(before.trim().is_empty() && !out.is_empty()) {
            out.push_str(before);
        }
        out.push_str(&word);
        rest = &tail[end + 2..];
    }
    out.push_str(rest);
    out
}

fn strip_html(html: &str) -> String {
    let mut text = String::with_capacity(html.len() / 2);
    let lower = html.to_ascii_lowercase();
    let mut i = 0;
    let bytes = html.as_bytes();
    while i < bytes.len() {
        if bytes[i] == b'<' {
            // Styles and scripts go whole; any other tag becomes a space.
            let skip = ["style", "script", "head", "title"].iter().find(|t| lower[i + 1..].starts_with(*t));
            let end = match skip {
                Some(tag) => lower[i..].find(&format!("</{tag}")).map(|e| i + e).and_then(|e| lower[e..].find('>').map(|g| e + g)),
                None => lower[i..].find('>').map(|g| i + g),
            };
            match end {
                Some(end) => {
                    text.push(' ');
                    i = end + 1;
                }
                None => break,
            }
        } else {
            let ch = html[i..].chars().next().unwrap_or(' ');
            text.push(ch);
            i += ch.len_utf8();
        }
    }
    for (entity, plain) in [("&nbsp;", " "), ("&amp;", "&"), ("&lt;", "<"), ("&gt;", ">"), ("&quot;", "\""), ("&#39;", "'"), ("&aacute;", "á"), ("&eacute;", "é"), ("&iacute;", "í"), ("&oacute;", "ó"), ("&uacute;", "ú"), ("&ntilde;", "ñ"), ("&#36;", "$")] {
        text = text.replace(entity, plain);
    }
    text
}

/// One line of spaces between words, control characters out, at most `max` characters.
fn clean(text: &str, max: usize) -> String {
    let mut out = String::new();
    for word in text.split(|c: char| c.is_whitespace() || c.is_control() || c == '\u{200c}' || c == '\u{feff}' || c == '\u{34f}').filter(|w| !w.is_empty()) {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(word);
        if out.chars().count() >= max {
            break;
        }
    }
    out.chars().take(max).collect()
}

// MARK: Filter

/// Senders whose mail is about money more often than not.
const SENDERS: &[&str] = &[
    "bcp.com.pe", "viabcp", "yape", "interbank", "bbva", "scotiabank", "plin", "banbif", "pichincha", "falabella", "ripley", "oh.com.pe", "izipay", "niubiz",
    "culqi", "mercadopago", "paypal", "stripe", "apple.com", "google.com", "amazon", "netflix", "spotify", "disney", "anthropic", "openai", "notion", "perplexity",
    "cabify", "uber", "rappi", "pedidosya", "didi", "sunat",
];
/// Words of a charge, a payment or a subscription (already folded: no accents, lowercase).
const WORDS: &[&str] = &[
    "cobro", "cargo", "consumo", "compra", "pago", "pagaste", "abono", "transferencia", "transferiste", "deposito", "yapeo", "yapeaste", "plineaste", "recibo",
    "boleta", "factura", "comprobante", "constancia", "operacion", "suscripcion", "renovacion", "renovo", "cancelacion", "reembolso", "devolucion", "debito",
    "receipt", "invoice", "payment", "charged", "subscription", "refund", "order", "purchase",
];

/// Whether a mail looks like a money movement: an amount in it, and a known sender or a money word. Cheap and
/// generous on purpose: the model turn after it decides; this only keeps newsletters and chats away from it.
pub fn is_money(mail: &Mail) -> bool {
    let from = crate::store::fold(&mail.from);
    let text = crate::store::fold(&format!("{} {}", mail.subject, mail.body));
    let known = SENDERS.iter().any(|s| from.contains(s));
    let worded = WORDS.iter().any(|w| text.contains(w));
    has_amount(&text) && (known || worded)
}

/// «S/ 45.90», «S/.45», «US$ 12», «$ 9.99», «PEN 30», «USD 110», «30 soles».
fn has_amount(text: &str) -> bool {
    let bytes = text.as_bytes();
    let digit_after = |at: usize| bytes[at..].iter().take(4).any(|b| b.is_ascii_digit());
    for marker in ["s/", "us$", "$", "pen ", "usd ", "s/."] {
        let mut from = 0;
        while let Some(found) = text[from..].find(marker) {
            let end = from + found + marker.len();
            if digit_after(end.min(bytes.len())) {
                return true;
            }
            from = end;
        }
    }
    ["soles", " usd", " pen", "dolares"].iter().any(|unit| {
        text.match_indices(unit).any(|(at, _)| text[..at].trim_end().chars().last().is_some_and(|c| c.is_ascii_digit()))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mail(from: &str, subject: &str, body: &str) -> Mail {
        Mail { id: "1a".into(), thread: "1a".into(), from: from.into(), subject: subject.into(), body: body.into() }
    }

    #[test]
    fn money_mails_pass_and_the_rest_do_not() {
        for m in [
            mail("BCP <notificaciones@notificacionesbcp.com.pe>", "Realizaste un consumo con tu Tarjeta", "Monto: S/ 186.50 en MFA877 Centro Cívico"),
            mail("Anthropic <invoice+statements@mail.anthropic.com>", "Your receipt from Anthropic", "Amount paid $110.00"),
            mail("Tienda Nueva <hola@tiendanueva.pe>", "Confirmación de tu compra", "Total: S/.59,90 pagado con Visa"),
            mail("Yape <yape@yape.pe>", "Yapeaste", "Enviaste S/ 20 a Juan"),
            mail("Gimnasio <no-reply@gym.com>", "Renovación de tu suscripción", "Se cobraron 120 soles a tu tarjeta"),
        ] {
            assert!(is_money(&m), "{}", m.subject);
        }
        for m in [
            mail("Juan <juan@gmail.com>", "Almuerzo mañana", "¿Vamos a las 13?"),
            mail("BCP <promos@bcp.com.pe>", "Conoce tu nueva app", "Descárgala hoy"),
            mail("Newsletter <news@medium.com>", "5 ideas para tu semana", "Lee sobre pagos digitales en 2026"),
            mail("Equipo <rrhh@empresa.com>", "Orden del día", "Reunión el 30 a las 10"),
        ] {
            assert!(!is_money(&m), "{}", m.subject);
        }
    }

    #[test]
    fn a_mail_is_reduced_to_its_text() {
        let header = b"From: =?UTF-8?B?QkNQIE5vdGlmaWNhY2nDs24=?= <a@bcp.com.pe>\r\nSubject: =?iso-8859-1?Q?Operaci=F3n_realizada?=\r\nContent-Type: multipart/alternative;\r\n boundary=\"xyz\"\r\n\r\n";
        let body = b"--xyz\r\nContent-Type: text/html; charset=UTF-8\r\nContent-Transfer-Encoding: quoted-printable\r\n\r\n<html><style>p{color:red}</style><p>Monto: <b>S/&nbsp;45.90</b></p>=\r\n<p>Comercio: Caf=C3=A9 Lima</p></html>\r\n--xyz--\r\n";
        let m = read_mail("abc".into(), "abc".into(), header, body);
        assert_eq!(m.from, "BCP Notificación <a@bcp.com.pe>");
        assert_eq!(m.subject, "Operación realizada");
        assert_eq!(m.body, "Monto: S/ 45.90 Comercio: Café Lima");

        let plain = read_mail("abc".into(), "abc".into(), b"From: a@b.pe\r\nSubject: Hola\r\nContent-Transfer-Encoding: base64\r\n\r\n", b"UGFnYXN0ZSBTLyAxMCBlbiBib2RlZ2E=\r\n");
        assert_eq!(plain.body, "Pagaste S/ 10 en bodega");
    }

    /// Talks to Gmail for real (`cargo test -- --ignored live_`): TLS, the greeting, and a login Gmail refuses.
    #[test]
    #[ignore]
    fn live_gmail_refuses_a_wrong_app_password() {
        let mut imap = Imap::open().expect("TLS to imap.gmail.com");
        let error = imap.login("buddy.prueba.inexistente@gmail.com", "abcdabcdabcdabcd").unwrap_err();
        assert!(error.contains("contraseña"), "{error}");
    }

    #[test]
    fn addresses_and_numbers() {
        assert!(valid_address("ana.perez+buddy@gmail.com"));
        for bad in ["", "ana", "ana@gmail", "ana perez@gmail.com", "a\"b@gmail.com"] {
            assert!(!valid_address(bad), "{bad}");
        }
        assert_eq!(number_after("* OK [UIDNEXT 4312] Predicted next UID", "UIDNEXT "), Some(4312));
        assert_eq!(number_after("* 3 FETCH (X-GM-MSGID 1278455344230334865 UID 44", "X-GM-MSGID "), Some(1278455344230334865));
    }
}
