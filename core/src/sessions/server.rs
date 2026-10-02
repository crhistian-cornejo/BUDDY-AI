// Ported from MIKA (MIT, revision d050bc5): apps/macos/Sources/Agents/ClaudeCode/HookServer.swift and
// apps/windows/src-tauri/src/agents/claude_code/pipe.rs
//! The local server buddy-hook talks to: a Unix domain socket on Mac (and other Unix), a named pipe on Windows,
//! behind one interface (`spawn` + `Sink`). Plain std threads: one accept loop, one short-lived thread per
//! connection.
//!
//! Protocol: the relay writes one JSON line. Every event but `PermissionRequest` is fire-and-forget: we read it,
//! hand it to the sink and close. `PermissionRequest` keeps its connection open until the sink decides; we then
//! write one line, `allow` or `deny`, and close. No decision means we close without writing, and the agent asks in
//! its own terminal exactly as if Buddy were not running. Turning the word into the agents' JSON is the relay's job,
//! so the wire format they expect lives in exactly one place.
//!
//! Hardening (Unix): the data folder is 0700 and the socket 0600 (umask 077 around bind); a client must run as the
//! same user; a request is capped at 1 MB and must arrive within 5 s; at most 16 connections are served at once.
//! Hardening (Windows): the pipe name carries the user's SID, its access list names that user and nobody else, the
//! first instance must be ours (`FILE_FLAG_FIRST_PIPE_INSTANCE`), remote clients are refused, and the client process
//! must run as the same user. Same caps.

use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::log;

/// Largest request accepted from the relay.
const MAX_REQUEST_BYTES: usize = 1 << 20;
/// A client must send its whole request within this long.
const READ_TIMEOUT: Duration = Duration::from_secs(5);
/// Connections served at the same time; extra ones are closed right away and the relay exits cleanly.
const MAX_CONNECTIONS: usize = 16;

/// Where the hub's decisions come from.
pub(crate) trait Sink: Send + Sync + 'static {
    /// A fire-and-forget hook event (a JSON object, as the relay sent it).
    fn event(&self, payload: Value);
    /// A `PermissionRequest`: blocks until there is a decision (`"allow"` / `"deny"`) or none. `closed` says whether
    /// the relay hung up meanwhile (the user answered in the terminal, the agent was stopped).
    fn permission(&self, payload: Value, closed: &dyn Fn() -> bool) -> Option<&'static str>;
}

/// Where to listen.
#[derive(Debug, Clone)]
pub(crate) enum Endpoint {
    /// A Unix domain socket at this path.
    #[cfg(unix)]
    Socket(std::path::PathBuf),
    /// A named pipe with this full name (`\\.\pipe\…`).
    #[cfg(windows)]
    Pipe(String),
}

/// One accepted connection, whatever the platform.
trait Conn {
    /// Reads up to the first newline (exclusive) within `READ_TIMEOUT`. `None` on timeout, error, an empty or an
    /// oversized request.
    fn read_request(&mut self) -> Option<Vec<u8>>;
    fn send_line(&mut self, text: &str);
    /// True once the client has gone away.
    fn peer_closed(&self) -> bool;
}

/// Counts a served connection; releases its slot when dropped.
struct Slot(Arc<AtomicUsize>);

impl Slot {
    fn take(active: &Arc<AtomicUsize>) -> Option<Slot> {
        if active.fetch_add(1, Ordering::AcqRel) >= MAX_CONNECTIONS {
            active.fetch_sub(1, Ordering::AcqRel);
            None
        } else {
            Some(Slot(active.clone()))
        }
    }
}

