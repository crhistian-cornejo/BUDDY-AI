//! The desktop's connection to the relay: one outgoing WebSocket, kept while a phone is paired (or a pairing code
//! is on screen). It carries `Session`'s frames and nothing else. Blocked on the socket and on the core's events it
//! costs no CPU; a lost connection is retried after 2 s, 4 s, 8 s… up to 5 min.

use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use buddy_remote::pairing::Offer;
use buddy_remote::wire::Envelope;
use futures_util::{SinkExt, StreamExt};
use tokio::sync::mpsc::UnboundedReceiver;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::{Connector, MaybeTlsStream, WebSocketStream};

use super::session::{Clock, Out, Session};
use crate::events::Event;

const MAX_BACKOFF: Duration = Duration::from_secs(300);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);
/// A ping this often shows a dead connection (a router that forgot it) without waiting for the next event.
const PING_EVERY: Duration = Duration::from_secs(240);

pub type Socket = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

pub enum Command {
    Event(Event),
    /// A pairing code went on screen.
    Pair(Offer),
    /// «Aprobar permisos desde el iPhone» changed.
    Approvals(bool),
    Stop,
}

pub struct Config {
    pub relay: String,
    pub room: String,
    pub room_key: String,
}

/// Where the link says what happens.
pub trait Report: Send + Sync {
    /// The socket to the relay opened or closed (`error`: why the last try failed, in words for the user).
    fn connected(&self, on: bool, error: &str);
    /// Everything of the session that is not a frame to send.
    fn out(&self, out: Out);
}

/// The socket address for a relay: `https://…` becomes `wss://…`. Plain `http://` is taken only for this machine
/// (a relay under test), never for the network.
pub fn ws_url(relay: &str, room: &str, role: &str) -> Result<String, String> {
    let relay = relay.trim().trim_end_matches('/');
    let base = if let Some(rest) = relay.strip_prefix("https://") {
        format!("wss://{rest}")
    } else if let Some(rest) = relay.strip_prefix("http://").filter(|r| is_local(r)) {
        format!("ws://{rest}")
    } else {
        return Err("La dirección del relé debe empezar por https://".into());
    };
    Ok(format!("{base}/rooms/{room}/ws?role={role}"))
}

fn is_local(host_and_path: &str) -> bool {
    let host = host_and_path.split(['/', ':']).next().unwrap_or("");
    host == "127.0.0.1" || host == "localhost"
}

/// How long to wait before try number `failures + 1`.
pub fn backoff(failures: u32) -> Duration {
    Duration::from_secs(2u64.saturating_pow(failures.clamp(1, 9))).min(MAX_BACKOFF)
}

/// Runs the link until `Command::Stop` (or its sender is gone). Call it on its own thread.
pub fn run(config: Config, session: Session, rx: UnboundedReceiver<Command>, report: Arc<dyn Report>) {
    match tokio::runtime::Builder::new_current_thread().enable_all().build() {
        Ok(runtime) => runtime.block_on(serve(config, session, rx, report)),
        Err(e) => report.connected(false, &format!("No se pudo iniciar la conexión: {e}")),
    }
}

async fn serve(config: Config, mut session: Session, mut rx: UnboundedReceiver<Command>, report: Arc<dyn Report>) {
    let started = Instant::now();
    let clock = move || Clock {
        unix: SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0),
        ms: started.elapsed().as_millis() as u64,
    };
    let mut failures = 0u32;
    loop {
        match connect(&config).await {
            Ok(mut socket) => {
                failures = 0;
                report.connected(true, "");
                let stopped = pump(&mut socket, &mut session, &mut rx, &report, &clock).await;
                // The phone's channel does not outlive the socket it came through.
                for out in session.on_frame(&Envelope::Peer { on: false }.to_text(), clock()) {
                    report.out(out);
                }
                report.connected(false, "");
                if stopped {
                    return;
                }
            }
            Err(error) => report.connected(false, &error),
        }
        failures += 1;
        // Wait, still listening: a stop ends at once; events keep the session's own state right (no socket to send on).
        let wait = tokio::time::sleep(backoff(failures));
        tokio::pin!(wait);
        loop {
            tokio::select! {
                _ = &mut wait => break,
                command = rx.recv() => match command {
                    None | Some(Command::Stop) => return,
                    Some(Command::Pair(offer)) => session.start_pairing(offer),
                    Some(Command::Approvals(on)) => session.opts.approvals = on,
                    Some(Command::Event(event)) => drop(session.on_event(&event, clock())),
                },
            }
        }
    }
}

