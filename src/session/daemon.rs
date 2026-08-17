//! The session daemon — owns each session's PTYs and multiplexes raw bytes
//! between them and attached clients over a Unix socket. Ported from TUIOS
//! `internal/session/daemon*.go`.
//!
//! The daemon never parses VT or renders: it only spawns shells and forwards
//! input/output, so the client keeps running its own emulator and renderer.
//!
//! Each session has a *broadcast hub*: every live window pumps its PTY output
//! into the hub, which fans it out to every currently-attached client. This is
//! what lets several `attach` clients (or the remote TUI and a `list` shell)
//! watch the same session at once.

use std::collections::HashMap;
use std::io;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crossbeam_channel::{unbounded, Receiver, RecvTimeoutError, Sender};

use crate::terminal::pty::{spawn_pty, PtyHandle, PtyWriter, WinSize};

use super::manager::Manager;
use super::model::{Session, SessionConfig, WindowInfo, WindowState};
use super::persistence;
use super::protocol::{self, Message, VERSION};

/// The default Unix socket path.
pub fn default_socket_path() -> PathBuf {
    if let Ok(p) = std::env::var("TUIOS_SOCKET") {
        return PathBuf::from(p);
    }
    if let Some(dir) = dirs::runtime_dir() {
        let d = dir.join("tuios");
        let _ = std::fs::create_dir_all(&d);
        return d.join("tuios.sock");
    }
    let uid = unsafe { nix::libc::getuid() };
    PathBuf::from(format!("/tmp/tuios-{uid}.sock"))
}

/// A live window: the PTY plus its metadata.
struct LiveWindow {
    info: WindowInfo,
    writer: PtyWriter,
    // Kept alive so the child is SIGHUP'd and reaped when the window closes.
    _handle: PtyHandle,
    shell: String,
}

/// One session's fan-out point. Each attached client registers a `Sender`;
/// window pumps and window-lifecycle events write to every registered sender.
struct SessionBroadcast {
    subscribers: Mutex<HashMap<u64, Sender<Message>>>,
    next_id: AtomicU64,
}

impl SessionBroadcast {
    fn new() -> Self {
        Self {
            subscribers: Mutex::new(HashMap::new()),
            next_id: AtomicU64::new(0),
        }
    }

    fn subscribe(&self) -> (u64, Receiver<Message>) {
        let (tx, rx) = unbounded();
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        self.subscribers.lock().unwrap().insert(id, tx);
        (id, rx)
    }

    fn unsubscribe(&self, id: u64) {
        self.subscribers.lock().unwrap().remove(&id);
    }

    fn is_attached(&self) -> bool {
        !self.subscribers.lock().unwrap().is_empty()
    }

    fn send_to_all(&self, msg: &Message) {
        let subs: Vec<Sender<Message>> = self
            .subscribers
            .lock()
            .unwrap()
            .values()
            .cloned()
            .collect();
        for tx in subs {
            let _ = tx.send(msg.clone());
        }
    }
}

/// The daemon: a session registry plus each session's live windows and
/// broadcast hubs.
pub struct Daemon {
    manager: Manager,
    windows: Mutex<HashMap<String, Vec<LiveWindow>>>,
    broadcast: Mutex<HashMap<String, Arc<SessionBroadcast>>>,
    /// Lifecycle hooks fired daemon-side for the window/session events the
    /// daemon owns (authoritative for daemon-mode windows).
    hook_manager: crate::hooks::Manager,
}

impl Daemon {
    pub fn new() -> Self {
        Self {
            manager: Manager::new(),
            windows: Mutex::new(HashMap::new()),
            broadcast: Mutex::new(HashMap::new()),
            hook_manager: crate::hooks::Manager::new(),
        }
    }

    /// Load hooks from the `[hooks]` config section (called by the CLI before
    /// the daemon accepts clients; kept separate so tests start hook-free).
    pub fn load_hooks(&self, hook_config: &std::collections::HashMap<String, toml::Value>) {
        self.hook_manager.load_from_config(hook_config);
    }