impl Drop for Slot {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

/// Starts listening on `endpoint` and serves it from a background thread for the life of the process. Returns once
/// the endpoint exists (so a relay started right after finds it), or the error that prevented it.
pub(crate) fn spawn(endpoint: Endpoint, sink: Arc<dyn Sink>) -> io::Result<()> {
    match endpoint {
        #[cfg(unix)]
        Endpoint::Socket(path) => unix::spawn(path, sink),
        #[cfg(windows)]
        Endpoint::Pipe(name) => windows_pipe::spawn(name, sink),
    }
}

/// Serves one connection from start to end.
fn handle(conn: &mut dyn Conn, sink: &dyn Sink) {
    // Oversized, timed out or not JSON: no decision, the relay prints nothing.
    let Some(line) = conn.read_request() else { return };
    let Ok(payload) = serde_json::from_slice::<Value>(&line) else { return };
    if !payload.is_object() {
        return;
    }
    if payload.get("hook_event_name").and_then(Value::as_str) != Some("PermissionRequest") {
        sink.event(payload);
        return;
    }
    let decision = {
        let conn: &dyn Conn = conn;
        sink.permission(payload, &|| conn.peer_closed())
    };
    if let Some(word) = decision {
        conn.send_line(word);
    }
}

/// Collects one request line from `read`, which returns the bytes read (0 = end of stream), honouring the size cap
/// and the overall deadline.
fn collect_line(mut read: impl FnMut(&mut [u8]) -> io::Result<usize>) -> Option<Vec<u8>> {
    let deadline = Instant::now() + READ_TIMEOUT;
    let mut raw = Vec::new();
    let mut buf = [0u8; 4096];
    loop {
        if Instant::now() >= deadline {
            return None;
        }
        let n = match read(&mut buf) {
            Ok(n) => n,
            // A read timeout (Unix) or "nothing yet" (Windows): try again until the deadline.
            Err(e) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut | io::ErrorKind::Interrupted) => {
                continue;
            }
            Err(_) => return None,
        };
        if n == 0 {
            break; // the peer closed: use what we have
        }
        if let Some(newline) = buf[..n].iter().position(|b| *b == b'\n') {
            raw.extend_from_slice(&buf[..newline]);
            break;
        }
        raw.extend_from_slice(&buf[..n]);
        if raw.len() > MAX_REQUEST_BYTES {
            return None;
        }
    }
    (!raw.is_empty() && raw.len() <= MAX_REQUEST_BYTES).then_some(raw)
}

#[cfg(unix)]
mod unix {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::io::{AsRawFd, RawFd};
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::path::{Path, PathBuf};

    struct SocketConn(UnixStream);

    impl Conn for SocketConn {
        fn read_request(&mut self) -> Option<Vec<u8>> {
            use std::io::Read;
            let stream = &mut self.0;
            collect_line(|buf| stream.read(buf))
        }

        fn send_line(&mut self, text: &str) {
            let bytes = format!("{text}\n").into_bytes();
            let fd = self.0.as_raw_fd();
            let mut sent = 0;
            while sent < bytes.len() {
                // SAFETY: the slice outlives the call; no SIGPIPE (MSG_NOSIGNAL / SO_NOSIGPIPE), which matters because
                // this runs inside the Mac app, where SIGPIPE is not ignored.
                let n = unsafe { libc::send(fd, bytes[sent..].as_ptr().cast(), bytes.len() - sent, SEND_FLAGS) };
                if n <= 0 {
                    break;
                }
                sent += n as usize;
            }
        }

        fn peer_closed(&self) -> bool {
            let fd = self.0.as_raw_fd();
            let mut poll = libc::pollfd { fd, events: libc::POLLIN, revents: 0 };
            // SAFETY: one live pollfd, no waiting.
            let ready = unsafe { libc::poll(&mut poll, 1, 0) };
            if ready < 0 {
                return true;
            }
            if ready == 0 {
                return false;
            }
            if poll.revents & (libc::POLLHUP | libc::POLLERR | libc::POLLNVAL) != 0 {
                return true;
            }
            let mut byte = 0u8;
            // SAFETY: peeks at most one byte into a live buffer, without blocking.
            let n = unsafe { libc::recv(fd, (&mut byte as *mut u8).cast(), 1, libc::MSG_PEEK | libc::MSG_DONTWAIT) };
            match n {
                0 => true,
                n if n > 0 => false,
                _ => {
                    let err = io::Error::last_os_error().kind();
                    !matches!(err, io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted)
                }
            }
        }
    }

    #[cfg(any(target_os = "linux", target_os = "android"))]
    const SEND_FLAGS: libc::c_int = libc::MSG_NOSIGNAL;
    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    const SEND_FLAGS: libc::c_int = 0;

    /// The peer process must run as the same user as Buddy.
    fn peer_is_current_user(fd: RawFd) -> bool {
        #[cfg(any(target_os = "linux", target_os = "android"))]
        {
            let mut cred = libc::ucred { pid: 0, uid: 0, gid: 0 };
            let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
            // SAFETY: `cred` and `len` are live and sized for SO_PEERCRED.
            let rc = unsafe {
                libc::getsockopt(fd, libc::SOL_SOCKET, libc::SO_PEERCRED, (&mut cred as *mut libc::ucred).cast(), &mut len)
            };
            rc == 0 && cred.uid == unsafe { libc::getuid() }
        }
        #[cfg(not(any(target_os = "linux", target_os = "android")))]
        {
            let mut uid: libc::uid_t = 0;
            let mut gid: libc::gid_t = 0;
            // SAFETY: plain out-parameters.
            let rc = unsafe { libc::getpeereid(fd, &mut uid, &mut gid) };
            rc == 0 && uid == unsafe { libc::getuid() }
        }
    }