/// Carries frames both ways until the socket closes (false) or a stop arrives (true).
async fn pump(socket: &mut Socket, session: &mut Session, rx: &mut UnboundedReceiver<Command>, report: &Arc<dyn Report>, clock: &impl Fn() -> Clock) -> bool {
    let mut ping = tokio::time::interval_at(tokio::time::Instant::now() + PING_EVERY, PING_EVERY);
    loop {
        // Answer text waiting for its turn leaves when its time comes.
        let flush_in = session.next_flush_ms().map(|at| Duration::from_millis(at.saturating_sub(clock().ms)));
        let outs = tokio::select! {
            frame = socket.next() => match frame {
                Some(Ok(Message::Text(text))) => session.on_frame(text.as_str(), clock()),
                Some(Ok(Message::Close(_))) | Some(Err(_)) | None => return false,
                Some(Ok(_)) => continue,
            },
            command = rx.recv() => match command {
                None | Some(Command::Stop) => {
                    let _ = socket.close(None).await;
                    return true;
                }
                Some(Command::Pair(offer)) => {
                    session.start_pairing(offer);
                    continue;
                }
                Some(Command::Approvals(on)) => {
                    session.opts.approvals = on;
                    continue;
                }
                Some(Command::Event(event)) => session.on_event(&event, clock()),
            },
            _ = tokio::time::sleep(flush_in.unwrap_or_default()), if flush_in.is_some() => session.flush(clock()),
            _ = ping.tick() => {
                if socket.send(Message::Ping(Default::default())).await.is_err() {
                    return false;
                }
                continue;
            }
        };
        for out in outs {
            match out {
                Out::Send(text) => {
                    if socket.send(Message::text(text)).await.is_err() {
                        return false;
                    }
                }
                other => report.out(other),
            }
        }
    }
}

async fn connect(config: &Config) -> Result<Socket, String> {
    open(&config.relay, &config.room, &config.room_key, "desktop").await
}

/// Opens a room's socket at the relay as `role` (`desktop`; `phone` for the console client that stands in for one).
pub async fn open(relay: &str, room: &str, room_key: &str, role: &str) -> Result<Socket, String> {
    let url = ws_url(relay, room, role)?;
    let mut request = url.as_str().into_client_request().map_err(|_| "La dirección del relé no es válida.".to_string())?;
    let bearer = format!("Bearer {room_key}").parse().map_err(|_| "La llave de la sala no es válida.".to_string())?;
    request.headers_mut().insert("Authorization", bearer);
    let connector = if url.starts_with("wss://") {
        let roots = rustls::RootCertStore { roots: webpki_roots::TLS_SERVER_ROOTS.to_vec() };
        let tls = rustls::ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_safe_default_protocol_versions()
            .map_err(|e| format!("TLS: {e}"))?
            .with_root_certificates(roots)
            .with_no_client_auth();
        Connector::Rustls(Arc::new(tls))
    } else {
        Connector::Plain
    };
    let connecting = tokio_tungstenite::connect_async_tls_with_config(request, None, false, Some(connector));
    match tokio::time::timeout(CONNECT_TIMEOUT, connecting).await {
        Ok(Ok((socket, _))) => Ok(socket),
        Ok(Err(tokio_tungstenite::tungstenite::Error::Http(response))) => Err(match response.status().as_u16() {
            401 => "El relé no reconoce la llave de esta sala.".into(),
            404 => "El relé ya no tiene esta sala: empareja de nuevo.".into(),
            code => format!("El relé respondió {code}."),
        }),
        Ok(Err(_)) => Err("No se pudo conectar con el relé.".into()),
        Err(_) => Err("El relé no respondió a tiempo.".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_relay_address_becomes_its_socket_address() {
        assert_eq!(ws_url("https://buddy-relay.example.workers.dev/", "r1", "desktop").unwrap(), "wss://buddy-relay.example.workers.dev/rooms/r1/ws?role=desktop");
        assert_eq!(ws_url("http://127.0.0.1:8787", "r1", "desktop").unwrap(), "ws://127.0.0.1:8787/rooms/r1/ws?role=desktop");
        assert_eq!(ws_url("http://localhost:8787", "r1", "desktop").unwrap(), "ws://localhost:8787/rooms/r1/ws?role=desktop");
        // Never in the clear over the network.
        assert!(ws_url("http://buddy-relay.example.workers.dev", "r1", "desktop").is_err());
        assert!(ws_url("http://127.0.0.1.evil.example", "r1", "desktop").is_err());
        assert!(ws_url("ws://x", "r1", "desktop").is_err());
        assert!(ws_url("", "r1", "desktop").is_err());
    }

    #[test]
    fn retries_wait_longer_each_time_up_to_five_minutes() {
        assert_eq!([1, 2, 3, 4].map(|n| backoff(n).as_secs()), [2, 4, 8, 16]);
        assert_eq!(backoff(8).as_secs(), 256);
        assert_eq!(backoff(9).as_secs(), 300);
        assert_eq!(backoff(40).as_secs(), 300);
        assert_eq!(backoff(0).as_secs(), 2);
    }
}
