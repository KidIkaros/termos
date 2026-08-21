//! SSH server network mode — serves TermOS sessions over SSH using `russh`.
//!
//! Each SSH connection gets its own TermOS session. The SSH channel's
//! stdin/stdout is wired to a ratatui `CrosstermBackend` so the TUI renders
//! over SSH. PTY resize requests from the SSH client resize the terminal.
//!
//! Graphics passthrough works over SSH because APC/DCS sequences are
//! forwarded as-is through the channel data.

use std::collections::HashMap;
use std::io::Write;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use russh::server::{Auth, Handle, Handler, Msg, Server, Session};
use russh::{Channel, ChannelId, Pty};
use russh_keys::key::PublicKey;
use tokio::sync::mpsc::{unbounded_channel, UnboundedSender};
use tokio::sync::Mutex;

use crate::app::render::render;
use crate::app::Os;
use crate::config::UserConfig;

/// SSH server configuration.
pub struct SshServerConfig {
    /// Bind address (e.g. "0.0.0.0:2222").
    pub addr: String,
    /// Path to the host key file. If None, a random key is generated.
    pub host_key_path: Option<String>,
    /// Read-only observer mode: client input is dropped at the Os layer.
    pub read_only: bool,
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
    /// Read-only observer mode: client input is dropped at the Os layer.
    read_only: bool,
}

/// A connected client's terminal and Os state.
struct ClientSession {
    /// The SSH channel write handle.
    terminal: TerminalHandle,
    os: Os,
    /// The client's reported terminal capabilities (kitty/sixel/cell size),
    /// detected from the pty-req and forwarded environment.
    caps: Option<crate::server::ClientCapabilities>,
}

/// A write handle to the SSH channel that implements `std::io::Write` for
/// ratatui's `CrosstermBackend`.
pub struct TerminalHandle {
    sender: UnboundedSender<Vec<u8>>,
    sink: Vec<u8>,
}

/// A simple writer that forwards bytes to the SSH channel via an unbounded
/// sender. Used by graphics passthrough to write APC/sixel sequences directly
/// to the channel.
struct ChannelWriter {
    sender: UnboundedSender<Vec<u8>>,
}

impl ChannelWriter {
    fn new(sender: UnboundedSender<Vec<u8>>) -> Self {
        Self { sender }
    }
}

impl std::io::Write for ChannelWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.sender
            .send(buf.to_vec())
            .map_err(|_| std::io::Error::new(std::io::ErrorKind::BrokenPipe, "channel closed"))?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
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

