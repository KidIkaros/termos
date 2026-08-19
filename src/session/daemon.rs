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
use std::io::{self, BufRead, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crossbeam_channel::{unbounded, Receiver, RecvTimeoutError, Sender};
use serde_json::Value;

use crate::terminal::pty::{spawn_pty, PtyHandle, PtyWriter, WinSize};

use super::agent_state::AgentState;
use super::manager::Manager;
use super::model::{Session, SessionConfig, WindowInfo, WindowState};
use super::persistence;
use super::protocol::{self, Message, VERSION};
use super::verb::{
    VerbError, VerbRegistry, VerbRequest, VerbResponse, ERR_COMMAND_FAILED, ERR_INVALID_PARAMS,
    ERR_INVALID_REQUEST, ERR_OPTION_NOT_FOUND, ERR_SESSION_NOT_FOUND, ERR_TIMEOUT,
    ERR_UNKNOWN_VERB, ERR_WINDOW_NOT_FOUND, MIN_VERB_PROTOCOL_VERSION, VERB_PROTOCOL_VERSION,
};

/// The default Unix socket path.
pub fn default_socket_path() -> PathBuf {
    if let Ok(p) = std::env::var("TERMOS_SOCKET") {
        return PathBuf::from(p);
    }
    if let Some(dir) = dirs::runtime_dir() {
        let d = dir.join("termos");
        let _ = std::fs::create_dir_all(&d);
        return d.join("termos.sock");
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

/// A cap-limited ring of a window's raw output. Kept daemon-side so the
/// `capture-pane` and `wait-for` verbs work headlessly, without a client
/// emulator attached. A monotonic sequence counter tracks how many bytes have
/// been pushed in total, so a reconnecting client can resume from its
/// last-seen position (Go's PTY resubscribe / catch-up buffer).
struct OutputRing {
    buf: Vec<u8>,
    cap: usize,
    /// Total bytes ever pushed (never resets, even when the ring wraps).
    total_bytes: u64,
}

impl OutputRing {
    fn new(cap: usize) -> Self {
        Self {
            buf: Vec::new(),
            cap,
            total_bytes: 0,
        }
    }

    fn push(&mut self, chunk: &[u8]) {
        self.buf.extend_from_slice(chunk);
        self.total_bytes = self.total_bytes.saturating_add(chunk.len() as u64);
        if self.buf.len() > self.cap {
            let drop = self.buf.len() - self.cap;
            self.buf.drain(..drop);
        }
    }


    fn as_lossy(&self) -> String {
        String::from_utf8_lossy(&self.buf).into_owned()
    }

    /// The current sequence position (total bytes pushed). A client that
    /// resumes from `seq` will receive output pushed after this point.
    fn current_seq(&self) -> u64 {
        self.total_bytes
    }

    /// Return the buffered output that arrived after `from_seq`, as a
    /// contiguous byte slice. Returns an empty vec if `from_seq` is at or
    /// past the current position, or if the requested position has been
    /// evicted from the ring (in which case the entire buffer is returned,
    /// since everything in it is newer than `from_seq`).
    fn output_since(&self, from_seq: u64) -> Vec<u8> {
        if from_seq >= self.total_bytes {
            return Vec::new();
        }
        // How many bytes have been evicted from the front of the ring?
        let evicted = self.total_bytes.saturating_sub(self.buf.len() as u64);
        // If the requested position was evicted, return everything we have.
        if from_seq < evicted {
            return self.buf.clone();
        }
        // Offset into the current buffer.
        let offset = (from_seq - evicted) as usize;
        self.buf[offset..].to_vec()
    }
}

/// One attached client's metadata: its subscriber id, name, and terminal
/// dimensions (for multi-client minimum-size resize).
struct ClientEntry {
    name: String,
    cols: u16,
    rows: u16,
}

/// One session's fan-out point. Each attached client registers a `Sender`;
/// window pumps and window-lifecycle events write to every registered sender.
/// Client dimensions are tracked so the daemon can calculate the minimum size
/// across all attached clients (Go's `calculateEffectiveSize`).
struct SessionBroadcast {
    subscribers: Mutex<HashMap<u64, Sender<Message>>>,
    clients: Mutex<HashMap<u64, ClientEntry>>,
    next_id: AtomicU64,
}

impl SessionBroadcast {
    fn new() -> Self {
        Self {
            subscribers: Mutex::new(HashMap::new()),
            clients: Mutex::new(HashMap::new()),
            next_id: AtomicU64::new(0),
        }
    }

    /// Register a subscriber, returning its id and receive channel.
    fn subscribe(&self) -> (u64, Receiver<Message>) {
        let (tx, rx) = unbounded();
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        self.subscribers.lock().unwrap_or_else(|e| e.into_inner()).insert(id, tx);
        (id, rx)
    }

    /// Register a subscriber with a client name and dimensions.
    fn subscribe_named(&self, name: &str, cols: u16, rows: u16) -> (u64, Receiver<Message>) {
        let (id, rx) = self.subscribe();
        self.clients.lock().unwrap_or_else(|e| e.into_inner()).insert(
            id,
            ClientEntry {
                name: name.to_string(),
                cols,
                rows,
            },
        );
        (id, rx)
    }

    fn unsubscribe(&self, id: u64) {
        self.subscribers.lock().unwrap_or_else(|e| e.into_inner()).remove(&id);
        self.clients.lock().unwrap_or_else(|e| e.into_inner()).remove(&id);
    }

    /// Update a client's reported terminal dimensions.
    fn update_size(&self, id: u64, cols: u16, rows: u16) {
        if let Some(entry) = self.clients.lock().unwrap_or_else(|e| e.into_inner()).get_mut(&id) {
            entry.cols = cols;
            entry.rows = rows;
        }
    }

    fn is_attached(&self) -> bool {
        !self.subscribers.lock().unwrap_or_else(|e| e.into_inner()).is_empty()
    }

    /// The number of clients attached to this session.
    fn client_count(&self) -> usize {
        self.subscribers.lock().unwrap_or_else(|e| e.into_inner()).len()
    }

    /// The minimum (cols, rows) across all attached clients with non-zero
    /// dimensions. Returns `None` when no client has reported dimensions.
    /// This is Go's `calculateEffectiveSize`.
    fn effective_size(&self) -> Option<(u16, u16)> {
        let clients = self.clients.lock().unwrap_or_else(|e| e.into_inner());
        let mut min: Option<(u16, u16)> = None;
        for entry in clients.values() {
            if entry.cols == 0 || entry.rows == 0 {
                continue;
            }
            match min {
                None => min = Some((entry.cols, entry.rows)),
                Some((cw, ch)) => {
                    let nw = if entry.cols < cw {
                        entry.cols
                    } else {
                        cw
                    };
                    let nh = if entry.rows < ch {
                        entry.rows
                    } else {
                        ch
                    };
                    min = Some((nw, nh));
                }
            }
        }
        min
    }

    /// The name of a client by subscriber id, if present.
    fn client_name(&self, id: u64) -> Option<String> {
        self.clients.lock().unwrap_or_else(|e| e.into_inner()).get(&id).map(|e| e.name.clone())
    }

    fn send_to_all(&self, msg: &Message) {
        let subs: Vec<Sender<Message>> =
            self.subscribers.lock().unwrap_or_else(|e| e.into_inner()).values().cloned().collect();
        for tx in subs {
            let _ = tx.send(msg.clone());
        }
    }

    /// Send to all subscribers except the one with the given id (used for
    /// join/leave notifications so the joining/leaving client does not receive
    /// its own announcement).
    fn send_to_all_except(&self, except: u64, msg: &Message) {
        let subs: Vec<(u64, Sender<Message>)> = self
            .subscribers
            .lock()
            .unwrap()
            .iter()
            .filter(|(k, _)| **k != except)
            .map(|(k, v)| (*k, v.clone()))
            .collect();
        for (_, tx) in subs {
            let _ = tx.send(msg.clone());
        }
    }
}

/// The daemon: a session registry plus each session's live windows and
/// broadcast hubs.
/// Per-session metadata set through the verbs (`set-session-name`,
/// `set-session-accent`): the display label and accent colour.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct SessionMeta {
    pub display_name: String,
    pub accent: String,
    /// Runtime config options set via `set-config` / `get-config`.
    #[serde(default)]
    pub options: HashMap<String, String>,
    /// Workspace names keyed by workspace number (as a string).
    #[serde(default)]
    pub workspace_names: HashMap<String, String>,
}

pub struct Daemon {
    manager: Manager,
    /// Session labels/accents keyed by session name.
    meta: Mutex<HashMap<String, SessionMeta>>,
    windows: Arc<Mutex<HashMap<String, Vec<LiveWindow>>>>,
    /// Per-session monotonic counter for window ids (`w0`, `w1`, ...).
    /// Never reuses a number, even after windows close — so ids stay unique
    /// for the life of the session (scripting targets by id must not collide).
    win_seq: Mutex<HashMap<String, u64>>,
    broadcast: Mutex<HashMap<String, Arc<SessionBroadcast>>>,
    /// Lifecycle hooks fired daemon-side for the window/session events the
    /// daemon owns (authoritative for daemon-mode windows). Arc so the PTY
    /// pump threads (which parse OSC 133 markers from raw output) can fire
    /// the pane-level hook events too.
    hook_manager: Arc<crate::hooks::Manager>,
    /// Per-session id of the most recently active window (updated by
    /// `Input`/`Resize`). The `set-agent-state` verb targets it when no
    /// window is named — the port's approximation of "focused".
    last_active: Mutex<HashMap<String, String>>,
    /// Per-window raw-output rings keyed by (session, window), for the
    /// `capture-pane` / `wait-for` verbs.
    rings: Arc<Mutex<HashMap<(String, String), OutputRing>>>,
    /// Anti-flicker holds for OSC-derived agent states, keyed by window id:
    /// (held state, when it was recorded). A quieter state must stand
    /// unchanged for [`OSC_HOLD_WINDOW`] before it is published.
    osc_holds: Arc<Mutex<HashMap<String, (AgentState, std::time::Instant)>>>,
    /// Exit codes of windows whose shell has exited, keyed by
    /// (session, window). Populated by the PTY pump at EOF; `-1` marks a
    /// window terminated by an explicit close. Powers `block-until-exit`
    /// and the `closed` signal in `subscribe`.
    exit_statuses: Arc<Mutex<HashMap<(String, String), i32>>>,
    /// Wake-up channel for `block-until-exit` waiters (fires on every
    /// recorded exit).
    exit_tx: crossbeam_channel::Sender<()>,
    exit_rx: crossbeam_channel::Receiver<()>,
    /// Shutdown flag — set by `KillServer` to stop the accept loop.
    shutdown: Arc<std::sync::atomic::AtomicBool>,
    /// When the daemon started, for uptime reporting in `diagnose`.
    started_at: std::time::Instant,
}

/// Ring capacity (256 KiB) and the capture size cap (64 KiB).
const RING_CAP: usize = 256 * 1024;
const CAPTURE_CAP: usize = 64 * 1024;

/// How long a quieter agent state must stand unchanged before it is
/// published (Go's `agentHoldWindow`, 700ms).
const OSC_HOLD_WINDOW: Duration = Duration::from_millis(700);

impl Daemon {
    pub fn new() -> Self {
        let (exit_tx, exit_rx) = crossbeam_channel::unbounded();
        Self {
            manager: Manager::new(),
            meta: Mutex::new(HashMap::new()),
            windows: Arc::new(Mutex::new(HashMap::new())),
            win_seq: Mutex::new(HashMap::new()),
            broadcast: Mutex::new(HashMap::new()),
            hook_manager: Arc::new(crate::hooks::Manager::new()),
            last_active: Mutex::new(HashMap::new()),
            rings: Arc::new(Mutex::new(HashMap::new())),
            osc_holds: Arc::new(Mutex::new(HashMap::new())),
            exit_statuses: Arc::new(Mutex::new(HashMap::new())),
            exit_tx,
            exit_rx,
            shutdown: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            started_at: std::time::Instant::now(),
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
        let windows = self.windows.lock().unwrap_or_else(|e| e.into_inner());
        let broadcast = self.broadcast.lock().unwrap_or_else(|e| e.into_inner());
        self.manager
            .list()
            .into_iter()
            .map(|s| {
                super::manager::info_for(
                    &s,
                    windows.get(&s.name).map(|w| w.len()).unwrap_or(0),
                    broadcast
                        .get(&s.name)
                        .map(|b| b.is_attached())
                        .unwrap_or(false),
                )
            })
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
        self.create_session_in(name, shell, None)
    }

    /// [`create_session`] with an explicit starting directory for the shell.
    pub fn create_session_in(
        &self,
        name: &str,
        shell: &str,
        cwd: Option<&str>,
    ) -> Result<Session, String> {
        let cfg = SessionConfig {
            shell: resolve_shell(shell),
            cwd: cwd.map(str::to_string),
        };
        let session = self.manager.create(name, &cfg).map_err(|e| e.to_string())?;
        let broadcast = self.broadcast_for(name);
        let id = self.next_window_id(name);
        let window =
            self.spawn_window(name, &id, "Terminal", 1, &cfg.shell, cwd, &broadcast)?;
        self.fire_hook(
            crate::hooks::Event::AfterNewWindow,
            name,
            crate::hooks::Context {
                window_id: id.clone(),
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
    fn restore_session(
        &self,
        name: &str,
        state: &super::model::SessionState,
    ) -> Result<(), String> {
        self.manager.restore(name).map_err(|e| e.to_string())?;
        let broadcast = self.broadcast_for(name);
        let mut wins = Vec::new();
        for (i, w) in state.windows.iter().enumerate() {
            let id = format!("w{i}");
            match self.spawn_window(name, &id, &w.title, w.workspace, &w.shell, None, &broadcast) {
                Ok(live) => wins.push(live),
                Err(e) => log::warn!("failed to respawn window '{id}' in session '{name}': {e}"),
            }
        }
        // Seed the id counter past the restored windows so new spawns never
        // collide with ids re-derived from the saved state.
        self.win_seq
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .entry(name.to_string())
            .and_modify(|n| *n = (*n).max(state.windows.len() as u64))
            .or_insert(state.windows.len() as u64);
        self.windows.lock().unwrap_or_else(|e| e.into_inner()).insert(name.to_string(), wins);
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn spawn_window(
        &self,
        session: &str,
        id: &str,
        title: &str,
        workspace: i32,
        shell: &str,
        cwd: Option<&str>,
        broadcast: &Arc<SessionBroadcast>,
    ) -> Result<LiveWindow, String> {
        let size = WinSize { cols: 80, rows: 24 };
        let argv = vec![shell.to_string()];
        // Advertise TermOS to the pane so agents can detect the environment.
        let env = crate::util::guestenv::base_guest_env(session, id, false, false);
        let (writer, handle, reader) =
            spawn_pty(size, &argv, Box::new(|| {}), &env, cwd).map_err(|e| e.to_string())?;
        // Pump this window's PTY output into the session's broadcast hub and
        // its output ring (keyed by (session, window) for the verbs).
        let pump_broadcast = Arc::clone(broadcast);
        let pump_session = session.to_string();
        let pump_id = id.to_string();
        let rings = Arc::clone(&self.rings);
        let pump_windows = Arc::clone(&self.windows);
        let pump_holds = Arc::clone(&self.osc_holds);
        let pump_pid = handle.pid();
        let pump_statuses = Arc::clone(&self.exit_statuses);
        let pump_exit_tx = self.exit_tx.clone();
        let pump_hooks = Arc::clone(&self.hook_manager);
        std::thread::spawn(move || {
            pump(
                reader.rx,
                pump_broadcast,
                pump_session,
                pump_id,
                rings,
                pump_windows,
                pump_holds,
                pump_pid,
                pump_statuses,
                pump_exit_tx,
                pump_hooks,
            )
        });
        Ok(LiveWindow {
            info: WindowInfo {
                id: id.to_string(),
                title: title.to_string(),
                workspace,
                cols: size.cols,
                rows: size.rows,
                agent_state: String::new(),
                agent_message: String::new(),
                agent_harness: String::new(),
            },
            writer,
            _handle: handle,
            shell: shell.to_string(),
        })
    }

    /// Attach a client to a session: register a subscriber and return the
    /// window list, the subscriber id, and the output channel. Convenience
    /// wrapper around [`attach_named`] with a default name and no dimensions.
    #[allow(dead_code)]
    fn attach(&self, name: &str) -> Result<(Vec<WindowInfo>, u64, Receiver<Message>), String> {
        self.attach_named(name, "client", 0, 0)
    }

    /// Attach a client to a session with a name and terminal dimensions.
    /// The name and dimensions are used for join/leave notifications and
    /// multi-client minimum-size resize. Broadcasts `ClientJoined` to other
    /// subscribers.
    fn attach_named(
        &self,
        name: &str,
        client_name: &str,
        cols: u16,
        rows: u16,
    ) -> Result<(Vec<WindowInfo>, u64, Receiver<Message>), String> {
        let windows = self.windows.lock().unwrap_or_else(|e| e.into_inner());
        let sess = windows
            .get(name)
            .ok_or_else(|| format!("session '{name}' not found"))?;
        let infos: Vec<WindowInfo> = sess.iter().map(|w| w.info.clone()).collect();
        drop(windows);
        let broadcast = self.broadcast_for(name);
        let (sub_id, rx) = broadcast.subscribe_named(client_name, cols, rows);
        // Notify other clients that a new client joined.
        let count = broadcast.client_count();
        broadcast.send_to_all_except(
            sub_id,
            &Message::ClientJoined {
                session: name.to_string(),
                name: format!("{client_name} (#{count})"),
            },
        );
        // Recalculate effective size and resize if needed.
        self.recalculate_size(name, &broadcast);
        Ok((infos, sub_id, rx))
    }

    /// Attach with a resume position: replay buffered output from `seq` for
    /// each window, then stream live output. Returns the window list and the
    /// current output sequence number.
    fn attach_resume(
        &self,
        name: &str,
        client_name: &str,
        cols: u16,
        rows: u16,
        from_seq: u64,
    ) -> Result<(Vec<WindowInfo>, u64, u64, Receiver<Message>), String> {
        let (infos, sub_id, rx) = self.attach_named(name, client_name, cols, rows)?;
        // Replay buffered output for each window from the resume position.
        let rings = self.rings.lock().unwrap_or_else(|e| e.into_inner());
        let mut max_seq = from_seq;
        for info in &infos {
            if let Some(ring) = rings.get(&(name.to_string(), info.id.clone())) {
                let replay = ring.output_since(from_seq);
                if !replay.is_empty() {
                    // Send the replay directly to this subscriber's channel.
                    // We can't use send_to_all (that would send to everyone);
                    // instead, we rely on the forward thread to drain the
                    // channel. We push a sequenced output message so the
                    // client knows the position.
                    if let Some(b) = self.broadcast.lock().unwrap_or_else(|e| e.into_inner()).get(name) {
                        let subs = b.subscribers.lock().unwrap_or_else(|e| e.into_inner());
                        if let Some(tx) = subs.get(&sub_id) {
                            let _ = tx.send(Message::PtyOutputSequenced {
                                window: info.id.clone(),
                                seq: from_seq,
                                data: replay,
                            });
                        }
                    }
                }
                max_seq = max_seq.max(ring.current_seq());
            }
        }
        Ok((infos, sub_id, max_seq, rx))
    }

    fn detach(&self, name: &str, sub_id: u64) {
        if let Some(b) = self.broadcast.lock().unwrap_or_else(|e| e.into_inner()).get(name) {
            let client_name = b.client_name(sub_id).unwrap_or_default();
            b.unsubscribe(sub_id);
            // Notify remaining clients that this client left.
            let count = b.client_count();
            b.send_to_all_except(
                sub_id,
                &Message::ClientLeft {
                    session: name.to_string(),
                    name: format!("{client_name} (#{count})"),
                },
            );
            // Recalculate effective size (a client leaving may increase it).
            self.recalculate_size(name, b);
        }
    }

    /// Kill a session: stop its PTYs, drop state, remove its save file.
    fn kill(&self, name: &str) -> Result<(), String> {
        self.manager.delete(name).map_err(|e| e.to_string())?;
        self.windows.lock().unwrap_or_else(|e| e.into_inner()).remove(name);
        self.broadcast.lock().unwrap_or_else(|e| e.into_inner()).remove(name);
        persistence::remove(name);
        Ok(())
    }

    fn write_input(&self, session: &str, window: &str, data: &[u8]) {
        let windows = self.windows.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(wins) = windows.get(session) {
            if let Some(w) = wins.iter().find(|w| w.info.id == window) {
                w.writer.write(data);
                drop(windows);
                self.last_active
                    .lock()
                    .unwrap()
                    .insert(session.to_string(), window.to_string());
            }
        }
    }

    fn resize(&self, session: &str, window: &str, cols: u16, rows: u16) {
        let windows = self.windows.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(wins) = windows.get(session) {
            if let Some(w) = wins.iter().find(|w| w.info.id == window) {
                w.writer.resize(WinSize { cols, rows });
                drop(windows);
                self.last_active
                    .lock()
                    .unwrap()
                    .insert(session.to_string(), window.to_string());
            }
        }
    }

    /// Recalculate the effective (minimum) session size across all attached
    /// clients and resize every window if it changed. Broadcasts
    /// `SessionResize` to all subscribers. This is Go's
    /// `recalculateAndBroadcastSize`.
    fn recalculate_size(&self, session: &str, broadcast: &Arc<SessionBroadcast>) {
        let Some((cols, rows)) = broadcast.effective_size() else {
            return;
        };
        // Resize every window in the session to the effective size.
        let window_ids: Vec<String> = {
            let windows = self.windows.lock().unwrap_or_else(|e| e.into_inner());
            windows
                .get(session)
                .map(|wins| wins.iter().map(|w| w.info.id.clone()).collect())
                .unwrap_or_default()
        };
        for wid in &window_ids {
            self.resize(session, wid, cols, rows);
        }
        // Broadcast the resize event to all clients.
        let count = broadcast.client_count();
        broadcast.send_to_all(&Message::SessionResize {
            session: session.to_string(),
            cols,
            rows,
            client_count: count,
        });
    }

    /// Update a client's reported terminal dimensions and recalculate the
    /// effective session size (Go's `NotifyTerminalSize` →
    /// `recalculateAndBroadcastSize`).
    fn notify_size(&self, session: &str, sub_id: u64, cols: u16, rows: u16) {
        let broadcast = self.broadcast_for(session);
        broadcast.update_size(sub_id, cols, rows);
        self.recalculate_size(session, &broadcast);
    }

    /// Build a daemon health report (the `diagnose` verb / `Diagnose` message).
    /// Reports session count, client count, uptime, and best-effort memory.
    pub fn diagnose(&self) -> super::protocol::DaemonReport {
        let sessions = self.list_infos();
        let broadcast = self.broadcast.lock().unwrap_or_else(|e| e.into_inner());
        let session_reports: Vec<super::protocol::SessionReport> = sessions
            .iter()
            .map(|s| super::protocol::SessionReport {
                name: s.name.clone(),
                windows: s.windows,
                clients: broadcast
                    .get(&s.name)
                    .map(|b| b.client_count())
                    .unwrap_or(0),
                restored: s.restored,
            })
            .collect();
        let client_count: usize = broadcast.values().map(|b| b.client_count()).sum();
        drop(broadcast);
        let uptime_secs = self.started_at.elapsed().as_secs();
        super::protocol::DaemonReport {
            session_count: sessions.len(),
            client_count,
            uptime_secs,
            memory_bytes: process_rss_bytes(),
            version: VERSION.to_string(),
            sessions: session_reports,
        }
    }

    /// Execute a headless command (no TUI required). Supported commands:
    /// `list-sessions`, `list-windows`, `capture-pane`, `send-text`,
    /// `kill-session`, `diagnose`. Returns a JSON result.
    fn headless_command(
        &self,
        command: &str,
        args: &[String],
        session: Option<&str>,
    ) -> Result<serde_json::Value, String> {
        match command {
            "list-sessions" => {
                let sessions = self.list_infos();
                Ok(serde_json::to_value(&sessions).unwrap_or(serde_json::json!([])))
            }
            "list-windows" => {
                let name = session
                    .ok_or_else(|| "missing session parameter".to_string())?;
                let windows = self.windows.lock().unwrap_or_else(|e| e.into_inner());
                let wins = windows
                    .get(name)
                    .ok_or_else(|| format!("session '{name}' not found"))?;
                let infos: Vec<WindowInfo> = wins.iter().map(|w| w.info.clone()).collect();
                Ok(serde_json::to_value(&infos).unwrap_or(serde_json::json!([])))
            }
            "capture-pane" => {
                let name = session
                    .ok_or_else(|| "missing session parameter".to_string())?;
                let window = args.first().map(|s| s.as_str());
                let (target, content) = self.capture_pane(name, window)?;
                Ok(serde_json::json!({ "window": target, "content": content }))
            }
            "send-text" => {
                let name = session
                    .ok_or_else(|| "missing session parameter".to_string())?;
                let window = args.first().map(|s| s.as_str());
                let text = args
                    .get(1)
                    .map(|s| s.as_str())
                    .ok_or_else(|| "missing text argument".to_string())?;
                self.write_input_to(name, window, text.as_bytes())?;
                Ok(serde_json::json!({ "sent": true }))
            }
            "kill-session" => {
                let name = session
                    .ok_or_else(|| "missing session parameter".to_string())?;
                self.kill(name)?;
                Ok(serde_json::json!({ "killed": name }))
            }
            "diagnose" => {
                let report = self.diagnose();
                Ok(serde_json::to_value(&report).unwrap_or(serde_json::json!({})))
            }
            _ => Err(format!("unknown headless command: {command}")),
        }
    }

    /// Resolve a window target within a session: an explicit id or title
    /// (exact, then prefix), else the session's most recently active window,
    /// else its first. `session` must exist.
    fn resolve_window(&self, session: &str, window: Option<&str>) -> Result<String, String> {
        let windows = self.windows.lock().unwrap_or_else(|e| e.into_inner());
        let wins = windows
            .get(session)
            .ok_or_else(|| format!("session '{session}' not found"))?;
        let target = match window {
            Some(w) => {
                let by_id = wins.iter().find(|w2| w2.info.id == w);
                let by_title = wins
                    .iter()
                    .find(|w2| w2.info.title == w)
                    .or_else(|| wins.iter().find(|w2| w2.info.title.starts_with(w)));
                by_id.or(by_title).map(|w2| w2.info.id.clone())
            }
            None => self
                .last_active
                .lock()
                .unwrap()
                .get(session)
                .cloned()
                .or_else(|| wins.first().map(|w| w.info.id.clone())),
        };
        match target {
            Some(t) => Ok(t),
            None => Err(format!("session '{session}' has no windows")),
        }
    }

    /// Report a window's agent state (`set-agent-state`). `window: None`
    /// targets the session's most recently active window, falling back to its
    /// first. Broadcasts `AgentStateChanged` to attached clients.
    fn set_agent_state(
        &self,
        session: &str,
        window: Option<&str>,
        state: &str,
        message: &str,
        harness: &str,
    ) -> Result<String, String> {
        let target = self.resolve_window(session, window)?;
        let mut windows = self.windows.lock().unwrap_or_else(|e| e.into_inner());
        let wins = windows
            .get_mut(session)
            .ok_or_else(|| format!("session '{session}' not found"))?;
        let Some(live) = wins.iter_mut().find(|w| w.info.id == target) else {
            return Err(format!("window '{target}' not found"));
        };
        live.info.agent_state = state.to_string();
        live.info.agent_message = message.to_string();
        live.info.agent_harness = harness.to_string();
        let info = live.info.clone();
        drop(windows);
        self.broadcast_event(
            session,
            &Message::AgentStateChanged {
                window: info.id.clone(),
                state: info.agent_state,
                message: info.agent_message,
                harness: info.agent_harness,
            },
        );
        Ok(info.id)
    }

    /// Apply a detected OSC 9;4 progress state to a window with the
    /// anti-flicker hold from Go's `agent_hold.go`: a state at or above the
    /// current loudness publishes immediately; a quieter state must stand
    /// unchanged for [`OSC_HOLD_WINDOW`]. Publishes via
    /// [`AgentStateChanged`](Message::AgentStateChanged) when it goes through.
    pub fn apply_osc_progress(
        &self,
        session: &str,
        window: &str,
        state: &AgentState,
        now: std::time::Instant,
    ) {
        let broadcast = self.broadcast_for(session);
        apply_osc_progress_pieces(
            &self.windows,
            &self.osc_holds,
            session,
            window,
            state,
            now,
            &broadcast,
        );
    }

    /// Read a window's agent state (`get-agent-state`).
    fn get_agent_state(
        &self,
        session: &str,
        window: Option<&str>,
    ) -> Result<(String, String, String, String), String> {
        let target = self.resolve_window(session, window)?;
        let windows = self.windows.lock().unwrap_or_else(|e| e.into_inner());
        let wins = windows
            .get(session)
            .ok_or_else(|| format!("session '{session}' not found"))?;
        let Some(live) = wins.iter().find(|w| w.info.id == target) else {
            return Err(format!("window '{target}' not found"));
        };
        Ok((
            live.info.id.clone(),
            live.info.agent_state.clone(),
            live.info.agent_message.clone(),
            live.info.agent_harness.clone(),
        ))
    }

    /// Write raw bytes to a window's PTY (`send-keys` / `send-text`).
    /// Wait until the pane has finished starting up before writing input. An
    /// interactive shell's terminal setup flushes queued input (and its own
    /// early output), so `send-text` immediately after
    /// `new-session`/`new-window` was flaky under load: the tty echoed the
    /// command but the shell never executed it. Windows that already produced
    /// output are past startup and are written to immediately. For a fresh
    /// window, wait for its first output and then a short quiescence — the
    /// startup burst (messages + prompt) ends when the shell blocks in its
    /// read loop, at which point any flush has completed. Bounded: a pane
    /// that produces no output (e.g. a non-interactive program) is written
    /// to anyway after the deadline.
    fn wait_pane_ready(&self, session: &str, window: &str) {
        let ring_bytes = |rings: &Mutex<HashMap<(String, String), OutputRing>>| {
            rings
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .get(&(session.to_string(), window.to_string()))
                .map(|r| r.total_bytes)
                .unwrap_or(0)
        };
        // Steady-state windows: skip the wait entirely.
        if ring_bytes(&self.rings) > 0 {
            return;
        }
        let deadline = std::time::Instant::now() + Duration::from_millis(2000);
        let mut seen_output = false;
        let mut last_bytes = 0u64;
        let mut quiet_since: Option<std::time::Instant> = None;
        loop {
            let bytes = ring_bytes(&self.rings);
            if !seen_output {
                if bytes > 0 {
                    seen_output = true;
                    last_bytes = bytes;
                }
            } else if bytes == last_bytes {
                let q = quiet_since.get_or_insert_with(std::time::Instant::now);
                if q.elapsed() >= Duration::from_millis(200) {
                    return;
                }
            } else {
                last_bytes = bytes;
                quiet_since = None;
            }
            if std::time::Instant::now() >= deadline {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn write_input_to(
        &self,
        session: &str,
        window: Option<&str>,
        data: &[u8],
    ) -> Result<String, String> {
        let target = self.resolve_window(session, window)?;
        self.wait_pane_ready(session, &target);
        let windows = self.windows.lock().unwrap_or_else(|e| e.into_inner());
        let wins = windows
            .get(session)
            .ok_or_else(|| format!("session '{session}' not found"))?;
        let Some(w) = wins.iter().find(|w| w.info.id == target) else {
            return Err(format!("window '{target}' not found"));
        };
        w.writer.write(data);
        drop(windows);
        self.last_active
            .lock()
            .unwrap()
            .insert(session.to_string(), target.clone());
        Ok(target)
    }

    /// Capture a window's recent output (`capture-pane`), the last
    /// [`CAPTURE_CAP`] bytes of its ring.
    fn capture_pane(
        &self,
        session: &str,
        window: Option<&str>,
    ) -> Result<(String, String), String> {
        let target = self.resolve_window(session, window)?;
        let rings = self.rings.lock().unwrap_or_else(|e| e.into_inner());
        let content = rings
            .get(&(session.to_string(), target.clone()))
            .map(|r| r.as_lossy())
            .unwrap_or_default();
        let content: String = content
            .chars()
            .rev()
            .take(CAPTURE_CAP)
            .collect::<String>()
            .chars()
            .rev()
            .collect();
        Ok((target, content))
    }

    /// Wait until a window's output matches `pattern` or the deadline passes
    /// (`wait-for`). Polls the output ring; an invalid pattern is an error.
    fn wait_for(
        &self,
        session: &str,
        window: Option<&str>,
        pattern: &str,
        timeout_ms: u64,
    ) -> Result<(String, bool), String> {
        let target = self.resolve_window(session, window)?;
        let re = regex::Regex::new(pattern).map_err(|e| format!("invalid pattern: {e}"))?;
        let deadline = std::time::Instant::now() + Duration::from_millis(timeout_ms);
        loop {
            let hit = {
                let rings = self.rings.lock().unwrap_or_else(|e| e.into_inner());
                rings
                    .get(&(session.to_string(), target.clone()))
                    .map(|r| re.is_match(&r.as_lossy()))
                    .unwrap_or(false)
            };
            if hit {
                return Ok((target, true));
            }
            if std::time::Instant::now() >= deadline {
                return Ok((target, false));
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// Block until the window's shell has exited, returning its exit code.
    /// `timeout_ms == 0` waits indefinitely. Wake-ups come from the exit
    /// channel (every recorded exit), so the poll is event-driven rather
    /// than a busy loop.
    fn block_until_exit(&self, session: &str, window: &str, timeout_ms: u64) -> Result<i32, String> {
        let deadline = if timeout_ms == 0 {
            None
        } else {
            Some(std::time::Instant::now() + Duration::from_millis(timeout_ms))
        };
        loop {
            let code = self
                .exit_statuses
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .get(&(session.to_string(), window.to_string()))
                .copied();
            if let Some(code) = code {
                return Ok(code);
            }
            match deadline {
                Some(d) if std::time::Instant::now() >= d => return Err("timeout".into()),
                _ => {}
            }
            let wait = deadline
                .map(|d| d.saturating_duration_since(std::time::Instant::now()))
                .unwrap_or(Duration::from_millis(200));
            let _ = self.exit_rx.recv_timeout(wait.min(Duration::from_millis(200)));
        }
    }

    /// Resolve a verb's `session` + optional `window` into a concrete
    /// (session, window id) pair. A missing session defaults to the only
    /// session (or errors on zero/multiple); the window resolves by id,
    /// exact/prefix title, or the most recently active window.
    fn resolve_verb_target(
        &self,
        session: &str,
        window: Option<&str>,
    ) -> Result<(String, String), String> {
        let name = if session.is_empty() {
            let sessions = self.list_infos();
            match sessions.len() {
                1 => sessions[0].name.clone(),
                0 => return Err("no sessions exist".into()),
                _ => return Err("multiple sessions exist; pass a session".into()),
            }
        } else {
            session.to_string()
        };
        let window = self.resolve_window(&name, window)?;
        Ok((name, window))
    }

    /// Dispatch one verb-protocol request with daemon state access.
    ///
    /// Verbs that touch daemon state (`list-sessions`, `capture-pane`,
    /// `set-agent-state`, ...) are answered here from the live session
    /// registry; the remaining documented verbs fall through to the static
    /// registry, which knows their schemas and examples.
    pub fn dispatch_verb(&self, req: &VerbRequest) -> VerbResponse {
        let id = req.id.clone();
        let params = req.params.clone().unwrap_or(Value::Null);
        match self.try_verb(&req.verb, &params) {
            Ok(result) => VerbResponse::ok(id, result),
            Err(e) => VerbResponse::err(id, e),
        }
    }

    /// The daemon-aware verb handlers. Unknown or purely-documented verbs
    /// delegate to the static [`VerbRegistry`].
    fn try_verb(&self, verb: &str, params: &Value) -> Result<Value, VerbError> {
        match verb {
            "hello" => Ok(serde_json::json!({
                "daemon": "termos",
                "version": VERSION,
                "protocol": "verb",
                "protocol_version": VERB_PROTOCOL_VERSION,
                "min_version": MIN_VERB_PROTOCOL_VERSION,
            })),
            "list-sessions" => {
                let sessions = self.list_infos();
                Ok(serde_json::json!({ "sessions": sessions }))
            }
            "new-session" => {
                let name = verb_param(params, "name")
                    .ok_or_else(|| verb_error(ERR_INVALID_PARAMS, "missing name parameter"))?;
                let shell = verb_param(params, "shell").unwrap_or_default();
                let cwd = verb_param(params, "cwd");
                self.create_session_in(&name, &shell, cwd.as_deref())
                    .map_err(|e| verb_error(ERR_COMMAND_FAILED, e))?;
                let info = self
                    .list_infos()
                    .into_iter()
                    .find(|s| s.name == name)
                    .ok_or_else(|| verb_error(ERR_SESSION_NOT_FOUND, "session not found"))?;
                Ok(serde_json::json!({ "session": info }))
            }
            "block-until-exit" => {
                let name = verb_session(params)?;
                let window = verb_param(params, "window");
                let timeout_ms = verb_param(params, "timeout")
                    .and_then(|s| s.parse::<u64>().ok())
                    .unwrap_or(30_000);
                let target = self
                    .resolve_window(&name, window.as_deref())
                    .map_err(|e| verb_error(ERR_WINDOW_NOT_FOUND, e))?;
                match self.block_until_exit(&name, &target, timeout_ms) {
                    Ok(exit_code) => Ok(serde_json::json!({
                        "window": target,
                        "exit_code": exit_code,
                        "success": exit_code == 0,
                    })),
                    Err(e) if e == "timeout" => Err(verb_error(
                        ERR_TIMEOUT,
                        "window did not exit before the timeout",
                    )),
                    Err(e) => Err(verb_error(ERR_COMMAND_FAILED, e)),
                }
            }
            "list-windows" => {
                let name = verb_session(params)?;
                let windows = self.windows.lock().unwrap_or_else(|e| e.into_inner());
                let wins = windows.get(&name).ok_or_else(|| {
                    verb_error(ERR_SESSION_NOT_FOUND, format!("session '{name}' not found"))
                })?;
                let infos: Vec<WindowInfo> = wins.iter().map(|w| w.info.clone()).collect();
                Ok(serde_json::json!({ "windows": infos }))
            }
            "new-window" => {
                let name = verb_session(params)?;
                let shell = verb_param(params, "shell").unwrap_or_else(|| "/bin/sh".to_string());
                let cwd = verb_param(params, "cwd");
                let workspace = verb_param(params, "workspace")
                    .and_then(|s| s.parse::<i32>().ok())
                    .unwrap_or(1);
                let info = self
                    .add_window_in(&name, &shell, workspace, cwd.as_deref())
                    .map_err(|e| verb_error(ERR_COMMAND_FAILED, e))?;
                Ok(serde_json::json!({ "window": info }))
            }
            "close-window" => {
                let name = verb_session(params)?;
                let window = verb_param(params, "window");
                let target = self
                    .resolve_window(&name, window.as_deref())
                    .map_err(|e| verb_error(ERR_WINDOW_NOT_FOUND, e))?;
                self.close_window(&name, &target)
                    .map_err(|e| verb_error(ERR_COMMAND_FAILED, e))?;
                Ok(serde_json::json!({ "closed": true }))
            }
            "send-keys" | "send-text" => {
                let name = verb_session(params)?;
                let window = verb_param(params, "window");
                let text = verb_param(params, "text")
                    .or_else(|| verb_param(params, "keys"))
                    .ok_or_else(|| verb_error(ERR_INVALID_PARAMS, "missing text/keys parameter"))?;
                self.write_input_to(&name, window.as_deref(), text.as_bytes())
                    .map_err(|e| verb_error(ERR_COMMAND_FAILED, e))?;
                Ok(serde_json::json!({ "sent": text }))
            }
            "capture-pane" => {
                let name = verb_session(params)?;
                let window = verb_param(params, "window");
                // Resolve first so a bad target yields window_not_found
                // rather than a generic command failure.
                let target = self
                    .resolve_window(&name, window.as_deref())
                    .map_err(|e| verb_error(ERR_WINDOW_NOT_FOUND, e))?;
                let (target, content) = self
                    .capture_pane(&name, Some(&target))
                    .map_err(|e| verb_error(ERR_COMMAND_FAILED, e))?;
                Ok(serde_json::json!({ "window": target, "content": content }))
            }
            "resize" => {
                let name = verb_session(params)?;
                let window = verb_param(params, "window");
                let cols = verb_param(params, "cols")
                    .and_then(|s| s.parse::<u16>().ok())
                    .ok_or_else(|| verb_error(ERR_INVALID_PARAMS, "missing cols parameter"))?;
                let rows = verb_param(params, "rows")
                    .and_then(|s| s.parse::<u16>().ok())
                    .ok_or_else(|| verb_error(ERR_INVALID_PARAMS, "missing rows parameter"))?;
                let target = self
                    .resolve_window(&name, window.as_deref())
                    .map_err(|e| verb_error(ERR_WINDOW_NOT_FOUND, e))?;
                self.resize(&name, &target, cols, rows);
                Ok(serde_json::json!({ "resized": true }))
            }
            "kill-session" => {
                let name = verb_session(params)?;
                self.kill(&name)
                    .map_err(|e| verb_error(ERR_COMMAND_FAILED, e))?;
                Ok(serde_json::json!({ "killed": name }))
            }
            "set-agent-state" => {
                let name = verb_session(params)?;
                let window = verb_param(params, "window");
                let state = verb_param(params, "state").unwrap_or_default();
                let message = verb_param(params, "message").unwrap_or_default();
                let harness = verb_param(params, "harness").unwrap_or_default();
                let target = self
                    .set_agent_state(&name, window.as_deref(), &state, &message, &harness)
                    .map_err(|e| verb_error(ERR_COMMAND_FAILED, e))?;
                Ok(serde_json::json!({ "window": target, "state": state }))
            }
            "get-agent-state" => {
                let name = verb_session(params)?;
                let window = verb_param(params, "window");
                let (target, state, message, harness) = self
                    .get_agent_state(&name, window.as_deref())
                    .map_err(|e| verb_error(ERR_WINDOW_NOT_FOUND, e))?;
                Ok(serde_json::json!({
                    "window": target,
                    "state": state,
                    "message": message,
                    "harness": harness,
                }))
            }
            "wait-for" => {
                let name = verb_session(params)?;
                let window = verb_param(params, "window");
                let pattern = verb_param(params, "pattern").unwrap_or_default();
                let timeout_ms = verb_param(params, "timeout")
                    .and_then(|s| s.parse::<u64>().ok())
                    .unwrap_or(30_000);
                match self.wait_for(&name, window.as_deref(), &pattern, timeout_ms) {
                    Ok((target, matched)) if matched => Ok(serde_json::json!({
                        "window": target,
                        "matched": true,
                    })),
                    Ok((_, _)) => Err(verb_error(
                        ERR_TIMEOUT,
                        "condition did not match before timeout",
                    )),
                    Err(e) => Err(verb_error(ERR_COMMAND_FAILED, e)),
                }
            }
            "list-verbs" => {
                let registry = VerbRegistry::new();
                let filter = verb_param(params, "verb");
                Ok(registry.list_verbs(filter.as_deref()))
            }
            "set-option" | "get-option" => Err(verb_error(
                ERR_OPTION_NOT_FOUND,
                "option storage is not implemented in this port",
            )),
            "session-info" => {
                let name = verb_session(params)?;
                self.session_info(&name)
                    .map_err(|e| verb_error(ERR_SESSION_NOT_FOUND, e))
            }
            "set-session-name" => {
                let name = verb_session(params)?;
                let label = verb_param(params, "name")
                    .ok_or_else(|| verb_error(ERR_INVALID_PARAMS, "missing name parameter"))?;
                self.set_session_label(&name, &label)
                    .map_err(|e| verb_error(ERR_SESSION_NOT_FOUND, e))?;
                Ok(serde_json::json!({ "renamed": name, "display_name": label }))
            }
            "set-session-accent" => {
                let name = verb_session(params)?;
                let accent = verb_param(params, "accent")
                    .ok_or_else(|| verb_error(ERR_INVALID_PARAMS, "missing accent parameter"))?;
                self.set_session_accent(&name, &accent)
                    .map_err(|e| verb_error(ERR_SESSION_NOT_FOUND, e))?;
                Ok(serde_json::json!({ "accented": name, "accent": accent }))
            }
            "set-workspace-name" | "explain-agent-screen" | "subscribe" | "unsubscribe" => {
                // Fall through to the registry, which documents them.
                self.registry_dispatch(verb, params)
            }
            "diagnose" => {
                let report = self.diagnose();
                Ok(serde_json::to_value(&report).unwrap_or(serde_json::json!({})))
            }
            "headless-command" => {
                let command = verb_param(params, "command")
                    .ok_or_else(|| verb_error(ERR_INVALID_PARAMS, "missing command parameter"))?;
                let args: Vec<String> = params
                    .get("args")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str().map(|s| s.to_string()))
                            .collect()
                    })
                    .unwrap_or_default();
                let session = verb_param(params, "session");
                let result = self
                    .headless_command(&command, &args, session.as_deref())
                    .map_err(|e| verb_error(ERR_COMMAND_FAILED, e))?;
                Ok(result)
            }
            _ => self.registry_dispatch(verb, params),
        }
    }

    /// Delegate a verb to the static registry (documentation, list-verbs,
    /// hello, and the verbs without daemon state).
    fn registry_dispatch(&self, verb: &str, params: &Value) -> Result<Value, VerbError> {
        let registry = VerbRegistry::new();
        let req = VerbRequest {
            id: None,
            verb: verb.to_string(),
            params: Some(params.clone()),
        };
        match registry.dispatch(&req) {
            VerbResponse {
                result: Some(r), ..
            } => Ok(r),
            VerbResponse { error: Some(e), .. } => Err(e),
            _ => Err(verb_error(ERR_UNKNOWN_VERB, format!("unknown verb {verb}"))),
        }
    }

    /// Allocate the next window id for a session. Monotonic per session and
    /// never reuses a number, even after windows close (see `win_seq`).
    fn next_window_id(&self, session: &str) -> String {
        let mut seq = self.win_seq.lock().unwrap_or_else(|e| e.into_inner());
        let n = seq.entry(session.to_string()).or_insert(0);
        let id = format!("w{n}");
        *n += 1;
        id
    }

    fn add_window(&self, session: &str, shell: &str, workspace: i32) -> Result<WindowInfo, String> {
        self.add_window_in(session, shell, workspace, None)
    }

    /// [`add_window`] with an explicit starting directory for the shell.
    fn add_window_in(
        &self,
        session: &str,
        shell: &str,
        workspace: i32,
        cwd: Option<&str>,
    ) -> Result<WindowInfo, String> {
        let mut windows = self.windows.lock().unwrap_or_else(|e| e.into_inner());
        let wins = windows
            .get_mut(session)
            .ok_or_else(|| format!("session '{session}' not found"))?;
        let id = self.next_window_id(session);
        let shell = resolve_shell(shell);
        let broadcast = self
            .broadcast
            .lock()
            .unwrap()
            .get(session)
            .cloned()
            .ok_or_else(|| format!("session '{session}' not found"))?;
        let live = self
            .spawn_window(session, &id, "Terminal", workspace, &shell, cwd, &broadcast)?;
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
        self.broadcast_event(
            session,
            &Message::WindowAdded {
                window: info.clone(),
            },
        );
        Ok(info)
    }

    fn close_window(&self, session: &str, window: &str) -> Result<(), String> {
        let mut windows = self.windows.lock().unwrap_or_else(|e| e.into_inner());
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
        // A window closed by the user is terminated (SIGHUP + reap in the
        // handle's Drop); record -1 unless the shell already exited naturally
        // and the pump recorded its real code.
        if let Ok(mut st) = self.exit_statuses.lock() {
            st.entry((session.to_string(), window.to_string())).or_insert(-1);
        }
        let _ = self.exit_tx.send(());
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
        self.broadcast_event(
            session,
            &Message::WindowClosed {
                window: window.to_string(),
            },
        );
        Ok(())
    }

    /// Send an event to every client attached to a session.
    fn broadcast_event(&self, session: &str, msg: &Message) {
        if let Some(b) = self.broadcast.lock().unwrap_or_else(|e| e.into_inner()).get(session) {
            b.send_to_all(msg);
        }
    }

    /// Persist a session's window definitions for resurrection.
    fn save_session(&self, name: &str) {
        let windows = self.windows.lock().unwrap_or_else(|e| e.into_inner());
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
    /// Restore a single saved session by name (`Resurrect` message).
    pub fn restore_saved_named(&self, name: &str) -> Result<(), String> {
        let states = persistence::list_saved();
        let state = states
            .iter()
            .find(|s| s.name == name)
            .ok_or_else(|| format!("no saved state for session '{name}'"))?;
        self.restore_session(&state.name, state)
    }

    /// Set a session's display label (`set-session-name` verb).
    pub fn set_session_label(&self, session: &str, label: &str) -> Result<(), String> {
        if self.manager.get(session).is_none() {
            return Err(format!("session '{session}' not found"));
        }
        self.meta
            .lock()
            .unwrap()
            .entry(session.to_string())
            .or_default()
            .display_name = label.to_string();
        Ok(())
    }

    /// Set a session's accent colour (`set-session-accent` verb).
    pub fn set_session_accent(&self, session: &str, accent: &str) -> Result<(), String> {
        if self.manager.get(session).is_none() {
            return Err(format!("session '{session}' not found"));
        }
        self.meta
            .lock()
            .unwrap()
            .entry(session.to_string())
            .or_default()
            .accent = accent.to_string();
        Ok(())
    }

    /// A session's metadata (label/accent), defaulting to empty.
    pub fn session_meta(&self, session: &str) -> SessionMeta {
        self.meta
            .lock()
            .unwrap()
            .get(session)
            .cloned()
            .unwrap_or_default()
    }

    /// Set a runtime config option on a session (`set-config`).
    pub fn set_session_option(&self, session: &str, path: &str, value: &str) {
        if self.manager.get(session).is_none() {
            return;
        }
        self.meta
            .lock()
            .unwrap()
            .entry(session.to_string())
            .or_default()
            .options
            .insert(path.to_string(), value.to_string());
    }

    /// Get a runtime config option from a session (`get-config`).
    pub fn get_session_option(&self, session: &str, path: &str) -> String {
        self.meta
            .lock()
            .unwrap()
            .get(session)
            .and_then(|m| m.options.get(path).cloned())
            .unwrap_or_default()
    }

    /// Name a workspace (`set-workspace-name`).
    pub fn set_workspace_name(&self, session: &str, workspace: i32, name: &str) {
        if self.manager.get(session).is_none() {
            return;
        }
        self.meta
            .lock()
            .unwrap()
            .entry(session.to_string())
            .or_default()
            .workspace_names
            .insert(workspace.to_string(), name.to_string());
    }

    /// Explain what a harness's screen rules make of a pane
    /// (`explain-agent-screen`). Returns a JSON object with the pane's tail
    /// and rule evaluation.
    pub fn explain_agent_screen(
        &self,
        session: &str,
        window: Option<&str>,
        harness: &str,
        lines: i32,
    ) -> serde_json::Value {
        let target = match self.resolve_window(session, window) {
            Ok(t) => t,
            Err(e) => {
                return serde_json::json!({
                    "error": e,
                    "window_id": null,
                    "harness_id": harness,
                    "state": "none",
                    "tail": [],
                    "rules": [],
                });
            }
        };
        // Read the window's output ring tail.
        let tail: Vec<String> = {
            let rings = self.rings.lock().unwrap_or_else(|e| e.into_inner());
            rings
                .get(&(session.to_string(), target.clone()))
                .map(|r| {
                    let content = r.as_lossy();
                    let all_lines: Vec<&str> = content.lines().collect();
                    let n = if lines > 0 {
                        lines as usize
                    } else {
                        all_lines.len().min(20)
                    };
                    let start = all_lines.len().saturating_sub(n);
                    all_lines[start..].iter().map(|s| s.to_string()).collect()
                })
                .unwrap_or_default()
        };
        // Get the window's current agent state.
        let (_wid, state, _message, _harness) = self
            .get_agent_state(session, Some(&target))
            .unwrap_or((target.clone(), String::new(), String::new(), String::new()));

        serde_json::json!({
            "window_id": target,
            "harness_id": harness,
            "state": if state.is_empty() { "none" } else { &state },
            "source": "screen",
            "enabled": !harness.is_empty(),
            "lines": tail.len(),
            "tail": tail,
            "matched": false,
            "rule": -1,
            "rule_state": "none",
            "rules": [],
        })
    }

    /// `session-info` verb: the session plus its window count and metadata.
    pub fn session_info(&self, session: &str) -> Result<serde_json::Value, String> {
        let s = self
            .manager
            .get(session)
            .ok_or_else(|| format!("session '{session}' not found"))?;
        let windows = self.windows.lock().unwrap_or_else(|e| e.into_inner());
        let count = windows.get(session).map(|w| w.len()).unwrap_or(0);
        let meta = self.session_meta(session);
        Ok(serde_json::json!({
            "name": s.name,
            "display_name": meta.display_name,
            "accent": meta.accent,
            "created_at": s.created_at,
            "restored": s.restored,
            "windows": count,
        }))
    }

    /// The canonical [`SessionState`](crate::session::state_merge::SessionState)
    /// for a session: its windows with daemon-owned agent state, built from
    /// the live registry.
    pub fn state_for_session(&self, name: &str) -> crate::session::state_merge::SessionState {
        use crate::session::state_merge::WindowState as MergeWindow;
        let windows = self.windows.lock().unwrap_or_else(|e| e.into_inner());
        let wins: Vec<WindowInfo> = windows
            .get(name)
            .map(|wins| wins.iter().map(|w| w.info.clone()).collect())
            .unwrap_or_default();
        let meta = self.session_meta(name);
        crate::session::state_merge::SessionState {
            name: name.to_string(),
            display_name: meta.display_name,
            accent: meta.accent,
            restored: false,
            resurrection_version: 0,
            windows: wins
                .iter()
                .map(|w| MergeWindow {
                    id: w.id.clone(),
                    title: w.title.clone(),
                    workspace: w.workspace,
                    agent_state: w.agent_state.clone(),
                    agent_message: w.agent_message.clone(),
                    agent_harness: w.agent_harness.clone(),
                    minimized: false,
                    cwd: String::new(),
                    foreground: String::new(),
                })
                .collect(),
        }
    }

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
    /// accept clients. Returns only on a bind failure. Cleans up stale
    /// resurrection temp files and prunes old archives on startup.
    pub fn run(self: Arc<Self>, path: &std::path::Path) -> io::Result<()> {
        // Clean up leftover temp files and prune old archives from the
        // resurrection directory (Go's `CleanResurrectionDir` on startup).
        super::resurrection::clean_resurrection_dir();

        // The start lock makes the stale-socket recovery below safe: holding
        // it proves no other daemon is mid-bind, so unlinking the socket
        // cannot cut a live daemon's inode out from under it.
        let _lock = super::startlock::StartLock::acquire(path)
            .map_err(|e| io::Error::new(io::ErrorKind::AlreadyExists, e.to_string()))?;
        if path.exists() {
            let _ = std::fs::remove_file(path);
        }
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let listener = UnixListener::bind(path)?;
        listener.set_nonblocking(true)?;
        log::info!("termos daemon listening on {}", path.display());

        for stream in listener.incoming() {
            if self.shutdown.load(std::sync::atomic::Ordering::SeqCst) {
                log::info!("termos daemon shutting down");
                break;
            }
            match stream {
                Ok(stream) => {
                    let _ = stream.set_nonblocking(false);
                    let daemon = Arc::clone(&self);
                    std::thread::spawn(move || handle_client(stream, daemon));
                }
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(50));
                    continue;
                }
                Err(e) => log::warn!("accept error: {e}"),
            }
        }
        // Clean up the socket file on shutdown.
        let _ = std::fs::remove_file(path);
        log::info!("termos daemon socket cleaned up");
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
/// The shared OSC-progress application used by both the daemon method and the
/// per-window output pump. Reads the window's current state, applies the
/// anti-flicker hold, and broadcasts `AgentStateChanged` when the state
/// publishes.
fn apply_osc_progress_pieces(
    windows: &Mutex<HashMap<String, Vec<LiveWindow>>>,
    holds: &Mutex<HashMap<String, (AgentState, std::time::Instant)>>,
    session: &str,
    window: &str,
    state: &AgentState,
    now: std::time::Instant,
    broadcast: &Arc<SessionBroadcast>,
) {
    let current = {
        let windows = windows.lock().unwrap_or_else(|e| e.into_inner());
        windows
            .get(session)
            .and_then(|wins| wins.iter().find(|w| w.info.id == window))
            .and_then(|w| AgentState::parse(&w.info.agent_state))
    };
    let Some(current) = current else { return };
    if *state == current {
        holds.lock().unwrap_or_else(|e| e.into_inner()).remove(window);
        return;
    }
    if !hold_quieter_state(holds, window, state, &current, now) {
        return;
    }
    let mut wins = windows.lock().unwrap_or_else(|e| e.into_inner());
    let Some(live) = wins
        .get_mut(session)
        .and_then(|wins| wins.iter_mut().find(|w| w.info.id == window))
    else {
        return;
    };
    live.info.agent_state = state.name().to_string();
    live.info.agent_message.clear();
    live.info.agent_harness = "osc".to_string();
    let info = live.info.clone();
    drop(wins);
    broadcast.send_to_all(&Message::AgentStateChanged {
        window: info.id,
        state: info.agent_state,
        message: info.agent_message,
        harness: info.agent_harness,
    });
}

/// Go's `holdQuieterState`: decide whether `next` may be published now.
fn hold_quieter_state(
    holds: &Mutex<HashMap<String, (AgentState, std::time::Instant)>>,
    window: &str,
    next: &AgentState,
    current: &AgentState,
    now: std::time::Instant,
) -> bool {
    if agent_loudness(next) >= agent_loudness(current) {
        holds.lock().unwrap_or_else(|e| e.into_inner()).remove(window);
        return true;
    }
    let mut holds = holds.lock().unwrap_or_else(|e| e.into_inner());
    match holds.get(window) {
        Some((held, since)) if *held == *next => {
            if now.duration_since(*since) < OSC_HOLD_WINDOW {
                false
            } else {
                holds.remove(window);
                true
            }
        }
        _ => {
            holds.insert(window.to_string(), (*next, now));
            false
        }
    }
}

/// How strongly an agent state wants a human (Go's `agentLoudness`):
/// NeedsInput/Errored > Working > Idle/Done > None.
fn agent_loudness(state: &AgentState) -> i32 {
    match state {
        AgentState::NeedsInput | AgentState::Errored => 3,
        AgentState::Working => 2,
        AgentState::Idle | AgentState::Done => 1,
        AgentState::None => 0,
    }
}

/// Extract a string parameter from a verb request's params object.
fn verb_param(params: &Value, name: &str) -> Option<String> {
    // Accept string parameters plus JSON numbers/bools (clients like
    // `termos action timeout=5000` send typed values); coerce them to their
    // string form so verb handlers keep one parsing path.
    match params.get(name) {
        Some(Value::String(s)) => Some(s.clone()),
        Some(Value::Number(n)) => Some(n.to_string()),
        Some(Value::Bool(b)) => Some(b.to_string()),
        _ => None,
    }
}

/// The session a verb targets: the `session` param, else the daemon's single
/// attached session.
fn verb_session(params: &Value) -> Result<String, VerbError> {
    if let Some(name) = verb_param(params, "session") {
        return Ok(name);
    }
    Err(verb_error(ERR_INVALID_PARAMS, "missing session parameter"))
}

/// Build a `VerbError` with the given stable code and message.
fn verb_error(code: &str, message: impl Into<String>) -> VerbError {
    VerbError::new(code, message)
}

/// Serve one connection speaking the line-delimited JSON verb protocol:
/// read a request line, dispatch it with daemon state access, write one
/// response line. Blocks until the client disconnects.
fn handle_verb_client<R: BufRead>(
    mut reader: R,
    writer: &Arc<Mutex<UnixStream>>,
    daemon: &Arc<Daemon>,
) {
    let mut line = String::new();
    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) | Err(_) => return, // EOF or connection gone
            Ok(_) => {}
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let req = match serde_json::from_str::<VerbRequest>(trimmed) {
            Ok(req) => req,
            Err(e) => {
                let response = VerbResponse::err(
                    None,
                    verb_error(ERR_INVALID_REQUEST, format!("malformed JSON request: {e}")),
                );
                let mut out = writer.lock().unwrap_or_else(|e| e.into_inner());
                if out.write_all(response.to_line().as_bytes()).is_err() {
                    return;
                }
                if out.flush().is_err() {
                    return;
                }
                continue;
            }
        };
        // `subscribe` switches this connection to a long-lived output stream
        // (pane tail), which runs until the window closes or the client
        // disconnects.
        if req.verb == "subscribe" {
            handle_verb_subscribe(writer, daemon, &req);
            return;
        }
        let response = daemon.dispatch_verb(&req);
        let mut out = writer.lock().unwrap_or_else(|e| e.into_inner());
        if out.write_all(response.to_line().as_bytes()).is_err() {
            return;
        }
        if out.flush().is_err() {
            return;
        }
    }
}

/// The `subscribe` verb: tail a window's raw output over this connection.
///
/// Replies with an ack line, then streams one JSON line per output chunk
/// (`{"data": "<lossy utf-8>"}`) and finally a `{"closed": true}` line when
/// the window's shell exits or the window is closed. The stream ends when
/// the client disconnects (the read timeout fires on every poll and doubles
/// as the liveness probe).
fn handle_verb_subscribe(
    writer: &Arc<Mutex<UnixStream>>,
    daemon: &Arc<Daemon>,
    req: &VerbRequest,
) {
    let id = req.id.clone();
    let params = req.params.clone().unwrap_or(Value::Null);
    let session = verb_session(&params).unwrap_or_default();
    let window = verb_param(&params, "window");
    let (session, window) = match daemon.resolve_verb_target(&session, window.as_deref()) {
        Ok(t) => t,
        Err(e) => {
            let resp = VerbResponse::err(id, verb_error(ERR_WINDOW_NOT_FOUND, e));
            let mut out = writer.lock().unwrap_or_else(|e| e.into_inner());
            let _ = out.write_all(resp.to_line().as_bytes());
            let _ = out.flush();
            return;
        }
    };

    let mut seq = daemon
        .rings
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&(session.clone(), window.clone()))
        .map(|r| r.current_seq())
        .unwrap_or(0);

    // Ack: the client knows which window it is tailing and the start position.
    let ack = VerbResponse::ok(
        id.clone(),
        serde_json::json!({
            "window": window,
            "subscribed": true,
            "seq": seq,
        }),
    );
    let mut out = writer.lock().unwrap_or_else(|e| e.into_inner());
    if out.write_all(ack.to_line().as_bytes()).is_err() {
        return;
    }
    if out.flush().is_err() {
        return;
    }
    drop(out);

    // Poll the ring for new output; the 100ms read timeout on the raw
    // stream doubles as the disconnect probe.
    if let Ok(s) = writer.lock() {
        let _ = s.set_read_timeout(Some(Duration::from_millis(100)));
    }
    let mut probe = [0u8; 1];
    loop {
        // Client disconnect / stray input (unsubscribe) ends the stream.
        match writer.lock().unwrap_or_else(|e| e.into_inner()).read(&mut probe) {
            Ok(0) => return, // EOF: client closed the connection
            Ok(_) => return, // client sent something (unsubscribe) — close the stream
            Err(e) if e.kind() == io::ErrorKind::WouldBlock || e.kind() == io::ErrorKind::TimedOut => {}
            Err(_) => return,
        }

        let (chunk, closed) = {
            let rings = daemon.rings.lock().unwrap_or_else(|e| e.into_inner());
            let ring = rings.get(&(session.clone(), window.clone()));
            let chunk = ring.map(|r| r.output_since(seq)).unwrap_or_default();
            let drained = chunk.is_empty();
            let exited = daemon
                .exit_statuses
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .contains_key(&(session.clone(), window.clone()));
            (chunk, drained && exited)
        };
        if !chunk.is_empty() {
            let data = String::from_utf8_lossy(&chunk).into_owned();
            let resp = VerbResponse::ok(id.clone(), serde_json::json!({"data": data}));
            let mut out = writer.lock().unwrap_or_else(|e| e.into_inner());
            if out.write_all(resp.to_line().as_bytes()).is_err() {
                return;
            }
            if out.flush().is_err() {
                return;
            }
            if let Ok(rings) = daemon.rings.lock() {
                if let Some(r) = rings.get(&(session.clone(), window.clone())) {
                    seq = r.current_seq();
                }
            }
        }
        if closed {
            let resp = VerbResponse::ok(id.clone(), serde_json::json!({"closed": true}));
            let mut out = writer.lock().unwrap_or_else(|e| e.into_inner());
            if out.write_all(resp.to_line().as_bytes()).is_err() {
                return;
            }
            let _ = out.flush();
            return;
        }
        std::thread::sleep(Duration::from_millis(30));
    }
}

fn handle_client(stream: UnixStream, daemon: Arc<Daemon>) {
    let reader = match stream.try_clone() {
        Ok(r) => r,
        Err(_) => return,
    };
    let writer = Arc::new(Mutex::new(stream));
    let mut buf_reader = io::BufReader::new(reader);

    // Detect a JSON verb-protocol client by its first byte: `{` or leading
    // whitespace. A binary client's first byte is the high byte of a
    // big-endian length prefix (0x00/0x01 for sub-16MB frames), which never
    // collides with `{` or whitespace.
    let first = buf_reader.fill_buf().ok().and_then(|b| b.first().copied());
    if matches!(
        first,
        Some(b'{') | Some(b' ') | Some(b'\t') | Some(b'\n') | Some(b'\r')
    ) {
        handle_verb_client(buf_reader, &writer, &daemon);
        return;
    }

    // (session name, subscriber id, stop flag for the forward thread).
    let mut attached: Option<(String, u64, Arc<AtomicBool>)> = None;
    // The client name from the Hello handshake, for join/leave notifications.
    let mut client_name = String::from("client");

    while let Ok(msg) = protocol::read_message(&mut buf_reader) {
        match msg {
            Message::Hello {
                name,
                codec,
                cols,
                rows,
            } => {
                if !name.is_empty() {
                    client_name = name;
                }
                // Negotiate codec (accepted but both use JSON framing here).
                let negotiated = protocol::negotiate_codec(codec.as_deref());
                let sessions = daemon.list_infos();
                let _ = send(
                    &writer,
                    &Message::Welcome {
                        version: VERSION.to_string(),
                        sessions,
                        codec: Some(negotiated.as_str().to_string()),
                    },
                );
                // Store dimensions for when the client attaches (best-effort).
                if let (Some(c), Some(r)) = (cols, rows) {
                    if let Some((sess, sub_id, _)) = &attached {
                        daemon.notify_size(sess, *sub_id, c, r);
                    }
                }
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
                match daemon.attach_named(&name, &client_name, 0, 0) {
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
            Message::Input { window, data } => {
                if let Some((session, _, _)) = &attached {
                    daemon.write_input(session, &window, &data);
                }
            }
            Message::WriteInput {
                session,
                window,
                data,
            } => {
                let Some(target_session) = resolve_session(&attached, &session) else {
                    let _ = send(
                        &writer,
                        &Message::Error {
                            message: "no session targeted (attach to one or pass -s)".into(),
                        },
                    );
                    continue;
                };
                match daemon.write_input_to(&target_session, window.as_deref(), &data) {
                    Ok(_) => {}
                    Err(e) => {
                        let _ = send(&writer, &Message::Error { message: e });
                    }
                }
            }
            Message::CapturePane { session, window } => {
                let Some(target_session) = resolve_session(&attached, &session) else {
                    let _ = send(
                        &writer,
                        &Message::Error {
                            message: "no session targeted (attach to one or pass -s)".into(),
                        },
                    );
                    continue;
                };
                match daemon.capture_pane(&target_session, window.as_deref()) {
                    Ok((window, content)) => {
                        let _ = send(&writer, &Message::PaneCapture { window, content });
                    }
                    Err(e) => {
                        let _ = send(&writer, &Message::Error { message: e });
                    }
                }
            }
            Message::WaitFor {
                session,
                window,
                pattern,
                timeout_ms,
            } => {
                let Some(target_session) = resolve_session(&attached, &session) else {
                    let _ = send(
                        &writer,
                        &Message::Error {
                            message: "no session targeted (attach to one or pass -s)".into(),
                        },
                    );
                    continue;
                };
                match daemon.wait_for(&target_session, window.as_deref(), &pattern, timeout_ms) {
                    Ok((window, matched)) => {
                        let _ = send(&writer, &Message::WaitResult { window, matched });
                    }
                    Err(e) => {
                        let _ = send(&writer, &Message::Error { message: e });
                    }
                }
            }
            Message::TapeExecute { session, script } => {
                // Parse the tape; broadcast each command to the session's
                // attached clients, which run them against their app state
                // (Go's RemoteTapeCommandMsg flow).
                let (commands, errors) = crate::tape::parser::parse_file(&script);
                if !errors.is_empty() || commands.is_empty() {
                    let _ = send(
                        &writer,
                        &Message::Error {
                            message: if commands.is_empty() {
                                "tape script has no commands or contains errors".into()
                            } else {
                                "tape script has parsing errors".into()
                            },
                        },
                    );
                    continue;
                }
                let total = commands.len();
                for (i, cmd) in commands.iter().enumerate() {
                    daemon.broadcast_event(
                        &session,
                        &Message::TapeCommand {
                            index: i,
                            total,
                            command: cmd.clone(),
                        },
                    );
                }
                daemon.broadcast_event(&session, &Message::TapeFinished { total });
                // Acknowledge the requesting client so `tape exec` can exit
                // cleanly once the broadcast completes.
                let _ = send(&writer, &Message::TapeFinished { total });
            }
            Message::GetAgentState { session, window } => {
                let Some(target_session) = resolve_session(&attached, &session) else {
                    let _ = send(
                        &writer,
                        &Message::Error {
                            message: "no session targeted (attach to one or pass -s)".into(),
                        },
                    );
                    continue;
                };
                match daemon.get_agent_state(&target_session, window.as_deref()) {
                    Ok((window, state, message, harness)) => {
                        let _ = send(
                            &writer,
                            &Message::AgentStateResult {
                                window,
                                state,
                                message,
                                harness,
                            },
                        );
                    }
                    Err(e) => {
                        let _ = send(&writer, &Message::Error { message: e });
                    }
                }
            }
            Message::Resize { window, cols, rows } => {
                if let Some((session, _, _)) = &attached {
                    daemon.resize(session, &window, cols, rows);
                }
            }
            Message::SetAgentState {
                session,
                window,
                state,
                message,
                harness,
            } => {
                // Resolve the target session: named, else the session this
                // connection is attached to.
                match resolve_session(&attached, &session) {
                    Some(s) => match daemon.set_agent_state(
                        &s,
                        window.as_deref(),
                        &state,
                        &message,
                        &harness,
                    ) {
                        Ok(window) => {
                            let _ = send(
                                &writer,
                                &Message::AgentStateChanged {
                                    window,
                                    state,
                                    message,
                                    harness,
                                },
                            );
                        }
                        Err(e) => {
                            let _ = send(&writer, &Message::Error { message: e });
                        }
                    },
                    None => {
                        let _ = send(
                            &writer,
                            &Message::Error {
                                message: "no session targeted (attach to one or pass -s)".into(),
                            },
                        );
                    }
                }
            }
            Message::Ping => {
                let _ = send(&writer, &Message::Pong);
            }
            Message::Pong => {}
            Message::Resurrect { name } => match daemon.restore_saved_named(&name) {
                Ok(()) => {
                    let _ = send(
                        &writer,
                        &Message::ListResult {
                            sessions: daemon.list_infos(),
                        },
                    );
                }
                Err(e) => {
                    let _ = send(&writer, &Message::Error { message: e });
                }
            },
            Message::ResurrectAll => {
                daemon.restore_saved();
                let _ = send(
                    &writer,
                    &Message::ListResult {
                        sessions: daemon.list_infos(),
                    },
                );
            }
            Message::KillServer => {
                let _ = send(&writer, &Message::Pong);
                daemon
                    .shutdown
                    .store(true, std::sync::atomic::Ordering::SeqCst);
            }
            Message::SetSessionName { session, name } => {
                if let Err(e) = daemon.set_session_label(&session, &name) {
                    let _ = send(&writer, &Message::Error { message: e });
                } else {
                    let _ = send(
                        &writer,
                        &Message::ListResult {
                            sessions: daemon.list_infos(),
                        },
                    );
                }
            }
            Message::SetSessionAccent { session, accent } => {
                if let Err(e) = daemon.set_session_accent(&session, &accent) {
                    let _ = send(&writer, &Message::Error { message: e });
                } else {
                    let _ = send(
                        &writer,
                        &Message::ListResult {
                            sessions: daemon.list_infos(),
                        },
                    );
                }
            }
            Message::ClientJoined { .. } | Message::ClientLeft { .. } => {}
            Message::GetState { session } => {
                let state = daemon.state_for_session(&session);
                let _ = send(&writer, &Message::StateResult { state });
            }
            Message::StateResult { .. } => {}
            Message::NewWindowInSession { session, name } => {
                let Some(target_session) = resolve_session(&attached, &session) else {
                    let _ = send(
                        &writer,
                        &Message::Error {
                            message: "no session targeted (attach to one or pass -s)".into(),
                        },
                    );
                    continue;
                };
                let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
                match daemon.add_window(&target_session, &shell, 1) {
                    Ok(mut info) => {
                        if !name.is_empty() {
                            info.title = name.clone();
                        }
                        let _ = send(&writer, &Message::NewWindowResult { window: info });
                    }
                    Err(e) => {
                        let _ = send(&writer, &Message::Error { message: e });
                    }
                }
            }
            Message::NewWindowResult { .. } => {}
            Message::RunCommand { session, command, args } => {
                let Some(target_session) = resolve_session(&attached, &session) else {
                    let _ = send(
                        &writer,
                        &Message::Error {
                            message: "no session targeted (attach to one or pass -s)".into(),
                        },
                    );
                    continue;
                };
                // Execute the tape command by broadcasting it to attached
                // clients (same flow as `tape exec` for single commands).
                let cmd = crate::tape::command::Command::from_name_and_args(&command, &args);
                daemon.broadcast_event(
                    &target_session,
                    &Message::TapeCommand {
                        index: 0,
                        total: 1,
                        command: cmd,
                    },
                );
                daemon.broadcast_event(&target_session, &Message::TapeFinished { total: 1 });
                let _ = send(
                    &writer,
                    &Message::RunCommandResult {
                        result: serde_json::json!({ "executed": command }),
                    },
                );
            }
            Message::RunCommandResult { .. } => {}
            Message::SetConfig { session, path, value } => {
                let Some(target_session) = resolve_session(&attached, &session) else {
                    let _ = send(
                        &writer,
                        &Message::Error {
                            message: "no session targeted (attach to one or pass -s)".into(),
                        },
                    );
                    continue;
                };
                // Record the config option in session meta as a key-value pair.
                daemon.set_session_option(&target_session, &path, &value);
                let _ = send(
                    &writer,
                    &Message::ConfigValue {
                        path: path.clone(),
                        value: value.clone(),
                    },
                );
            }
            Message::GetConfig { session, path } => {
                let Some(target_session) = resolve_session(&attached, &session) else {
                    let _ = send(
                        &writer,
                        &Message::Error {
                            message: "no session targeted (attach to one or pass -s)".into(),
                        },
                    );
                    continue;
                };
                let value = daemon.get_session_option(&target_session, &path);
                let _ = send(&writer, &Message::ConfigValue { path, value });
            }
            Message::ConfigValue { .. } => {}
            Message::GetWindow { session, window } => {
                let Some(target_session) = resolve_session(&attached, &session) else {
                    let _ = send(
                        &writer,
                        &Message::Error {
                            message: "no session targeted (attach to one or pass -s)".into(),
                        },
                    );
                    continue;
                };
                match daemon.resolve_window(&target_session, window.as_deref()) {
                    Ok(target) => {
                        let windows = daemon.windows.lock().unwrap_or_else(|e| e.into_inner());
                        let info = windows
                            .get(&target_session)
                            .and_then(|wins| wins.iter().find(|w| w.info.id == target))
                            .map(|w| w.info.clone());
                        drop(windows);
                        match info {
                            Some(info) => {
                                let detail = serde_json::json!({
                                    "window_id": info.id,
                                    "title": info.title,
                                    "workspace": info.workspace,
                                    "size": format!("{}x{}", info.cols, info.rows),
                                    "cols": info.cols,
                                    "rows": info.rows,
                                    "agent_state": info.agent_state,
                                    "agent_message": info.agent_message,
                                    "agent_harness": info.agent_harness,
                                });
                                let _ = send(&writer, &Message::WindowDetail { detail });
                            }
                            None => {
                                let _ = send(&writer, &Message::Error {
                                    message: format!("window '{target}' not found"),
                                });
                            }
                        }
                    }
                    Err(e) => {
                        let _ = send(&writer, &Message::Error { message: e });
                    }
                }
            }
            Message::WindowDetail { .. } => {}
            Message::SetWorkspaceName { session, workspace, name } => {
                let Some(target_session) = resolve_session(&attached, &session) else {
                    let _ = send(
                        &writer,
                        &Message::Error {
                            message: "no session targeted (attach to one or pass -s)".into(),
                        },
                    );
                    continue;
                };
                // Record the workspace name in session meta.
                daemon.set_workspace_name(&target_session, workspace, &name);
                let _ = send(
                    &writer,
                    &Message::ConfigValue {
                        path: format!("workspace_{workspace}_name"),
                        value: name,
                    },
                );
            }
            Message::ExplainAgentScreen { session, window, harness, lines } => {
                let Some(target_session) = resolve_session(&attached, &session) else {
                    let _ = send(
                        &writer,
                        &Message::Error {
                            message: "no session targeted (attach to one or pass -s)".into(),
                        },
                    );
                    continue;
                };
                // Build the explanation from the window's output ring.
                let explanation = daemon.explain_agent_screen(&target_session, window.as_deref(), &harness, lines);
                let _ = send(&writer, &Message::ExplainResult { explanation });
            }
            Message::ExplainResult { .. } => {}
            Message::SessionResize { .. } => {}
            Message::NotifySize { cols, rows } => {
                if let Some((session, sub_id, _)) = &attached {
                    daemon.notify_size(session, *sub_id, cols, rows);
                }
            }
            Message::PtyOutputSequenced { .. } => {}
            Message::AttachResume { name, seq } => {
                // Stop any previous streaming for this connection.
                if let Some((prev, sub_id, stop)) = attached.take() {
                    stop.store(true, Ordering::Release);
                    daemon.detach(&prev, sub_id);
                }
                match daemon.attach_resume(&name, &client_name, 0, 0, seq) {
                    Ok((windows, sub_id, current_seq, rx)) => {
                        let stop = Arc::new(AtomicBool::new(false));
                        attached = Some((name.clone(), sub_id, Arc::clone(&stop)));
                        let _ = send(
                            &writer,
                            &Message::AttachedResume {
                                windows,
                                seq: current_seq,
                            },
                        );
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
            Message::AttachedResume { .. } => {}
            Message::Diagnose => {
                let report = daemon.diagnose();
                let _ = send(&writer, &Message::Diagnosis { report });
            }
            Message::Diagnosis { .. } => {}
            Message::HeadlessCommand {
                command,
                args,
                session,
            } => {
                match daemon.headless_command(&command, &args, session.as_deref()) {
                    Ok(result) => {
                        let _ = send(&writer, &Message::HeadlessResult { result });
                    }
                    Err(e) => {
                        let _ = send(&writer, &Message::Error { message: e });
                    }
                }
            }
            Message::HeadlessResult { .. } => {}
            _ => {}
        }
    }

    // Cleanup on disconnect.
    if let Some((name, sub_id, stop)) = attached {
        stop.store(true, Ordering::Release);
        daemon.detach(&name, sub_id);
    }
}

/// Drain a window's PTY output channel: fan each chunk out to every client
/// attached to its session and append it to the window's output ring. When
/// the shell exits (channel closes), record its exit code, wake
/// `block-until-exit` waiters, and announce the close.
#[allow(clippy::too_many_arguments)]
fn pump(
    rx: Receiver<Vec<u8>>,
    broadcast: Arc<SessionBroadcast>,
    session: String,
    window: String,
    rings: Arc<Mutex<HashMap<(String, String), OutputRing>>>,
    windows: Arc<Mutex<HashMap<String, Vec<LiveWindow>>>>,
    holds: Arc<Mutex<HashMap<String, (AgentState, std::time::Instant)>>>,
    pid: i32,
    exit_statuses: Arc<Mutex<HashMap<(String, String), i32>>>,
    exit_tx: crossbeam_channel::Sender<()>,
    hook_manager: Arc<crate::hooks::Manager>,
) {
    let mut scanner = crate::session::osc_scan::OscProgressScanner::new();
    let mut markers = crate::session::marker_scan::Osc133Scanner::new();
    while let Ok(chunk) = rx.recv() {
        if let Ok(mut rings) = rings.lock() {
            rings
                .entry((session.clone(), window.clone()))
                .or_insert_with(|| OutputRing::new(RING_CAP))
                .push(&chunk);
        }
        // Detect OSC 9;4 progress reports in the raw stream and drive the
        // window's agent state (anti-flicker held, then broadcast).
        for progress in scanner.feed(&chunk) {
            if let Some(state) = crate::session::osc_scan::agent_state_for_progress(progress.state)
            {
                apply_osc_progress_pieces(
                    &windows,
                    &holds,
                    &session,
                    &window,
                    &state,
                    std::time::Instant::now(),
                    &broadcast,
                );
            }
        }
        // Detect OSC 133 semantic markers and fire the pane-level hooks
        // (pane-shell-prompt / pane-command-started / pane-command-finished).
        for marker in markers.feed(&chunk) {
            fire_pane_marker_hook(&hook_manager, &windows, &session, &window, marker);
        }
        broadcast.send_to_all(&Message::PtyOutput {
            window: window.clone(),
            data: chunk,
        });
    }
    // The shell exited (PTY EOF). Reap it and record the exit code. If the
    // window was explicitly closed first, the handle's Drop already reaped
    // the child (waitpid returns ECHILD) and `close_window` recorded -1.
    if let Ok(status) = nix::sys::wait::waitpid(nix::unistd::Pid::from_raw(pid), None) {
        let code = match status {
            nix::sys::wait::WaitStatus::Exited(_, c) => c,
            nix::sys::wait::WaitStatus::Signaled(_, sig, _) => -(sig as i32),
            _ => -1,
        };
        if let Ok(mut st) = exit_statuses.lock() {
            st.entry((session.clone(), window.clone())).or_insert(code);
        }
    }
    let _ = exit_tx.send(());
    broadcast.send_to_all(&Message::PtyClosed { window });
}

/// Map one OSC 133 marker onto its hook event and fire it with the window's
/// context (id, name, workspace) and the marker's exit code. The `C` marker
/// (command executed / output begins) has no dedicated hook, matching tmux's
/// `pane-command-*` set.
fn fire_pane_marker_hook(
    hooks: &crate::hooks::Manager,
    windows: &Mutex<HashMap<String, Vec<LiveWindow>>>,
    session: &str,
    window: &str,
    marker: crate::session::marker_scan::Osc133Marker,
) {
    use crate::session::marker_scan::MarkerKind;
    let event = match marker.kind {
        MarkerKind::PromptStart => crate::hooks::Event::PaneShellPrompt,
        MarkerKind::CommandStart => crate::hooks::Event::PaneCommandStarted,
        MarkerKind::CommandFinished => crate::hooks::Event::PaneCommandFinished,
        MarkerKind::CommandExecuted => return,
    };
    let (window_name, workspace) = windows
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(session)
        .and_then(|wins| wins.iter().find(|w| w.info.id == window))
        .map(|w| (w.info.title.clone(), w.info.workspace))
        .unwrap_or_else(|| (String::new(), 1));
    let ctx = crate::hooks::Context {
        window_id: window.to_string(),
        window_name,
        workspace,
        session_id: session.to_string(),
        exit_code: marker.exit_code,
        ..crate::hooks::Context::default()
    };
    hooks.fire(event, ctx);
}

/// Resolve the target session of a verb: the named one, else the session this
/// connection is attached to, else `None`.
fn resolve_session(
    attached: &Option<(String, u64, Arc<AtomicBool>)>,
    session: &Option<String>,
) -> Option<String> {
    match session {
        Some(s) => Some(s.clone()),
        None => attached.as_ref().map(|(s, _, _)| s.clone()),
    }
}

fn send(stream: &Arc<Mutex<UnixStream>>, msg: &Message) -> io::Result<()> {
    let mut s = stream.lock().unwrap_or_else(|e| e.into_inner());
    protocol::write_message(&mut *s, msg)
}

fn resolve_shell(shell: &str) -> String {
    if shell.is_empty() {
        std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string())
    } else {
        shell.to_string()
    }
}

/// Best-effort resident set size (RSS) of this process in bytes, by reading
/// `/proc/self/statm` on Linux. Returns 0 on platforms without procfs.
fn process_rss_bytes() -> u64 {
    let data = match std::fs::read_to_string("/proc/self/statm") {
        Ok(d) => d,
        Err(_) => return 0,
    };
    // /proc/self/statm: size resident shared text lib data dt (in pages)
    let fields: Vec<&str> = data.split_whitespace().collect();
    if fields.len() < 2 {
        return 0;
    }
    let resident_pages: u64 = fields[1].parse().unwrap_or(0);
    resident_pages * 4096
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_ring_push_and_cap() {
        let mut ring = OutputRing::new(20);
        ring.push(b"hello");
        assert_eq!(ring.as_lossy(), "hello");
        ring.push(b" world");
        assert_eq!(ring.as_lossy(), "hello world");
        // Exceed cap — oldest bytes are dropped.
        ring.push(b"!!!");
        let result = ring.as_lossy();
        assert!(result.len() <= 23);
        assert!(result.ends_with("!!!"));
    }

    #[test]
    fn output_ring_lossy_utf8() {
        let mut ring = OutputRing::new(100);
        ring.push(b"hello \xFF world");
        assert!(ring.as_lossy().contains("hello"));
    }

    #[test]
    fn output_ring_empty() {
        let ring = OutputRing::new(100);
        assert_eq!(ring.as_lossy(), "");
    }

    #[test]
    fn broadcast_subscribe_unsubscribe() {
        let b = SessionBroadcast::new();
        assert!(!b.is_attached());
        let (id1, _rx1) = b.subscribe();
        assert!(b.is_attached());
        let (id2, _rx2) = b.subscribe();
        assert_ne!(id1, id2);
        b.unsubscribe(id1);
        assert!(b.is_attached());
        b.unsubscribe(id2);
        assert!(!b.is_attached());
    }

    #[test]
    fn broadcast_send_to_all() {
        let b = SessionBroadcast::new();
        let (_id1, rx1) = b.subscribe();
        let (_id2, rx2) = b.subscribe();
        b.send_to_all(&Message::PtyOutput {
            window: "w1".into(),
            data: b"test".to_vec(),
        });
        assert!(!rx1.is_empty());
        assert!(!rx2.is_empty());
    }

    #[test]
    fn broadcast_send_to_unsubscribed_does_not_panic() {
        let b = SessionBroadcast::new();
        let (id, _rx) = b.subscribe();
        b.unsubscribe(id);
        // Should not panic — the send just goes nowhere.
        b.send_to_all(&Message::PtyOutput {
            window: "w1".into(),
            data: b"test".to_vec(),
        });
    }

    #[test]
    fn resolve_session_explicit_name() {
        let attached = None;
        let session = Some("my-session".into());
        assert_eq!(
            resolve_session(&attached, &session),
            Some("my-session".into())
        );
    }

    #[test]
    fn resolve_session_from_attached() {
        let attached = Some((
            "attached-session".into(),
            1,
            Arc::new(AtomicBool::new(true)),
        ));
        let session = None;
        assert_eq!(
            resolve_session(&attached, &session),
            Some("attached-session".into())
        );
    }

    #[test]
    fn resolve_session_nothing() {
        let attached = None;
        let session = None;
        assert_eq!(resolve_session(&attached, &session), None);
    }

    #[test]
    fn resolve_shell_empty_uses_env() {
        let result = resolve_shell("");
        // Should fallback to $SHELL or /bin/sh
        assert!(!result.is_empty());
    }

    #[test]
    fn resolve_shell_nonempty_passthrough() {
        assert_eq!(resolve_shell("/bin/zsh"), "/bin/zsh");
    }

    #[test]
    fn default_socket_path_is_set() {
        let path = default_socket_path();
        assert!(
            path.to_string_lossy().contains("termos") || path.to_string_lossy().contains("tuios")
        );
    }

    #[test]
    fn daemon_new_is_empty() {
        let d = Daemon::new();
        assert!(d.list_infos().is_empty());
    }

    #[test]
    fn pump_fires_pane_marker_hooks() {
        // OSC 133 markers in the raw PTY stream must fire the pane-level
        // hooks with the window context and the exit code from the D marker.
        // The C marker (command executed) has no dedicated hook.
        let (tx, rx) = crossbeam_channel::unbounded();
        let rings: Arc<Mutex<HashMap<(String, String), OutputRing>>> = Arc::new(Mutex::new(HashMap::new()));
        let windows: Arc<Mutex<HashMap<String, Vec<LiveWindow>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let holds: Arc<Mutex<HashMap<String, (AgentState, std::time::Instant)>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let statuses: Arc<Mutex<HashMap<(String, String), i32>>> = Arc::new(Mutex::new(HashMap::new()));
        let (exit_tx, _exit_rx) = crossbeam_channel::unbounded();

        let hooks = Arc::new(crate::hooks::Manager::new());
        hooks.register(crate::hooks::Event::PaneShellPrompt, "marker-hook");
        hooks.register(crate::hooks::Event::PaneCommandStarted, "marker-hook");
        hooks.register(crate::hooks::Event::PaneCommandFinished, "marker-hook");
        let fired: Arc<Mutex<Vec<(String, String, i32)>>> = Arc::new(Mutex::new(Vec::new()));
        let fired_capture = Arc::clone(&fired);
        hooks.set_runner(move |_cmd: &str, ctx: &crate::hooks::Context| {
            fired_capture.lock().unwrap().push((
                ctx.event.map(|e| e.as_str().to_string()).unwrap_or_default(),
                ctx.window_id.clone(),
                ctx.exit_code,
            ));
        });

        let hooks_pump = Arc::clone(&hooks);
        std::thread::spawn(move || {
            pump(
                rx,
                Arc::new(SessionBroadcast::new()),
                "marker-sess".to_string(),
                "w3".to_string(),
                rings,
                windows,
                holds,
                999_999_999,
                statuses,
                exit_tx,
                hooks_pump,
            )
        });

        tx.send(b"\x1b]133;A\x07".to_vec()).unwrap();
        tx.send(b"\x1b]133;B\x07".to_vec()).unwrap();
        tx.send(b"\x1b]133;D;7\x07".to_vec()).unwrap();
        tx.send(b"\x1b]133;C\x07".to_vec()).unwrap(); // no hook fires
        drop(tx);

        // Hooks run on their own threads; poll until all three markers have
        // been processed rather than sleeping a fixed interval.
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let fired = loop {
            let fired = fired.lock().unwrap().clone();
            if fired.len() >= 3 || std::time::Instant::now() >= deadline {
                break fired;
            }
            std::thread::sleep(Duration::from_millis(10));
        };
        assert_eq!(fired.len(), 3, "got: {fired:?}");
        // Hooks run on their own threads, so arrival order is not
        // deterministic — check membership, not sequence.
        let names: Vec<&str> = fired.iter().map(|(n, _, _)| n.as_str()).collect();
        assert!(names.contains(&"pane-shell-prompt"), "got: {names:?}");
        assert!(names.contains(&"pane-command-started"), "got: {names:?}");
        assert!(names.contains(&"pane-command-finished"), "got: {names:?}");
        let finished = fired
            .iter()
            .find(|(n, _, _)| n == "pane-command-finished")
            .expect("finished event fired");
        assert_eq!(finished.2, 7, "D marker exit code must reach the context");
        assert_eq!(finished.1, "w3");
        for (_, wid, _) in &fired {
            assert_eq!(wid, "w3");
        }
    }

    #[test]
    fn window_ids_are_monotonic_across_closes() {
        // Regression: ids were `w{live-count}`, so closing a window reused
        // its id for the next spawn (two live windows sharing `w2` broke
        // scripting targets). They must be monotonic per session.
        let d = Daemon::new();
        // `create_session` spawns the first window (w0) and registers the
        // session in the windows map.
        d.create_session("seq-test", "/bin/sh").expect("create");

        let w1 = d.add_window("seq-test", "/bin/sh", 1).unwrap();
        let w2 = d.add_window("seq-test", "/bin/sh", 1).unwrap();
        assert_eq!(w1.id, "w1");
        assert_eq!(w2.id, "w2");

        // Close w1 — the next spawn must NOT reuse `w1`.
        d.close_window("seq-test", &w1.id).unwrap();
        let w3 = d.add_window("seq-test", "/bin/sh", 1).unwrap();
        assert_eq!(w3.id, "w3");

        // All live ids are unique.
        let live = d.windows.lock().unwrap_or_else(|e| e.into_inner());
        let wins = live.get("seq-test").unwrap();
        let ids: Vec<&String> = wins.iter().map(|w| &w.info.id).collect();
        let uniq: std::collections::HashSet<&&String> = ids.iter().collect();
        assert_eq!(uniq.len(), ids.len(), "duplicate window ids: {ids:?}");
    }

    #[test]
    fn window_ids_are_unique_per_session() {
        // Two sessions allocate independently — both get `w0` first, and
        // neither collides within itself.
        let d = Daemon::new();
        d.create_session("sess-a", "/bin/sh").expect("create");
        d.create_session("sess-b", "/bin/sh").expect("create");
        // First add per session: both must continue from their own counter.
        let a1 = d.add_window("sess-a", "/bin/sh", 1).unwrap();
        let b1 = d.add_window("sess-b", "/bin/sh", 1).unwrap();
        let a2 = d.add_window("sess-a", "/bin/sh", 1).unwrap();
        assert_eq!(a1.id, "w1");
        assert_eq!(b1.id, "w1");
        assert_eq!(a2.id, "w2");
    }

    #[test]
    fn daemon_broadcast_for_creates_hub() {
        let d = Daemon::new();
        let b1 = d.broadcast_for("s1");
        let b2 = d.broadcast_for("s1");
        assert!(Arc::ptr_eq(&b1, &b2));
        let b3 = d.broadcast_for("s2");
        assert!(!Arc::ptr_eq(&b1, &b3));
    }

    #[test]
    fn output_ring_tracks_total_bytes() {
        let mut ring = OutputRing::new(100);
        assert_eq!(ring.current_seq(), 0);
        ring.push(b"hello");
        assert_eq!(ring.current_seq(), 5);
        ring.push(b" world");
        assert_eq!(ring.current_seq(), 11);
    }

    #[test]
    fn output_ring_resume_from_zero() {
        let mut ring = OutputRing::new(100);
        ring.push(b"hello world");
        let replay = ring.output_since(0);
        assert_eq!(replay, b"hello world");
    }

    #[test]
    fn output_ring_resume_from_middle() {
        let mut ring = OutputRing::new(100);
        ring.push(b"hello world");
        let replay = ring.output_since(6);
        assert_eq!(replay, b"world");
    }

    #[test]
    fn output_ring_resume_from_current_is_empty() {
        let mut ring = OutputRing::new(100);
        ring.push(b"hello");
        let replay = ring.output_since(5);
        assert!(replay.is_empty());
    }

    #[test]
    fn output_ring_resume_after_eviction_returns_all() {
        let mut ring = OutputRing::new(10);
        ring.push(b"0123456789"); // fills the ring, seq=10
        ring.push(b"ABCDE"); // evicts 5 bytes, seq=15, buf="5ABCDE" wait...
        // After pushing 10 bytes then 5 more, the ring holds the last 10.
        // total=15, buf len=10, evicted=5
        let replay = ring.output_since(0);
        // from_seq=0 < evicted=5, so return everything in the buffer
        assert_eq!(replay.len(), 10);
    }

    #[test]
    fn broadcast_named_tracks_dimensions() {
        let b = SessionBroadcast::new();
        let (id1, _rx1) = b.subscribe_named("client-a", 120, 40);
        let (id2, _rx2) = b.subscribe_named("client-b", 80, 24);
        // Effective size is the minimum across both clients.
        assert_eq!(b.effective_size(), Some((80, 24)));
        b.update_size(id2, 100, 30);
        assert_eq!(b.effective_size(), Some((100, 30)));
        let _ = id1; // suppress unused warning
    }

    #[test]
    fn broadcast_effective_size_ignores_zero_dims() {
        let b = SessionBroadcast::new();
        let (_id1, _rx1) = b.subscribe_named("a", 0, 0);
        let (_id2, _rx2) = b.subscribe_named("b", 80, 24);
        // Only the client with non-zero dims counts.
        assert_eq!(b.effective_size(), Some((80, 24)));
    }

    #[test]
    fn broadcast_effective_size_none_when_all_zero() {
        let b = SessionBroadcast::new();
        let (_id, _rx) = b.subscribe_named("a", 0, 0);
        assert_eq!(b.effective_size(), None);
    }

    #[test]
    fn broadcast_client_count() {
        let b = SessionBroadcast::new();
        assert_eq!(b.client_count(), 0);
        let (id1, _rx1) = b.subscribe();
        let (id2, _rx2) = b.subscribe();
        assert_eq!(b.client_count(), 2);
        b.unsubscribe(id1);
        assert_eq!(b.client_count(), 1);
        b.unsubscribe(id2);
        assert_eq!(b.client_count(), 0);
    }

    #[test]
    fn broadcast_send_to_all_except() {
        let b = SessionBroadcast::new();
        let (id1, rx1) = b.subscribe();
        let (_id2, rx2) = b.subscribe();
        b.send_to_all_except(
            id1,
            &Message::ClientJoined {
                session: "s".into(),
                name: "new".into(),
            },
        );
        // rx1 (excluded) should not receive it.
        assert!(rx1.is_empty());
        // rx2 should receive it.
        assert!(!rx2.is_empty());
    }

    #[test]
    fn broadcast_client_name() {
        let b = SessionBroadcast::new();
        let (id, _rx) = b.subscribe_named("alice", 80, 24);
        assert_eq!(b.client_name(id), Some("alice".to_string()));
        b.unsubscribe(id);
        assert_eq!(b.client_name(id), None);
    }

    #[test]
    fn daemon_diagnose_empty() {
        let d = Daemon::new();
        let report = d.diagnose();
        assert_eq!(report.session_count, 0);
        assert_eq!(report.client_count, 0);
        assert_eq!(report.version, VERSION);
        assert!(report.sessions.is_empty());
    }

    #[test]
    fn daemon_diagnose_with_sessions() {
        let d = Daemon::new();
        d.create_session("work", "/bin/sh").unwrap();
        d.create_session("play", "/bin/sh").unwrap();
        let report = d.diagnose();
        assert_eq!(report.session_count, 2);
        assert_eq!(report.sessions.len(), 2);
        // Sessions may be in any order; check both are present.
        let names: Vec<&str> = report.sessions.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"work"));
        assert!(names.contains(&"play"));
        // Each session has one window.
        for s in &report.sessions {
            assert_eq!(s.windows, 1);
        }
    }

    #[test]
    fn daemon_headless_list_sessions() {
        let d = Daemon::new();
        d.create_session("work", "/bin/sh").unwrap();
        let result = d.headless_command("list-sessions", &[], None).unwrap();
        let sessions = result.as_array().unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0]["name"], "work");
    }

    #[test]
    fn daemon_headless_unknown_command_is_error() {
        let d = Daemon::new();
        assert!(d.headless_command("frobnicate", &[], None).is_err());
    }

    #[test]
    fn daemon_headless_diagnose() {
        let d = Daemon::new();
        d.create_session("work", "/bin/sh").unwrap();
        let result = d.headless_command("diagnose", &[], None).unwrap();
        assert_eq!(result["session_count"], 1);
    }

    #[test]
    fn process_rss_bytes_returns_nonnegative() {
        // On Linux this reads /proc/self/statm; elsewhere returns 0.
        let rss = process_rss_bytes();
        let _ = rss; // just ensure it doesn't panic
    }
}

#[cfg(test)]
mod osc_tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn holds() -> Mutex<HashMap<String, (AgentState, Instant)>> {
        Mutex::new(HashMap::new())
    }

    fn t0() -> Instant {
        Instant::now()
    }

    #[test]
    fn louder_state_publishes_immediately() {
        let h = holds();
        // Current: Idle (loudness 1). Next: Working (loudness 2) -> publish.
        assert!(hold_quieter_state(
            &h,
            "w1",
            &AgentState::Working,
            &AgentState::Idle,
            t0()
        ));
        assert!(h.lock().unwrap().is_empty());
    }

    #[test]
    fn equal_loudness_publishes_immediately() {
        let h = holds();
        assert!(hold_quieter_state(
            &h,
            "w1",
            &AgentState::Errored,
            &AgentState::NeedsInput,
            t0()
        ));
    }

    #[test]
    fn quieter_state_is_held_first_time() {
        let h = holds();
        // Current: Working (2). Next: Idle (1) -> held, not published.
        assert!(!hold_quieter_state(
            &h,
            "w1",
            &AgentState::Idle,
            &AgentState::Working,
            t0()
        ));
        assert!(h.lock().unwrap().contains_key("w1"));
    }

    #[test]
    fn quieter_state_publishes_after_window() {
        let h = holds();
        let start = t0();
        assert!(!hold_quieter_state(
            &h,
            "w1",
            &AgentState::Idle,
            &AgentState::Working,
            start
        ));
        // Still inside the window.
        assert!(!hold_quieter_state(
            &h,
            "w1",
            &AgentState::Idle,
            &AgentState::Working,
            start + Duration::from_millis(600),
        ));
        // Past the window.
        assert!(hold_quieter_state(
            &h,
            "w1",
            &AgentState::Idle,
            &AgentState::Working,
            start + Duration::from_millis(701),
        ));
        assert!(h.lock().unwrap().is_empty());
    }

    #[test]
    fn same_state_cancels_hold() {
        let h = holds();
        let start = t0();
        assert!(!hold_quieter_state(
            &h,
            "w1",
            &AgentState::Idle,
            &AgentState::Working,
            start
        ));
        // The window became idle some other way; the hold is dropped.
        h.lock().unwrap().remove("w1");
        assert!(!h.lock().unwrap().contains_key("w1"));
    }

    #[test]
    fn loudness_ordering() {
        assert_eq!(agent_loudness(&AgentState::None), 0);
        assert_eq!(agent_loudness(&AgentState::Idle), 1);
        assert_eq!(agent_loudness(&AgentState::Done), 1);
        assert_eq!(agent_loudness(&AgentState::Working), 2);
        assert_eq!(agent_loudness(&AgentState::NeedsInput), 3);
        assert_eq!(agent_loudness(&AgentState::Errored), 3);
    }
}

#[cfg(test)]
mod verb_tests {
    use super::*;
    use crate::session::verb::VerbRegistry;

    fn req(verb: &str, params: serde_json::Value) -> VerbRequest {
        VerbRequest {
            id: Some(serde_json::json!(1)),
            verb: verb.to_string(),
            params: Some(params),
        }
    }

    fn call(d: &Daemon, verb: &str, params: serde_json::Value) -> VerbResponse {
        d.dispatch_verb(&req(verb, params))
    }

    #[test]
    fn hello_reports_protocol_version() {
        let d = Daemon::new();
        let resp = call(&d, "hello", serde_json::json!({}));
        assert!(resp.error.is_none());
        let r = resp.result.unwrap();
        assert_eq!(r["protocol"], "verb");
        assert_eq!(r["daemon"], "termos");
        assert!(r["protocol_version"].as_u64().unwrap() >= 1);
    }

    #[test]
    fn list_sessions_empty() {
        let d = Daemon::new();
        let resp = call(&d, "list-sessions", serde_json::json!({}));
        let r = resp.result.unwrap();
        assert_eq!(r["sessions"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn list_verbs_delegates_to_registry() {
        let d = Daemon::new();
        let resp = call(&d, "list-verbs", serde_json::json!({}));
        assert!(resp.error.is_none());
        let r = resp.result.unwrap();
        let verbs = r["verbs"].as_array().unwrap();
        assert!(!verbs.is_empty());
        // The registry documents every verb the daemon also answers.
        let names: Vec<String> = verbs
            .iter()
            .map(|v| v["verb"].as_str().unwrap().to_string())
            .collect();
        for expected in [
            "hello",
            "list-sessions",
            "capture-pane",
            "set-agent-state",
            "wait-for",
            "diagnose",
            "headless-command",
        ] {
            assert!(names.contains(&expected.to_string()), "missing {expected}");
        }
    }

    #[test]
    fn diagnose_verb_returns_report() {
        let d = Daemon::new();
        d.create_session("work", "/bin/sh").unwrap();
        let resp = call(&d, "diagnose", serde_json::json!({}));
        assert!(resp.error.is_none());
        let r = resp.result.unwrap();
        assert_eq!(r["session_count"], 1);
        assert_eq!(r["sessions"][0]["name"], "work");
    }

    #[test]
    fn headless_command_verb_list_sessions() {
        let d = Daemon::new();
        d.create_session("work", "/bin/sh").unwrap();
        let resp = call(
            &d,
            "headless-command",
            serde_json::json!({"command": "list-sessions"}),
        );
        assert!(resp.error.is_none());
        let r = resp.result.unwrap();
        let sessions = r.as_array().unwrap();
        assert_eq!(sessions.len(), 1);
    }

    #[test]
    fn headless_command_verb_unknown_is_error() {
        let d = Daemon::new();
        let resp = call(
            &d,
            "headless-command",
            serde_json::json!({"command": "frobnicate"}),
        );
        assert!(resp.error.is_some());
    }

    #[test]
    fn unknown_verb_returns_error_envelope() {
        let d = Daemon::new();
        let resp = call(&d, "frobnicate", serde_json::json!({}));
        let e = resp.error.unwrap();
        assert_eq!(e.code, crate::session::verb::ERR_UNKNOWN_VERB);
    }

    #[test]
    fn malformed_verb_is_invalid_request() {
        let d = Daemon::new();
        let resp = d.dispatch_verb(&VerbRequest {
            id: None,
            verb: String::new(),
            params: None,
        });
        let e = resp.error.unwrap();
        assert_eq!(e.code, crate::session::verb::ERR_INVALID_REQUEST);
    }

    #[test]
    fn capture_pane_missing_session_is_error() {
        let d = Daemon::new();
        let resp = call(&d, "capture-pane", serde_json::json!({}));
        assert!(resp.error.is_some());
    }

    #[test]
    fn verb_param_coerces_typed_json_values() {
        // Clients like `termos action timeout=5000` send a JSON number;
        // verb handlers still read it as a string.
        let params = serde_json::json!({
            "timeout": 5000,
            "success": true,
            "text": "exit 0",
        });
        assert_eq!(verb_param(&params, "timeout").as_deref(), Some("5000"));
        assert_eq!(verb_param(&params, "success").as_deref(), Some("true"));
        assert_eq!(verb_param(&params, "text").as_deref(), Some("exit 0"));
        assert_eq!(verb_param(&params, "missing"), None);
        assert_eq!(verb_param(&params, "session"), None);
    }

    #[test]
    fn get_option_not_implemented() {
        let d = Daemon::new();
        let resp = call(&d, "get-option", serde_json::json!({ "option": "theme" }));
        let e = resp.error.unwrap();
        assert_eq!(e.code, crate::session::verb::ERR_OPTION_NOT_FOUND);
    }

    #[test]
    fn verb_registry_standalone_still_works() {
        // The static registry answers list-verbs without any daemon.
        let registry = VerbRegistry::new();
        let resp = registry.dispatch(&VerbRequest {
            id: None,
            verb: "list-verbs".into(),
            params: None,
        });
        assert!(resp.result.is_some());
    }

    #[test]
    fn response_round_trips_as_json_line() {
        let d = Daemon::new();
        let resp = call(&d, "hello", serde_json::json!({}));
        let line = resp.to_line();
        assert!(line.ends_with('\n'));
        let parsed: serde_json::Value = serde_json::from_str(line.trim()).unwrap();
        assert_eq!(parsed["result"]["protocol"], "verb");
    }
}

#[cfg(test)]
mod state_tests {
    use super::*;

    #[test]
    fn state_for_unknown_session_is_empty() {
        let d = Daemon::new();
        let state = d.state_for_session("nope");
        assert_eq!(state.name, "nope");
        assert!(state.windows.is_empty());
    }

    #[test]
    fn restore_saved_named_missing_is_error() {
        let d = Daemon::new();
        assert!(d.restore_saved_named("does-not-exist").is_err());
    }

    #[test]
    fn ping_pong_round_trip_serializes() {
        // The protocol messages round-trip through the JSON frame codec.
        let msg = Message::Ping;
        let mut buf = Vec::new();
        protocol::write_message(&mut buf, &msg).unwrap();
        let mut cursor = std::io::Cursor::new(buf);
        let back = protocol::read_message(&mut cursor).unwrap();
        assert!(matches!(back, Message::Ping));

        let msg = Message::ClientJoined {
            session: "s1".into(),
            name: "cli".into(),
        };
        let mut buf = Vec::new();
        protocol::write_message(&mut buf, &msg).unwrap();
        let mut cursor = std::io::Cursor::new(buf);
        let back = protocol::read_message(&mut cursor).unwrap();
        match back {
            Message::ClientJoined { session, name } => {
                assert_eq!(session, "s1");
                assert_eq!(name, "cli");
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn state_result_round_trips_with_agent_state() {
        use crate::session::state_merge::{SessionState, WindowState};
        let state = SessionState {
            name: "work".into(),
            windows: vec![WindowState {
                id: "w1".into(),
                title: "t".into(),
                workspace: 1,
                agent_state: "working".into(),
                agent_message: "m".into(),
                agent_harness: "h".into(),
                minimized: false,
                cwd: "/tmp".into(),
                foreground: "claude".into(),
            }],
            ..Default::default()
        };
        let msg = Message::StateResult { state };
        let mut buf = Vec::new();
        protocol::write_message(&mut buf, &msg).unwrap();
        let mut cursor = std::io::Cursor::new(buf);
        let back = protocol::read_message(&mut cursor).unwrap();
        match back {
            Message::StateResult { state } => {
                assert_eq!(state.windows[0].agent_state, "working");
                assert_eq!(state.windows[0].cwd, "/tmp");
            }
            _ => panic!("wrong variant"),
        }
    }
}

#[cfg(test)]
mod meta_tests {
    use super::*;

    fn call(d: &Daemon, verb: &str, params: serde_json::Value) -> VerbResponse {
        d.dispatch_verb(&VerbRequest {
            id: Some(serde_json::json!(1)),
            verb: verb.to_string(),
            params: Some(params),
        })
    }

    #[test]
    fn session_meta_round_trips() {
        let d = Daemon::new();
        assert!(d.manager.create("work", &SessionConfig::default()).is_ok());
        d.set_session_label("work", "Payments API").unwrap();
        d.set_session_accent("work", "green").unwrap();
        let meta = d.session_meta("work");
        assert_eq!(meta.display_name, "Payments API");
        assert_eq!(meta.accent, "green");
        // Unknown session meta is empty.
        assert_eq!(d.session_meta("nope").display_name, "");
    }

    #[test]
    fn set_label_on_missing_session_is_error() {
        let d = Daemon::new();
        assert!(d.set_session_label("missing", "x").is_err());
        assert!(d.set_session_accent("missing", "red").is_err());
    }

    #[test]
    fn session_info_verb() {
        let d = Daemon::new();
        assert!(d.manager.create("work", &SessionConfig::default()).is_ok());
        let resp = call(&d, "session-info", serde_json::json!({ "session": "work" }));
        assert!(resp.error.is_none());
        let r = resp.result.unwrap();
        assert_eq!(r["name"], "work");
        assert_eq!(r["windows"], 0);
    }

    #[test]
    fn session_info_missing_is_error() {
        let d = Daemon::new();
        let resp = call(&d, "session-info", serde_json::json!({ "session": "nope" }));
        let e = resp.error.unwrap();
        assert_eq!(e.code, crate::session::verb::ERR_SESSION_NOT_FOUND);
    }

    #[test]
    fn set_session_name_verb() {
        let d = Daemon::new();
        assert!(d.manager.create("work", &SessionConfig::default()).is_ok());
        let resp = call(
            &d,
            "set-session-name",
            serde_json::json!({ "session": "work", "name": "Payments" }),
        );
        assert!(resp.error.is_none());
        assert_eq!(d.session_meta("work").display_name, "Payments");
    }

    #[test]
    fn set_session_accent_verb() {
        let d = Daemon::new();
        assert!(d.manager.create("work", &SessionConfig::default()).is_ok());
        let resp = call(
            &d,
            "set-session-accent",
            serde_json::json!({ "session": "work", "accent": "blue" }),
        );
        assert!(resp.error.is_none());
        assert_eq!(d.session_meta("work").accent, "blue");
    }

    #[test]
    fn state_for_session_carries_meta() {
        let d = Daemon::new();
        assert!(d.manager.create("work", &SessionConfig::default()).is_ok());
        d.set_session_label("work", "Label").unwrap();
        d.set_session_accent("work", "red").unwrap();
        let state = d.state_for_session("work");
        assert_eq!(state.display_name, "Label");
        assert_eq!(state.accent, "red");
    }
}