    /// Test seam: replace the hook command runner.
    pub fn set_hook_runner<F>(&self, run: F)
    where
        F: Fn(&str, &crate::hooks::Context) + Send + Sync + 'static,
    {
        self.hook_manager.set_runner(run);
    }

    /// Fire a hook with the session name in the context.
    fn fire_hook(&self, event: crate::hooks::Event, session: &str, mut ctx: crate::hooks::Context) {
        ctx.session_id = session.to_string();
        self.hook_manager.fire(event, ctx);
    }

    /// The session list with window counts and attach state.
    pub fn list_infos(&self) -> Vec<super::model::SessionInfo> {
        let windows = self.windows.lock().unwrap();
        let broadcast = self.broadcast.lock().unwrap();
        self.manager
            .list()
            .into_iter()
            .map(|s| super::manager::info_for(
                &s,
                windows.get(&s.name).map(|w| w.len()).unwrap_or(0),
                broadcast
                    .get(&s.name)
                    .map(|b| b.is_attached())
                    .unwrap_or(false),
            ))
            .collect()
    }

    /// Get (or create) a session's broadcast hub.
    fn broadcast_for(&self, name: &str) -> Arc<SessionBroadcast> {
        self.broadcast
            .lock()
            .unwrap()
            .entry(name.to_string())
            .or_insert_with(|| Arc::new(SessionBroadcast::new()))
            .clone()
    }

    /// Spawn the first window of a new session and register it.
    pub fn create_session(&self, name: &str, shell: &str) -> Result<Session, String> {
        let cfg = SessionConfig {
            shell: resolve_shell(shell),
            cwd: None,
        };
        let session = self
            .manager
            .create(name, &cfg)
            .map_err(|e| e.to_string())?;
        let broadcast = self.broadcast_for(name);
        let window = self.spawn_window("w0", "Terminal", 1, &cfg.shell, &broadcast)?;
        self.fire_hook(
            crate::hooks::Event::AfterNewWindow,
            name,
            crate::hooks::Context {
                window_id: "w0".into(),
                window_name: "Terminal".into(),
                workspace: 1,
                ..crate::hooks::Context::default()
            },
        );
        self.windows
            .lock()
            .unwrap()
            .insert(name.to_string(), vec![window]);
        self.save_session(name);
        Ok(session)
    }

    /// Respawn a session's windows from saved state at daemon start.
    fn restore_session(&self, name: &str, state: &super::model::SessionState) -> Result<(), String> {
        self.manager.restore(name).map_err(|e| e.to_string())?;
        let broadcast = self.broadcast_for(name);
        let mut wins = Vec::new();
        for (i, w) in state.windows.iter().enumerate() {
            let id = format!("w{i}");
            match self.spawn_window(&id, &w.title, w.workspace, &w.shell, &broadcast) {
                Ok(live) => wins.push(live),
                Err(e) => log::warn!("failed to respawn window '{id}' in session '{name}': {e}"),
            }
        }
        self.windows.lock().unwrap().insert(name.to_string(), wins);
        Ok(())
    }

    fn spawn_window(
        &self,
        id: &str,
        title: &str,
        workspace: i32,
        shell: &str,
        broadcast: &Arc<SessionBroadcast>,
    ) -> Result<LiveWindow, String> {
        let size = WinSize { cols: 80, rows: 24 };
        let argv = vec![shell.to_string()];
        let (writer, handle, reader) =
            spawn_pty(size, &argv, Box::new(|| {})).map_err(|e| e.to_string())?;
        // Pump this window's PTY output into the session's broadcast hub.
        let pump_broadcast = Arc::clone(broadcast);
        let pump_id = id.to_string();
        std::thread::spawn(move || pump(reader.rx, pump_broadcast, pump_id));
        Ok(LiveWindow {
            info: WindowInfo {
                id: id.to_string(),
                title: title.to_string(),
                workspace,
                cols: size.cols,
                rows: size.rows,
            },
            writer,
            _handle: handle,
            shell: shell.to_string(),
        })
    }