    /// Read/write timeouts so a silent client cannot hold a thread forever; no SIGPIPE on Apple platforms.
    fn configure(stream: &UnixStream) {
        let _ = stream.set_read_timeout(Some(READ_TIMEOUT));
        let _ = stream.set_write_timeout(Some(READ_TIMEOUT));
        #[cfg(any(target_os = "macos", target_os = "ios", target_os = "freebsd"))]
        {
            let on: libc::c_int = 1;
            // SAFETY: a live c_int of the size we pass.
            unsafe {
                libc::setsockopt(
                    stream.as_raw_fd(),
                    libc::SOL_SOCKET,
                    libc::SO_NOSIGPIPE,
                    (&on as *const libc::c_int).cast(),
                    std::mem::size_of::<libc::c_int>() as libc::socklen_t,
                );
            }
        }
    }

    /// Binds `path` as 0600 inside a 0700 folder. A stale socket from a previous run is replaced.
    fn bind(path: &Path) -> io::Result<UnixListener> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
            std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
        }
        let _ = std::fs::remove_file(path);
        // SAFETY: umask only swaps the process mask; it is put back right after the bind.
        let previous = unsafe { libc::umask(0o077) };
        let listener = UnixListener::bind(path);
        unsafe { libc::umask(previous) };
        let listener = listener?;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
        Ok(listener)
    }

    pub(super) fn spawn(path: PathBuf, sink: Arc<dyn Sink>) -> io::Result<()> {
        let listener = bind(&path)?;
        std::thread::Builder::new().name("buddy-hooks".into()).spawn(move || serve(listener, sink))?;
        Ok(())
    }

    fn serve(listener: UnixListener, sink: Arc<dyn Sink>) {
        let active = Arc::new(AtomicUsize::new(0));
        loop {
            let stream = match listener.accept() {
                Ok((stream, _)) => stream,
                Err(err) => {
                    match err.raw_os_error() {
                        Some(libc::EINTR) | Some(libc::ECONNABORTED) => continue,
                        Some(libc::EMFILE) | Some(libc::ENFILE) => {
                            std::thread::sleep(Duration::from_millis(200));
                            continue;
                        }
                        _ => {}
                    }
                    log::line(format!("servidor de ganchos detenido: {err}"));
                    return;
                }
            };
            // Same user only, and a bounded number of connections at once.
            if !peer_is_current_user(stream.as_raw_fd()) {
                continue;
            }
            let Some(slot) = Slot::take(&active) else {
                log::line("demasiadas conexiones de ganchos abiertas: se descarta una");
                continue;
            };
            configure(&stream);
            let sink = sink.clone();
            let spawned = std::thread::Builder::new().name("buddy-hook-conn".into()).spawn(move || {
                let _slot = slot;
                handle(&mut SocketConn(stream), sink.as_ref());
            });
            if spawned.is_err() {
                log::line("no se pudo atender una conexión de ganchos");
            }
        }
    }
}

#[cfg(windows)]
mod windows_pipe {
    use super::*;
    use std::fs::File;
    use std::os::windows::io::{AsRawHandle, FromRawHandle};

    use windows::Win32::Foundation::{ERROR_PIPE_CONNECTED, HANDLE, HLOCAL, LocalFree};
    use windows::Win32::Security::Authorization::{
        ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
    };
    use windows::Win32::Security::{PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES};
    use windows::Win32::Storage::FileSystem::{FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_FLAGS_AND_ATTRIBUTES, FlushFileBuffers, PIPE_ACCESS_DUPLEX};
    use windows::Win32::System::Pipes::{
        ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, GetNamedPipeClientProcessId, PIPE_READMODE_BYTE,
        PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT, PeekNamedPipe,
    };
    use windows::core::HSTRING;

    use crate::sessions::win_user;

    const BUFFER_SIZE: u32 = 64 * 1024;

    fn handle_of(file: &File) -> HANDLE {
        HANDLE(file.as_raw_handle())
    }

    struct PipeConn(File);