impl Write for TerminalHandle {
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

/// Parse raw SSH terminal input bytes into crossterm `KeyEvent`s.
///
/// SSH clients send raw terminal input (VT/xterm sequences). This parser
/// handles the common sequences needed for window management mode:
/// - Ctrl+letter combinations (Ctrl+B, Ctrl+C, etc.)
/// - Arrow keys (ESC [ A/B/C/D)
/// - Escape key
/// - Enter, Tab, Backspace
/// - Function keys (F1-F12)
///
/// Returns a list of parsed key events. Unrecognized sequences are returned
/// as raw bytes for passthrough to the PTY.
pub fn parse_ssh_input(data: &[u8]) -> Vec<crossterm::event::KeyEvent> {
    let mut events = Vec::new();
    let mut i = 0;

    while i < data.len() {
        let b = data[i];

        match b {
            // ESC sequences
            0x1b => {
                if i + 1 < data.len() {
                    match data[i + 1] {
                        // CSI sequences: ESC [
                        b'[' => {
                            if i + 2 < data.len() {
                                match data[i + 2] {
                                    b'A' => {
                                        events.push(crossterm::event::KeyEvent {
                                            code: crossterm::event::KeyCode::Up,
                                            modifiers: crossterm::event::KeyModifiers::NONE,
                                            kind: crossterm::event::KeyEventKind::Press,
                                            state: crossterm::event::KeyEventState::NONE,
                                        });
                                        i += 3;
                                    }
                                    b'B' => {
                                        events.push(crossterm::event::KeyEvent {
                                            code: crossterm::event::KeyCode::Down,
                                            modifiers: crossterm::event::KeyModifiers::NONE,
                                            kind: crossterm::event::KeyEventKind::Press,
                                            state: crossterm::event::KeyEventState::NONE,
                                        });
                                        i += 3;
                                    }
                                    b'C' => {
                                        events.push(crossterm::event::KeyEvent {
                                            code: crossterm::event::KeyCode::Right,
                                            modifiers: crossterm::event::KeyModifiers::NONE,
                                            kind: crossterm::event::KeyEventKind::Press,
                                            state: crossterm::event::KeyEventState::NONE,
                                        });
                                        i += 3;
                                    }
                                    b'D' => {
                                        events.push(crossterm::event::KeyEvent {
                                            code: crossterm::event::KeyCode::Left,
                                            modifiers: crossterm::event::KeyModifiers::NONE,
                                            kind: crossterm::event::KeyEventKind::Press,
                                            state: crossterm::event::KeyEventState::NONE,
                                        });
                                        i += 3;
                                    }
                                    b'H' => {
                                        events.push(crossterm::event::KeyEvent {
                                            code: crossterm::event::KeyCode::Home,
                                            modifiers: crossterm::event::KeyModifiers::NONE,
                                            kind: crossterm::event::KeyEventKind::Press,
                                            state: crossterm::event::KeyEventState::NONE,
                                        });
                                        i += 3;
                                    }
                                    b'F' => {
                                        events.push(crossterm::event::KeyEvent {
                                            code: crossterm::event::KeyCode::End,
                                            modifiers: crossterm::event::KeyModifiers::NONE,
                                            kind: crossterm::event::KeyEventKind::Press,
                                            state: crossterm::event::KeyEventState::NONE,
                                        });
                                        i += 3;
                                    }
                                    b'Z' => {
                                        // Shift+Tab (backtab)
                                        events.push(crossterm::event::KeyEvent {
                                            code: crossterm::event::KeyCode::BackTab,
                                            modifiers: crossterm::event::KeyModifiers::SHIFT,
                                            kind: crossterm::event::KeyEventKind::Press,
                                            state: crossterm::event::KeyEventState::NONE,
                                        });
                                        i += 3;
                                    }
                                    // CSI sequences with tilde: ESC [ <number> ~
                                    b'1'..=b'9' => {
                                        // Parse the number
                                        let mut num = (data[i + 2] - b'0') as u32;
                                        let mut j = i + 3;
                                        while j < data.len() && data[j].is_ascii_digit() {
                                            num = num * 10 + (data[j] - b'0') as u32;
                                            j += 1;
                                        }
                                        if j < data.len() && data[j] == b'~' {
                                            let code = match num {
                                                1 => Some(crossterm::event::KeyCode::Home),
                                                2 => Some(crossterm::event::KeyCode::Insert),
                                                3 => Some(crossterm::event::KeyCode::Delete),
                                                4 => Some(crossterm::event::KeyCode::End),
                                                5 => Some(crossterm::event::KeyCode::PageUp),
                                                6 => Some(crossterm::event::KeyCode::PageDown),
                                                11..=13 => Some(crossterm::event::KeyCode::F(
                                                    (num - 10) as u8,
                                                )),
                                                14..=15 => Some(crossterm::event::KeyCode::F(
                                                    (num - 10) as u8,
                                                )),
                                                17..=18 => Some(crossterm::event::KeyCode::F(
                                                    (num - 10) as u8,
                                                )),
                                                19..=21 => Some(crossterm::event::KeyCode::F(
                                                    (num - 10) as u8,
                                                )),
                                                23..=24 => Some(crossterm::event::KeyCode::F(
                                                    (num - 10) as u8,
                                                )),
                                                _ => None,
                                            };
                                            if let Some(code) = code {
                                                events.push(crossterm::event::KeyEvent {
                                                    code,
                                                    modifiers: crossterm::event::KeyModifiers::NONE,
                                                    kind: crossterm::event::KeyEventKind::Press,
                                                    state: crossterm::event::KeyEventState::NONE,
                                                });
                                            }
                                            i = j + 1;
                                        } else {
                                            // Not a tilde sequence, treat as escape
                                            events.push(crossterm::event::KeyEvent {
                                                code: crossterm::event::KeyCode::Esc,
                                                modifiers: crossterm::event::KeyModifiers::NONE,
                                                kind: crossterm::event::KeyEventKind::Press,
                                                state: crossterm::event::KeyEventState::NONE,
                                            });
                                            i += 1;
                                        }
                                    }
                                    _ => {
                                        // Unknown CSI sequence, skip ESC [
                                        i += 2;
                                    }
                                }
                            } else {
                                // Incomplete ESC [ sequence
                                events.push(crossterm::event::KeyEvent {
                                    code: crossterm::event::KeyCode::Esc,
                                    modifiers: crossterm::event::KeyModifiers::NONE,
                                    kind: crossterm::event::KeyEventKind::Press,
                                    state: crossterm::event::KeyEventState::NONE,
                                });
                                i += 1;
                            }
                        }
                        // Alt+letter: ESC x
                        _ if data[i + 1].is_ascii_alphabetic() => {
                            events.push(crossterm::event::KeyEvent {
                                code: crossterm::event::KeyCode::Char(data[i + 1] as char),
                                modifiers: crossterm::event::KeyModifiers::ALT,
                                kind: crossterm::event::KeyEventKind::Press,
                                state: crossterm::event::KeyEventState::NONE,
                            });
                            i += 2;
                        }
                        // Standalone ESC
                        _ => {
                            events.push(crossterm::event::KeyEvent {
                                code: crossterm::event::KeyCode::Esc,
                                modifiers: crossterm::event::KeyModifiers::NONE,
                                kind: crossterm::event::KeyEventKind::Press,
                                state: crossterm::event::KeyEventState::NONE,
                            });
                            i += 1;
                        }
                    }
                } else {
                    // ESC at end of buffer
                    events.push(crossterm::event::KeyEvent {
                        code: crossterm::event::KeyCode::Esc,
                        modifiers: crossterm::event::KeyModifiers::NONE,
                        kind: crossterm::event::KeyEventKind::Press,
                        state: crossterm::event::KeyEventState::NONE,
                    });
                    i += 1;
                }
            }
            // Backspace
            0x08 | 0x7f => {
                events.push(crossterm::event::KeyEvent {
                    code: crossterm::event::KeyCode::Backspace,
                    modifiers: crossterm::event::KeyModifiers::NONE,
                    kind: crossterm::event::KeyEventKind::Press,
                    state: crossterm::event::KeyEventState::NONE,
                });
                i += 1;
            }
            // Tab
            b'\t' => {
                events.push(crossterm::event::KeyEvent {
                    code: crossterm::event::KeyCode::Tab,
                    modifiers: crossterm::event::KeyModifiers::NONE,
                    kind: crossterm::event::KeyEventKind::Press,
                    state: crossterm::event::KeyEventState::NONE,
                });
                i += 1;
            }
            // Newline / Enter
            b'\n' | b'\r' => {
                events.push(crossterm::event::KeyEvent {
                    code: crossterm::event::KeyCode::Enter,
                    modifiers: crossterm::event::KeyModifiers::NONE,
                    kind: crossterm::event::KeyEventKind::Press,
                    state: crossterm::event::KeyEventState::NONE,
                });
                i += 1;
            }
            // Ctrl+backslash, Ctrl+], Ctrl+^, Ctrl+_
            0x1c..=0x1f => {
                let c = (b + b'a' - 1) as char;
                events.push(crossterm::event::KeyEvent {
                    code: crossterm::event::KeyCode::Char(c),
                    modifiers: crossterm::event::KeyModifiers::CONTROL,
                    kind: crossterm::event::KeyEventKind::Press,
                    state: crossterm::event::KeyEventState::NONE,
                });
                i += 1;
            }
            // Printable ASCII
            0x20..=0x7e => {
                events.push(crossterm::event::KeyEvent {
                    code: crossterm::event::KeyCode::Char(b as char),
                    modifiers: crossterm::event::KeyModifiers::NONE,
                    kind: crossterm::event::KeyEventKind::Press,
                    state: crossterm::event::KeyEventState::NONE,
                });
                i += 1;
            }
            // Ctrl+letter (catch remaining 0x01..=0x1b not handled above)
            0x01..=0x1a => {
                let c = (b + b'a' - 1) as char;
                events.push(crossterm::event::KeyEvent {
                    code: crossterm::event::KeyCode::Char(c),
                    modifiers: crossterm::event::KeyModifiers::CONTROL,
                    kind: crossterm::event::KeyEventKind::Press,
                    state: crossterm::event::KeyEventState::NONE,
                });
                i += 1;
            }
            // Everything else: skip
            _ => {
                i += 1;
            }
        }
    }

    events
}

impl TermosSshServer {
    pub fn new(config: UserConfig) -> Self {
        Self {
            clients: Arc::new(Mutex::new(HashMap::new())),
            next_id: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            config: Arc::new(config),
            read_only: false,
        }
    }