    /// Attach a client to a session: register a subscriber and return the
    /// window list, the subscriber id, and the output channel.
    fn attach(&self, name: &str) -> Result<(Vec<WindowInfo>, u64, Receiver<Message>), String> {
        let windows = self.windows.lock().unwrap();
        let sess = windows
            .get(name)
            .ok_or_else(|| format!("session '{name}' not found"))?;
        let infos: Vec<WindowInfo> = sess.iter().map(|w| w.info.clone()).collect();
        drop(windows);
        let broadcast = self
            .broadcast
            .lock()
            .unwrap()
            .get(name)
            .cloned()
            .ok_or_else(|| format!("session '{name}' not found"))?;
        let (sub_id, rx) = broadcast.subscribe();
        Ok((infos, sub_id, rx))
    }

    fn detach(&self, name: &str, sub_id: u64) {
        if let Some(b) = self.broadcast.lock().unwrap().get(name) {
            b.unsubscribe(sub_id);
        }
    }

    /// Kill a session: stop its PTYs, drop state, remove its save file.
    fn kill(&self, name: &str) -> Result<(), String> {
        self.manager.delete(name).map_err(|e| e.to_string())?;
        self.windows.lock().unwrap().remove(name);
        self.broadcast.lock().unwrap().remove(name);
        persistence::remove(name);
        Ok(())
    }

    fn write_input(&self, window: &str, data: &[u8]) {
        for wins in self.windows.lock().unwrap().values() {
            if let Some(w) = wins.iter().find(|w| w.info.id == window) {
                w.writer.write(data);
                return;
            }
        }
    }

    fn resize(&self, window: &str, cols: u16, rows: u16) {
        for wins in self.windows.lock().unwrap().values() {
            if let Some(w) = wins.iter().find(|w| w.info.id == window) {
                w.writer.resize(WinSize { cols, rows });
                return;
            }
        }
    }

    fn add_window(&self, session: &str, shell: &str, workspace: i32) -> Result<WindowInfo, String> {
        let mut windows = self.windows.lock().unwrap();
        let wins = windows
            .get_mut(session)
            .ok_or_else(|| format!("session '{session}' not found"))?;
        let id = format!("w{}", wins.len());
        let shell = resolve_shell(shell);
        let broadcast = self
            .broadcast
            .lock()
            .unwrap()
            .get(session)
            .cloned()
            .ok_or_else(|| format!("session '{session}' not found"))?;
        let live = self.spawn_window(&id, "Terminal", workspace, &shell, &broadcast)?;
        let info = live.info.clone();
        wins.push(live);
        drop(windows);
        self.save_session(session);
        self.fire_hook(
            crate::hooks::Event::AfterNewWindow,
            session,
            crate::hooks::Context {
                window_id: info.id.clone(),
                window_name: info.title.clone(),
                workspace: info.workspace,
                ..crate::hooks::Context::default()
            },
        );
        self.broadcast_event(session, &Message::WindowAdded { window: info.clone() });
        Ok(info)
    }

    fn close_window(&self, session: &str, window: &str) -> Result<(), String> {
        let mut windows = self.windows.lock().unwrap();
        let wins = windows
            .get_mut(session)
            .ok_or_else(|| format!("session '{session}' not found"))?;
        let before = wins.len();
        let closed: Option<WindowInfo> = wins
            .iter()
            .find(|w| w.info.id == window)
            .map(|w| w.info.clone());
        wins.retain(|w| w.info.id != window);
        if wins.len() == before {
            return Err(format!("window '{window}' not found"));
        }
        drop(windows);
        self.save_session(session);
        if let Some(info) = closed {
            self.fire_hook(
                crate::hooks::Event::AfterCloseWindow,
                session,
                crate::hooks::Context {
                    window_id: info.id.clone(),
                    window_name: info.title.clone(),
                    workspace: info.workspace,
                    ..crate::hooks::Context::default()
                },
            );
        }
        self.broadcast_event(session, &Message::WindowClosed { window: window.to_string() });
        Ok(())
    }