    impl PipeConn {
        /// Bytes waiting in the pipe, or `None` once the client is gone (`ERROR_BROKEN_PIPE`).
        fn available(&self) -> Option<u32> {
            let mut avail = 0u32;
            // SAFETY: a live pipe handle and a live out-parameter; no buffer is read.
            unsafe { PeekNamedPipe(handle_of(&self.0), None, 0, None, Some(&mut avail as *mut u32), None) }.ok()?;
            Some(avail)
        }
    }

    impl Conn for PipeConn {
        /// A blocking ReadFile cannot time out, so we only read what PeekNamedPipe says is already there.
        fn read_request(&mut self) -> Option<Vec<u8>> {
            use std::io::Read;
            let this = &*self;
            let mut file = &self.0;
            collect_line(|buf| match this.available() {
                None => Ok(0),
                Some(0) => {
                    // `collect_line` checks the deadline between reads: "nothing yet", try again shortly.
                    std::thread::sleep(Duration::from_millis(10));
                    Err(io::Error::from(io::ErrorKind::WouldBlock))
                }
                Some(n) => {
                    let take = (n as usize).min(buf.len());
                    file.read(&mut buf[..take])
                }
            })
        }

        fn send_line(&mut self, text: &str) {
            use std::io::Write;
            let _ = self.0.write_all(format!("{text}\n").as_bytes());
            // SAFETY: a live pipe handle; waits until the relay has read the line (or is gone).
            let _ = unsafe { FlushFileBuffers(handle_of(&self.0)) };
        }

        fn peer_closed(&self) -> bool {
            self.available().is_none()
        }
    }

    impl Drop for PipeConn {
        fn drop(&mut self) {
            // SAFETY: a live pipe handle; the File closes it right after.
            let _ = unsafe { DisconnectNamedPipe(handle_of(&self.0)) };
        }
    }

    /// An access list that names the current user and nobody else. Without it, Windows' default DACL would let
    /// other accounts on the machine reach the pipe.
    struct OwnerOnly(PSECURITY_DESCRIPTOR);

    impl OwnerOnly {
        fn new() -> Option<Self> {
            let sid = win_user::current_user_sid()?;
            let sddl = HSTRING::from(format!("D:P(A;;GA;;;{sid})"));
            let mut sd = PSECURITY_DESCRIPTOR::default();
            // SAFETY: a valid SDDL string and a live out-parameter; freed with LocalFree in Drop.
            unsafe { ConvertStringSecurityDescriptorToSecurityDescriptorW(&sddl, SDDL_REVISION_1, &mut sd, None) }.ok()?;
            Some(Self(sd))
        }

        fn attributes(&self) -> SECURITY_ATTRIBUTES {
            SECURITY_ATTRIBUTES {
                nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
                lpSecurityDescriptor: self.0.0,
                bInheritHandle: false.into(),
            }
        }
    }

    impl Drop for OwnerOnly {
        fn drop(&mut self) {
            // SAFETY: allocated by ConvertStringSecurityDescriptorToSecurityDescriptorW.
            unsafe {
                let _ = LocalFree(Some(HLOCAL(self.0.0)));
            }
        }
    }

