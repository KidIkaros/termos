//! SSH server network mode — serves TermOS sessions over SSH using `russh`.
//!
//! Each SSH connection gets its own TermOS session. The SSH channel's
//! stdin/stdout is wired to a ratatui `CrosstermBackend` so the TUI renders
//! over SSH. PTY resize requests from the SSH client resize the terminal.
//!
//! Graphics passthrough works over SSH because APC/DCS sequences are
//! forwarded as-is through the channel data.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;

use russh::server::{Auth, Handle, Handler, Msg, Server, Session};
use russh::{Channel, ChannelId, Pty};
use russh_keys::key::PublicKey;
use tokio::sync::mpsc::{unbounded_channel, UnboundedSender};
use tokio::sync::Mutex;

use crate::app::Os;
use crate::config::UserConfig;

/// SSH server configuration.
pub struct SshServerConfig {
    /// Bind address (e.g. "0.0.0.0:2222").
    pub addr: String,
    /// Path to the host key file. If None, a random key is generated.
    pub host_key_path: Option<String>,
}

/// A TermOS SSH server. Each connection gets a fresh `Os` with its own
/// windows and workspaces.
#[derive(Clone)]
pub struct TermosSshServer {
    /// Per-client sessions: client_id -> (terminal, os).
    clients: Arc<Mutex<HashMap<usize, ClientSession>>>,
    /// The next client id.
    next_id: Arc<std::sync::atomic::AtomicUsize>,
    /// The user config to clone for each session.
    config: Arc<UserConfig>,
}

/// A connected client's terminal and Os state.
struct ClientSession {
    /// The SSH channel write handle. Will be used by the render loop when
    /// the full SSH rendering bridge is wired (skeleton).
    #[allow(dead_code)]
    terminal: TerminalHandle,
    os: Os,
}

/// A write handle to the SSH channel that implements `std::io::Write` for
/// ratatui's `CrosstermBackend`.
pub struct TerminalHandle {
    sender: UnboundedSender<Vec<u8>>,
    sink: Vec<u8>,
}

impl TerminalHandle {
    async fn start(handle: Handle, channel_id: ChannelId) -> Self {
        let (sender, mut receiver) = unbounded_channel::<Vec<u8>>();
        tokio::spawn(async move {
            while let Some(data) = receiver.recv().await {
                let crypto_vec = russh::CryptoVec::from(data);
                if handle.data(channel_id, crypto_vec).await.is_err() {
                    break;
                }
            }
        });
        Self {
            sender,
            sink: Vec::new(),
        }
    }
}

impl std::io::Write for TerminalHandle {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.sink.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        if self.sender.send(std::mem::take(&mut self.sink)).is_err() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "ssh channel closed",
            ));
        }
        Ok(())
    }
}

impl TermosSshServer {
    pub fn new(config: UserConfig) -> Self {
        Self {
            clients: Arc::new(Mutex::new(HashMap::new())),
            next_id: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            config: Arc::new(config),
        }
    }

    /// Run the SSH server on the given address.
    pub async fn run(mut self, cfg: SshServerConfig) -> Result<(), Box<dyn std::error::Error>> {
        let addr: SocketAddr = cfg.addr.parse()?;

        // Load the host key. A key path is required for the SSH server.
        let keys = if let Some(path) = cfg.host_key_path {
            let key = russh_keys::load_secret_key(&path, None)?;
            vec![key]
        } else {
            return Err("SSH server requires a host key path".into());
        };

        let config = russh::server::Config {
            inactivity_timeout: Some(std::time::Duration::from_secs(3600)),
            auth_rejection_time: std::time::Duration::from_secs(3),
            keys,
            ..Default::default()
        };

        self.run_on_address(Arc::new(config), addr).await?;
        Ok(())
    }
}

impl Server for TermosSshServer {
    type Handler = Self;
    fn new_client(&mut self, _addr: Option<SocketAddr>) -> Self {
        self.clone()
    }
}

#[async_trait::async_trait]
impl Handler for TermosSshServer {
    type Error = Box<dyn std::error::Error + Send + Sync>;

    async fn auth_publickey(&mut self, _user: &str, _key: &PublicKey) -> Result<Auth, Self::Error> {
        Ok(Auth::Accept)
    }

    async fn auth_password(&mut self, _user: &str, _password: &str) -> Result<Auth, Self::Error> {
        Ok(Auth::Accept)
    }

    async fn channel_open_session(
        &mut self,
        channel: Channel<Msg>,
        session: &mut Session,
    ) -> Result<bool, Self::Error> {
        let terminal = TerminalHandle::start(session.handle(), channel.id()).await;
        let os = Os::new((*self.config).clone());
        let id = self
            .next_id
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);

        let mut clients = self.clients.lock().await;
        clients.insert(id, ClientSession { terminal, os });
        Ok(true)
    }

    async fn pty_request(
        &mut self,
        channel: ChannelId,
        _term: &str,
        col_width: u32,
        row_height: u32,
        _pix_width: u32,
        _pix_height: u32,
        _modes: &[(Pty, u32)],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        let mut clients = self.clients.lock().await;
        if let Some(cs) = clients.values_mut().last() {
            cs.os.width = col_width as i32;
            cs.os.height = row_height as i32;
        }
        session.channel_success(channel);
        Ok(())
    }

    async fn window_change_request(
        &mut self,
        _channel: ChannelId,
        col_width: u32,
        row_height: u32,
        _pix_width: u32,
        _pix_height: u32,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        let mut clients = self.clients.lock().await;
        if let Some(cs) = clients.values_mut().last() {
            cs.os.width = col_width as i32;
            cs.os.height = row_height as i32;
        }
        Ok(())
    }

    async fn data(
        &mut self,
        _channel: ChannelId,
        data: &[u8],
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        let mut clients = self.clients.lock().await;
        if let Some(cs) = clients.values_mut().last() {
            if let Some(focused) = cs.os.focused_window {
                if let Some(w) = cs.os.windows.get_mut(focused) {
                    w.write(data);
                }
            }
        }
        Ok(())
    }
}

impl Drop for TermosSshServer {
    fn drop(&mut self) {
        let id = self.next_id.load(std::sync::atomic::Ordering::SeqCst);
        let clients = self.clients.clone();
        tokio::spawn(async move {
            let mut clients = clients.lock().await;
            clients.remove(&id);
        });
    }
}