    /// Send an event to every client attached to a session.
    fn broadcast_event(&self, session: &str, msg: &Message) {
        if let Some(b) = self.broadcast.lock().unwrap().get(session) {
            b.send_to_all(msg);
        }
    }

    /// Persist a session's window definitions for resurrection.
    fn save_session(&self, name: &str) {
        let windows = self.windows.lock().unwrap();
        let Some(wins) = windows.get(name) else {
            return;
        };
        let states: Vec<WindowState> = wins
            .iter()
            .map(|w| WindowState {
                title: w.info.title.clone(),
                shell: w.shell.clone(),
                workspace: w.info.workspace,
            })
            .collect();
        let _ = persistence::save(name, &states);
    }

    /// Restore all saved sessions at startup (called by the CLI before run).
    pub fn restore_saved(&self) {
        for state in persistence::list_saved() {
            if let Err(e) = self.restore_session(&state.name, &state) {
                log::warn!("failed to restore session '{}': {}", state.name, e);
            }
        }
    }

    /// Run the daemon forever on the default socket path.
    pub fn run_default(self: Arc<Self>) -> io::Result<()> {
        self.run(&default_socket_path())
    }

    /// Run the daemon forever on a given socket path: bind the socket and
    /// accept clients. Returns only on a bind failure.
    pub fn run(self: Arc<Self>, path: &std::path::Path) -> io::Result<()> {
        if path.exists() {
            let _ = std::fs::remove_file(path);
        }
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let listener = UnixListener::bind(path)?;
        log::info!("tuios daemon listening on {}", path.display());

        for stream in listener.incoming() {
            match stream {
                Ok(stream) => {
                    let daemon = Arc::clone(&self);
                    std::thread::spawn(move || handle_client(stream, daemon));
                }
                Err(e) => log::warn!("accept error: {e}"),
            }
        }
        Ok(())
    }
}

impl Default for Daemon {
    fn default() -> Self {
        Self::new()
    }
}

