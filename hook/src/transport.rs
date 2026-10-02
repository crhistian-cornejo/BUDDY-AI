//! Where Buddy listens, and how we reach it. One `connect()` per platform, both returning something to read and
//! write; anything that goes wrong returns `None` and the agent carries on alone.

#[cfg(unix)]
pub use unix::connect;
#[cfg(windows)]
pub use windows_pipe::connect;

/// Buddy's data folder, as the core resolves it: `$BUDDY_DATA_DIR`, else `~/Library/Application Support/Buddy`
/// (Mac) or `~/.local/share/buddy` (other Unix).
#[cfg(unix)]
pub fn data_dir_from(
    buddy_data_dir: Option<std::ffi::OsString>,
    home: Option<std::ffi::OsString>,
) -> Option<std::path::PathBuf> {
    use std::path::PathBuf;
    if let Some(dir) = buddy_data_dir.filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(dir));
    }
    let home = PathBuf::from(home.filter(|v| !v.is_empty())?);
    if cfg!(target_os = "macos") {
        Some(home.join("Library/Application Support/Buddy"))
    } else {
        Some(home.join(".local/share/buddy"))
    }
}

#[cfg(unix)]
mod unix {
    use std::os::unix::io::AsRawFd;
    use std::os::unix::net::UnixStream;

    /// `<data_dir>/hooks.sock`.
    fn socket_path() -> Option<std::path::PathBuf> {
        super::data_dir_from(std::env::var_os("BUDDY_DATA_DIR"), std::env::var_os("HOME"))
            .map(|dir| dir.join("hooks.sock"))
    }

    /// The socket's owner must be us. Its folder is 0700 and the socket 0600, so this is belt and braces: nobody
    /// else should be able to stand in for Buddy, and if somebody does they get nothing.
    fn server_is_same_user(stream: &UnixStream) -> bool {
        let fd = stream.as_raw_fd();
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

    pub fn connect() -> Option<UnixStream> {
        let stream = UnixStream::connect(socket_path()?).ok()?;
        server_is_same_user(&stream).then_some(stream)
    }
}

#[cfg(windows)]
mod windows_pipe {
    use std::time::{Duration, Instant};

    /// Budget for getting a pipe connection. Beyond this the agent wins, always.
    const CONNECT_TIMEOUT: Duration = Duration::from_millis(300);
    /// `ERROR_PIPE_BUSY` — every instance is serving someone else right now. This is the one error worth retrying:
    /// the server exists and a slot will free up.
    const ERROR_PIPE_BUSY: i32 = 231;

    /// `\\.\pipe\buddy-<sid>`. The SID keeps two accounts on the same machine from ever meeting on the same pipe;
    /// the name falls back to the user name only if the SID cannot be read at all, which should not happen.
    fn pipe_path() -> String {
        // Debug builds only (never in a release): lets a test talk to a stand-in server instead of the running app.
        #[cfg(debug_assertions)]
        if let Some(name) = std::env::var("BUDDY_TEST_PIPE").ok().filter(|n| !n.is_empty()) {
            return format!(r"\\.\pipe\{name}");
        }
        let key = crate::win::current_user_sid()
            .unwrap_or_else(|| std::env::var("USERNAME").unwrap_or_else(|_| "user".into()));
        format!(r"\\.\pipe\buddy-{key}")
    }

    /// Opens the pipe. Retries only while the server is busy: any other error means there is nothing to talk to,
    /// and waiting would only delay the agent.
    pub fn connect() -> Option<std::fs::File> {
        use std::os::windows::io::AsRawHandle;
        let path = pipe_path();
        let deadline = Instant::now() + CONNECT_TIMEOUT;
        loop {
            match std::fs::OpenOptions::new().read(true).write(true).open(&path) {
                Ok(file) => {
                    let handle = windows::Win32::Foundation::HANDLE(file.as_raw_handle());
                    // Somebody else's server on our pipe name gets nothing from us.
                    return crate::win::pipe_server_is_same_user(handle).then_some(file);
                }
                Err(err) => {
                    if err.raw_os_error() != Some(ERROR_PIPE_BUSY) || Instant::now() >= deadline {
                        return None;
                    }
                    std::thread::sleep(Duration::from_millis(15));
                }
            }
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::ffi::OsString;
    use std::path::PathBuf;

    #[test]
    fn the_data_folder_follows_the_override_then_the_platform() {
        assert_eq!(
            data_dir_from(Some(OsString::from("/tmp/b")), Some(OsString::from("/Users/a"))),
            Some(PathBuf::from("/tmp/b"))
        );
        let default = data_dir_from(Some(OsString::new()), Some(OsString::from("/Users/a"))).unwrap();
        if cfg!(target_os = "macos") {
            assert_eq!(default, PathBuf::from("/Users/a/Library/Application Support/Buddy"));
        } else {
            assert_eq!(default, PathBuf::from("/Users/a/.local/share/buddy"));
        }
        assert_eq!(data_dir_from(None, None), None);
    }
}