    /// Run the SSH server on the given address.
    pub async fn run(mut self, cfg: SshServerConfig) -> Result<(), Box<dyn std::error::Error>> {
        self.read_only = cfg.read_only;
        let addr: SocketAddr = cfg.addr.parse()?;

        // Load the host key. A key path is required for the SSH server.
        let keys = if let Some(path) = cfg.host_key_path {
            let key = russh_keys::load_secret_key(&path, None)?;
            vec![key]
        } else {
            return Err("SSH server requires a host key path".into());
        };

        let config = russh::server::Config {
            inactivity_timeout: Some(Duration::from_secs(3600)),
            auth_rejection_time: Duration::from_secs(3),
            keys,
            ..Default::default()
        };

        self.run_on_address(Arc::new(config), addr).await?;
        Ok(())
    }

    /// Spawn the render loop for a client. This runs in a tokio task and
    /// continuously renders the Os state to the SSH channel.
    fn spawn_render_loop(client_id: usize, clients: Arc<Mutex<HashMap<usize, ClientSession>>>) {
        tokio::spawn(async move {
            let frame_budget = Duration::from_millis(16); // ~60 FPS
            let mut last_render = Instant::now();
            let mut buf: Vec<u8> = Vec::new();

            loop {
                // Check if client still exists.
                {
                    let clients = clients.lock().await;
                    if !clients.contains_key(&client_id) {
                        break;
                    }
                }

                // Rate-limit rendering and skip idle frames.
                if last_render.elapsed() < frame_budget {
                    tokio::time::sleep(Duration::from_millis(2)).await;
                    continue;
                }

                // Render the Os state.
                buf.clear();
                {
                    let mut clients = clients.lock().await;
                    if let Some(cs) = clients.get_mut(&client_id) {
                        // Tick the Os state machine.
                        cs.os.tick_agent_alerts();
                        cs.os.tick_script();
                        cs.os.sync_window_sizes();
                        cs.os.flush_graphics();

                        // Create a terminal backend writing to our buffer.
                        let backend = CrosstermBackend::new(&mut buf);
                        if cs.os.needs_render() {
                            cs.os.collect_pane_damage();
                            let damage = cs.os.damage_take();
                            if let Ok(mut terminal) = Terminal::new(backend) {
                                let _ = terminal.draw(|frame| {
                                    render(&cs.os, frame.buffer_mut(), &damage);
                                });
                                // Force flush the backend to write to buf.
                                let _ = terminal.backend_mut().flush();
                            }
                            cs.os.mark_rendered();
                        }

                        // Send the rendered output.
                        if !buf.is_empty() {
                            let data = std::mem::take(&mut buf);
                            if cs.terminal.sender.send(data).is_err() {
                                break; // Channel closed
                            }
                        }
                    }
                }

                last_render = Instant::now();
            }
        });
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
        let mut os = Os::new((*self.config).clone());
        os.read_only = self.read_only;
        os.init_graphics();

        let id = self
            .next_id
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);