/// Handle one client connection: read requests, dispatch, and stream the
/// attached session's PTY output back.
///
/// Reads and writes use separate handles so the request loop can block on a
/// read without holding the write lock that the broadcast-forwarding threads
/// need.
fn handle_client(stream: UnixStream, daemon: Arc<Daemon>) {
    let mut reader = match stream.try_clone() {
        Ok(r) => r,
        Err(_) => return,
    };
    let writer = Arc::new(Mutex::new(stream));
    // (session name, subscriber id, stop flag for the forward thread).
    let mut attached: Option<(String, u64, Arc<AtomicBool>)> = None;

    while let Ok(msg) = protocol::read_message(&mut reader) {
        match msg {
            Message::Hello { .. } => {
                let sessions = daemon.list_infos();
                let _ = send(&writer, &Message::Welcome {
                    version: VERSION.to_string(),
                    sessions,
                });
            }
            Message::List => {
                let sessions = daemon.list_infos();
                let _ = send(&writer, &Message::ListResult { sessions });
            }
            Message::New { name, shell } => match daemon.create_session(&name, &shell) {
                Ok(_) => {
                    let sessions = daemon.list_infos();
                    let _ = send(&writer, &Message::ListResult { sessions });
                }
                Err(e) => {
                    let _ = send(&writer, &Message::Error { message: e });
                }
            },
            Message::Attach { name } => {
                // Stop any previous streaming for this connection.
                if let Some((prev, sub_id, stop)) = attached.take() {
                    stop.store(true, Ordering::Release);
                    daemon.detach(&prev, sub_id);
                }
                match daemon.attach(&name) {
                    Ok((windows, sub_id, rx)) => {
                        let stop = Arc::new(AtomicBool::new(false));
                        attached = Some((name.clone(), sub_id, Arc::clone(&stop)));
                        let _ = send(&writer, &Message::Attached { windows });
                        // Forward the session's broadcast stream to this client.
                        let forward_writer = Arc::clone(&writer);
                        std::thread::spawn(move || {
                            while !stop.load(Ordering::Acquire) {
                                match rx.recv_timeout(Duration::from_millis(100)) {
                                    Ok(msg) => {
                                        if send(&forward_writer, &msg).is_err() {
                                            break;
                                        }
                                    }
                                    Err(RecvTimeoutError::Timeout) => continue,
                                    Err(RecvTimeoutError::Disconnected) => break,
                                }
                            }
                        });
                    }
                    Err(e) => {
                        let _ = send(&writer, &Message::Error { message: e });
                    }
                }
            }
            Message::Detach => {
                if let Some((name, sub_id, stop)) = attached.take() {
                    stop.store(true, Ordering::Release);
                    daemon.detach(&name, sub_id);
                }
            }
            Message::Kill { name } => {
                if let Err(e) = daemon.kill(&name) {
                    let _ = send(&writer, &Message::Error { message: e });
                } else {
                    let sessions = daemon.list_infos();
                    let _ = send(&writer, &Message::ListResult { sessions });
                }
            }
            Message::NewWindow { shell, workspace } => {
                if let Some((session, _, _)) = &attached {
                    // On success the window is announced to all subscribers by
                    // `add_window`; errors go straight back to this client.
                    if let Err(e) = daemon.add_window(session, &shell, workspace) {
                        let _ = send(&writer, &Message::Error { message: e });
                    }
                }
            }
            Message::CloseWindow { window } => {
                if let Some((session, _, _)) = &attached {
                    if let Err(e) = daemon.close_window(session, &window) {
                        let _ = send(&writer, &Message::Error { message: e });
                    }
                }
            }
            Message::Input { window, data } => daemon.write_input(&window, &data),
            Message::Resize {
                window,
                cols,
                rows,
            } => daemon.resize(&window, cols, rows),
            _ => {}
        }
    }

    // Cleanup on disconnect.
    if let Some((name, sub_id, stop)) = attached {
        stop.store(true, Ordering::Release);
        daemon.detach(&name, sub_id);
    }
}

/// Drain a window's PTY output channel and fan each chunk out to every client
/// attached to its session. When the shell exits (channel closes), announce
/// the close.
fn pump(rx: Receiver<Vec<u8>>, broadcast: Arc<SessionBroadcast>, window: String) {
    while let Ok(chunk) = rx.recv() {
        broadcast.send_to_all(&Message::PtyOutput {
            window: window.clone(),
            data: chunk,
        });
    }
    broadcast.send_to_all(&Message::PtyClosed { window });
}

fn send(stream: &Arc<Mutex<UnixStream>>, msg: &Message) -> io::Result<()> {
    let mut s = stream.lock().unwrap();
    protocol::write_message(&mut *s, msg)
}

fn resolve_shell(shell: &str) -> String {
    if shell.is_empty() {
        std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string())
    } else {
        shell.to_string()
    }
}

/// Spawn a background daemon if one is not already reachable, then wait for
/// its socket to come up.
pub fn ensure_daemon_running() -> io::Result<()> {
    let path = default_socket_path();
    if UnixStream::connect(&path).is_ok() {
        return Ok(());
    }
    let exe = std::env::current_exe()?;
    let _child = std::process::Command::new(exe)
        .arg("daemon")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .stdin(std::process::Stdio::null())
        .spawn()?;
    for _ in 0..40 {
        if UnixStream::connect(&path).is_ok() {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    Err(io::Error::new(
        io::ErrorKind::ConnectionRefused,
        "daemon did not start",
    ))
}