    /// One pipe instance. `first` refuses to join a pipe somebody else already owns under our name.
    fn create_instance(name: &HSTRING, first: bool, security: Option<&OwnerOnly>) -> io::Result<File> {
        let mut open_mode: FILE_FLAGS_AND_ATTRIBUTES = PIPE_ACCESS_DUPLEX;
        if first {
            open_mode = open_mode | FILE_FLAG_FIRST_PIPE_INSTANCE;
        }
        let attributes = security.map(OwnerOnly::attributes);
        // SAFETY: `attributes` (and the descriptor it points at) outlive the call; the kernel copies them.
        let handle = unsafe {
            CreateNamedPipeW(
                name,
                open_mode,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                PIPE_UNLIMITED_INSTANCES,
                BUFFER_SIZE,
                BUFFER_SIZE,
                0,
                attributes.as_ref().map(|a| a as *const SECURITY_ATTRIBUTES),
            )
        };
        if handle.is_invalid() {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: a fresh handle we own; the File closes it.
        Ok(unsafe { File::from_raw_handle(handle.0) })
    }

    /// Waits for a client on `pipe`. A client that connected between create and connect is a success too.
    fn wait_for_client(pipe: &File) -> bool {
        // SAFETY: a live pipe handle, synchronous mode.
        match unsafe { ConnectNamedPipe(handle_of(pipe), None) } {
            Ok(()) => true,
            Err(err) => err.code() == ERROR_PIPE_CONNECTED.to_hresult(),
        }
    }

    fn client_is_current_user(pipe: &File) -> bool {
        let mut pid = 0u32;
        // SAFETY: a live pipe handle and out-parameter.
        if unsafe { GetNamedPipeClientProcessId(handle_of(pipe), &mut pid) }.is_err() || pid == 0 {
            return false;
        }
        win_user::process_is_current_user(pid)
    }

    pub(super) fn spawn(name: String, sink: Arc<dyn Sink>) -> io::Result<()> {
        // The security descriptor holds raw pointers, so everything Win32 lives on the server thread; the first
        // instance's result comes back over a channel so the caller still learns whether the pipe exists.
        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<io::Result<()>>();
        std::thread::Builder::new().name("buddy-hooks".into()).spawn(move || {
            let name = HSTRING::from(name);
            let security = OwnerOnly::new();
            if security.is_none() {
                log::line("no se pudo crear el permiso solo-dueño de la tubería: se usa el predeterminado");
            }
            let mut next = match create_instance(&name, true, security.as_ref()) {
                Ok(pipe) => {
                    let _ = ready_tx.send(Ok(()));
                    pipe
                }
                Err(err) => {
                    let _ = ready_tx.send(Err(err));
                    return;
                }
            };
            let active = Arc::new(AtomicUsize::new(0));
            loop {
                if !wait_for_client(&next) {
                    // Nobody useful on this instance: drop it and open a fresh one.
                    drop(std::mem::replace(&mut next, match create_instance(&name, false, security.as_ref()) {
                        Ok(pipe) => pipe,
                        Err(err) => {
                            log::line(format!("no se pudo reabrir la tubería de ganchos: {err}"));
                            return;
                        }
                    }));
                    std::thread::sleep(Duration::from_millis(50));
                    continue;
                }
                // Hand the connected instance to a thread and listen on a fresh one.
                let fresh = match create_instance(&name, false, security.as_ref()) {
                    Ok(pipe) => pipe,
                    Err(err) => {
                        log::line(format!("no se pudo reabrir la tubería de ganchos: {err}"));
                        return;
                    }
                };
                let connected = PipeConn(std::mem::replace(&mut next, fresh));
                if !client_is_current_user(&connected.0) {
                    continue;
                }
                let Some(slot) = Slot::take(&active) else {
                    log::line("demasiadas conexiones de ganchos abiertas: se descarta una");
                    continue;
                };
                let sink = sink.clone();
                let spawned = std::thread::Builder::new().name("buddy-hook-conn".into()).spawn(move || {
                    let _slot = slot;
                    let mut connected = connected;
                    handle(&mut connected, sink.as_ref());
                });
                if spawned.is_err() {
                    log::line("no se pudo atender una conexión de ganchos");
                }
            }
        })?;
        ready_rx.recv().unwrap_or_else(|_| Err(io::Error::other("el servidor de ganchos no arrancó")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reader(chunks: Vec<&'static [u8]>) -> impl FnMut(&mut [u8]) -> io::Result<usize> {
        let mut chunks = chunks.into_iter();
        move |buf: &mut [u8]| match chunks.next() {
            Some(c) => {
                buf[..c.len()].copy_from_slice(c);
                Ok(c.len())
            }
            None => Ok(0),
        }
    }

    #[test]
    fn a_request_is_one_line_however_it_arrives() {
        assert_eq!(collect_line(reader(vec![b"{\"a\"", b":1}\nrest"])).unwrap(), b"{\"a\":1}");
        assert_eq!(collect_line(reader(vec![b"{}"])).unwrap(), b"{}", "EOF ends the line too");
        assert!(collect_line(reader(vec![])).is_none(), "an empty request is no request");
        assert!(collect_line(|_: &mut [u8]| Err(io::Error::from(io::ErrorKind::ConnectionReset))).is_none());
    }

    #[test]
    fn an_oversized_request_is_refused() {
        static CHUNK: [u8; 4096] = [b'x'; 4096];
        let mut sent = 0;
        let got = collect_line(|buf: &mut [u8]| {
            sent += 1;
            buf.copy_from_slice(&CHUNK);
            Ok(CHUNK.len())
        });
        assert!(got.is_none());
        assert!(sent <= MAX_REQUEST_BYTES / CHUNK.len() + 2);
    }
}
