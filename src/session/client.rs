//! The daemon client — connects to the daemon's Unix socket, handshakes, and
//! performs list/new/attach/kill plus raw input/output streaming. Ported from
//! TUIOS `internal/session/client.go`.

use std::io;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::daemon::default_socket_path;
use super::model::{SessionInfo, WindowInfo};
use super::protocol::{self, Message};

/// A connected daemon client.
#[derive(Clone)]
pub struct DaemonClient {
    stream: Arc<Mutex<UnixStream>>,
}

impl DaemonClient {
    /// Connect to the default socket path and complete the handshake.
    pub fn connect() -> io::Result<Self> {
        Self::connect_to(&default_socket_path())
    }

    /// Connect to a specific socket path and complete the handshake.
    pub fn connect_to(path: &Path) -> io::Result<Self> {
        let stream = UnixStream::connect(path)?;
        let client = Self {
            stream: Arc::new(Mutex::new(stream)),
        };
        client.send(&Message::Hello {
            name: "tuios-client".to_string(),
            codec: None,
            cols: None,
            rows: None,
        })?;
        match client.recv()? {
            Message::Welcome { .. } => Ok(client),
            Message::Error { message } => Err(io::Error::other(message)),
            other => Err(io::Error::other(format!(
                "unexpected handshake reply: {other:?}"
            ))),
        }
    }

    pub fn send(&self, msg: &Message) -> io::Result<()> {
        let mut s = self.stream.lock().unwrap();
        protocol::write_message(&mut *s, msg)
    }

    pub fn recv(&self) -> io::Result<Message> {
        let mut s = self.stream.lock().unwrap();
        protocol::read_message(&mut *s)
    }

    /// Read the next control reply, discarding interleaved streaming frames
    /// (`PtyOutput`/`PtyClosed`) that arrive while a session is attached.
    fn recv_reply(&self) -> io::Result<Message> {
        loop {
            match self.recv()? {
                Message::PtyOutput { .. } | Message::PtyClosed { .. } => continue,
                other => return Ok(other),
            }
        }
    }

    pub fn set_read_timeout(&self, d: Duration) -> io::Result<()> {
        self.stream.lock().unwrap().set_read_timeout(Some(d))
    }

    /// The raw stream, for callers that want to multiplex reads themselves.
    pub fn stream(&self) -> Arc<Mutex<UnixStream>> {
        Arc::clone(&self.stream)
    }

    /// An independent read handle so a reader thread can block on frames
    /// without holding the write lock used by `send`.
    pub fn reader(&self) -> io::Result<UnixStream> {
        self.stream.lock().unwrap().try_clone()
    }

    pub fn list(&self) -> io::Result<Vec<SessionInfo>> {
        self.send(&Message::List)?;
        match self.recv_reply()? {
            Message::ListResult { sessions } => Ok(sessions),
            Message::Error { message } => Err(io::Error::other(message)),
            other => Err(io::Error::other(format!("unexpected reply: {other:?}"))),
        }
    }

    pub fn new_session(&self, name: &str, shell: &str) -> io::Result<Vec<SessionInfo>> {
        self.send(&Message::New {
            name: name.to_string(),
            shell: shell.to_string(),
        })?;
        match self.recv_reply()? {
            Message::ListResult { sessions } => Ok(sessions),
            Message::Error { message } => Err(io::Error::other(message)),
            other => Err(io::Error::other(format!("unexpected reply: {other:?}"))),
        }
    }

    pub fn attach(&self, name: &str) -> io::Result<Vec<WindowInfo>> {
        self.send(&Message::Attach {
            name: name.to_string(),
        })?;
        match self.recv_reply()? {
            Message::Attached { windows } => Ok(windows),
            Message::Error { message } => Err(io::Error::other(message)),
            other => Err(io::Error::other(format!("unexpected reply: {other:?}"))),
        }
    }

    pub fn detach(&self) -> io::Result<()> {
        self.send(&Message::Detach)
    }

    pub fn kill(&self, name: &str) -> io::Result<Vec<SessionInfo>> {
        self.send(&Message::Kill {
            name: name.to_string(),
        })?;
        match self.recv_reply()? {
            Message::ListResult { sessions } => Ok(sessions),
            Message::Error { message } => Err(io::Error::other(message)),
            other => Err(io::Error::other(format!("unexpected reply: {other:?}"))),
        }
    }

    pub fn send_input(&self, window: &str, data: &[u8]) -> io::Result<()> {
        self.send(&Message::Input {
            window: window.to_string(),
            data: data.to_vec(),
        })
    }

    pub fn send_resize(&self, window: &str, cols: u16, rows: u16) -> io::Result<()> {
        self.send(&Message::Resize {
            window: window.to_string(),
            cols,
            rows,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn daemon_client_connect_to_nonexistent_fails() {
        let result = DaemonClient::connect_to(Path::new("/tmp/nonexistent-termos-test.sock"));
        assert!(result.is_err());
    }

    #[test]
    fn default_socket_path_contains_termos() {
        let path = default_socket_path();
        let s = path.to_string_lossy();
        assert!(s.contains("termos") || s.contains("tuios"));
    }
}