        // Spawn a shell.
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
        let wake = Box::new(|| {}) as Box<dyn Fn() + Send + 'static>;
        if let Err(e) = os.spawn_window(&shell, wake) {
            log::warn!("ssh: failed to spawn shell for client {id}: {e}");
        }

        let mut clients = self.clients.lock().await;
        clients.insert(
            id,
            ClientSession {
                terminal,
                os,
                caps: None,
            },
        );

        // Spawn the render loop for this client.
        let clients_clone = self.clients.clone();
        Self::spawn_render_loop(id, clients_clone);

        Ok(true)
    }

    async fn pty_request(
        &mut self,
        channel: ChannelId,
        term: &str,
        col_width: u32,
        row_height: u32,
        pix_width: u32,
        pix_height: u32,
        _modes: &[(Pty, u32)],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        let mut clients = self.clients.lock().await;
        if let Some(cs) = clients.values_mut().last() {
            // Detect the client's graphics capabilities from its pty-req and
            // environment (kitty/sixel terminal identity, cell pixel size).
            let environ: Vec<String> = std::env::vars().map(|(k, v)| format!("{k}={v}")).collect();
            cs.caps = Some(crate::server::build_client_capabilities(
                term,
                &environ,
                col_width as i32,
                row_height as i32,
                pix_width as i32,
                pix_height as i32,
            ));
            // Project client capabilities onto the app's host graphics caps,
            // overriding the server's local probe with the remote client's
            // reported capabilities.
            if let Some(ref caps) = cs.caps {
                let host = crate::server::client_to_host_capabilities(caps);
                cs.os.graphics_caps.kitty = host.kitty_graphics;
                cs.os.graphics_caps.sixel = host.sixel_graphics;
                // Re-initialize passthrough if capabilities changed.
                // The passthrough writes to the SSH channel via a clone of
                // the terminal's sender.
                if host.kitty_graphics && cs.os.kitty_passthrough.is_none() {
                    let out: Box<dyn std::io::Write + Send> =
                        Box::new(ChannelWriter::new(cs.terminal.sender.clone()));
                    cs.os.kitty_passthrough = Some(crate::graphics::kitty::KittyPassthrough::new(
                        cs.os.graphics_caps,
                        out,
                    ));
                }
                if host.sixel_graphics && cs.os.sixel_passthrough.is_none() {
                    let out: Box<dyn std::io::Write + Send> =
                        Box::new(ChannelWriter::new(cs.terminal.sender.clone()));
                    cs.os.sixel_passthrough = Some(crate::graphics::sixel::SixelPassthrough::new(
                        cs.os.graphics_caps,
                        out,
                    ));
                }
            }
            cs.os.width = col_width as i32;
            cs.os.height = row_height as i32;
            cs.os.damage_resize(col_width as i32, row_height as i32);
            cs.os.sync_window_sizes();
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
            cs.os.damage_resize(col_width as i32, row_height as i32);
            cs.os.sync_window_sizes();
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
            // Try SGR mouse sequences first (terminal app may have enabled
            // mouse tracking); fall back to key-event parsing.
            if let Some(mouse) = crate::network::web::parse_sgr_mouse(data) {
                let effects = cs.os.update(crate::app::msg::Msg::Mouse(mouse));
                if effects
                    .iter()
                    .any(|e| matches!(e, crate::app::effect::Effect::Quit))
                {
                    return Ok(());
                }
            } else {
                let events = parse_ssh_input(data);
                for key_event in &events {
                    let effects = cs.os.update(crate::app::msg::Msg::Key(*key_event));
                    if effects
                        .iter()
                        .any(|e| matches!(e, crate::app::effect::Effect::Quit))
                    {
                        return Ok(());
                    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_ctrl_b() {
        // Ctrl+B = 0x02
        let events = parse_ssh_input(&[0x02]);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].code, crossterm::event::KeyCode::Char('b'));
        assert!(events[0]
            .modifiers
            .contains(crossterm::event::KeyModifiers::CONTROL));
    }

    #[test]
    fn parse_arrow_keys() {
        let events = parse_ssh_input(b"\x1b[A\x1b[B\x1b[C\x1b[D");
        assert_eq!(events.len(), 4);
        assert_eq!(events[0].code, crossterm::event::KeyCode::Up);
        assert_eq!(events[1].code, crossterm::event::KeyCode::Down);
        assert_eq!(events[2].code, crossterm::event::KeyCode::Right);
        assert_eq!(events[3].code, crossterm::event::KeyCode::Left);
    }

    #[test]
    fn parse_enter_and_tab() {
        let events = parse_ssh_input(b"\n\t");
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].code, crossterm::event::KeyCode::Enter);
        assert_eq!(events[1].code, crossterm::event::KeyCode::Tab);
    }

    #[test]
    fn parse_escape() {
        let events = parse_ssh_input(&[0x1b]);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].code, crossterm::event::KeyCode::Esc);
    }

    #[test]
    fn parse_printable_chars() {
        let events = parse_ssh_input(b"hello");
        assert_eq!(events.len(), 5);
        assert_eq!(events[0].code, crossterm::event::KeyCode::Char('h'));
        assert_eq!(events[4].code, crossterm::event::KeyCode::Char('o'));
    }

    #[test]
    fn parse_backspace() {
        let events = parse_ssh_input(&[0x7f]);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].code, crossterm::event::KeyCode::Backspace);
    }

    #[test]
    fn parse_mixed_input() {
        // Ctrl+B, then "ls\n"
        let events = parse_ssh_input(b"\x02ls\n");
        assert_eq!(events.len(), 4);
        assert_eq!(events[0].code, crossterm::event::KeyCode::Char('b'));
        assert!(events[0]
            .modifiers
            .contains(crossterm::event::KeyModifiers::CONTROL));
        assert_eq!(events[1].code, crossterm::event::KeyCode::Char('l'));
        assert_eq!(events[2].code, crossterm::event::KeyCode::Char('s'));
        assert_eq!(events[3].code, crossterm::event::KeyCode::Enter);
    }
}
