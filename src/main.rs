//! TUIOS — Terminal UI Operating System, ported to Rust.
//!
//! The binary runs inside the existing terminal (like tmux/zellij): it takes
//! over the screen, spawns shell sessions in panes, and manages them with a
//! vim-like modal interface.
//!
//! Subcommands:
//!   tuios daemon            run the session daemon in the foreground
//!   tuios run [name]        start daemon, create/attach a session, run the TUI
//!   tuios attach <name>     attach to an existing session in the TUI
//!   tuios list | ls         list sessions
//!   tuios kill <name>       kill a session
//!   tuios                   legacy single-process mode

use std::collections::HashMap;
use std::io::stdout;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crossbeam_channel::{unbounded, Receiver, RecvTimeoutError, Sender};

use crossterm::event::{poll, read, Event, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

use tuios::app::input::{handle_key, handle_mouse, KeyResult};
use tuios::app::render::render;
use tuios::app::Os;
use tuios::config::userconfig::UserConfig;
use tuios::session::model::{SessionInfo, WindowInfo};
use tuios::session::{self, protocol, Daemon, DaemonClient, Message, RemoteSink};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();

    let args: Vec<String> = std::env::args().collect();
    if args.len() > 1 {
        return dispatch(&args);
    }

    run_local_tui()
}

/// Route a subcommand to its handler.
fn dispatch(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    match args[1].as_str() {
        "daemon" => {
            let daemon = Arc::new(Daemon::new());
            daemon.load_hooks(&UserConfig::load().hooks);
            daemon.restore_saved();
            daemon.run_default()?;
            Ok(())
        }
        "list" | "ls" => cmd_list(),
        "kill" => {
            let name = args.get(2).ok_or("usage: tuios kill <name>")?;
            cmd_kill(name)
        }
        "attach" => {
            let name = args.get(2).ok_or("usage: tuios attach <name>")?;
            cmd_attach(name)
        }
        "run" => {
            let name = args.get(2).map(|s| s.as_str());
            cmd_run(name)
        }
        "set-agent-state" => cmd_set_agent_state(&args[2..]),
        other => Err(format!("unknown command '{other}' (try: daemon, run, attach, list, kill, set-agent-state)").into()),
    }
}

fn cmd_list() -> Result<(), Box<dyn std::error::Error>> {
    let client = DaemonClient::connect()?;
    let sessions = client.list()?;
    if sessions.is_empty() {
        println!("no sessions");
        return Ok(());
    }
    for s in sessions {
        let mut line = format!("{}\t{} window(s)", s.name, s.windows);
        if s.attached {
            line.push_str("\t(attached)");
        }
        if s.restored {
            line.push_str("\t(restored)");
        }
        println!("{line}");
    }
    Ok(())
}

fn cmd_kill(name: &str) -> Result<(), Box<dyn std::error::Error>> {
    let client = DaemonClient::connect()?;
    client.kill(name)?;
    println!("killed session '{name}'");
    Ok(())
}

fn cmd_attach(name: &str) -> Result<(), Box<dyn std::error::Error>> {
    run_remote_tui(name)
}

/// `tuios set-agent-state <state> [-s session] [-w window] [-m message]
/// [--harness H]` — report a pane's agent state to the daemon.
fn cmd_set_agent_state(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let state = args
        .first()
        .ok_or("usage: tuios set-agent-state <state> [-s session] [-w window] [-m message] [--harness H]")?;
    if tuios::app::agent_alert::parse_agent_state(state).is_none() {
        return Err(format!(
            "invalid state '{state}' (valid: {})",
            tuios::app::agent_alert::AGENT_STATE_NAMES.join(", ")
        )
        .into());
    }

    let mut session: Option<String> = None;
    let mut window: Option<String> = None;
    let mut message = String::new();
    let mut harness = String::new();
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "-s" | "--session" => {
                i += 1;
                session = args.get(i).cloned();
            }
            "-w" | "--window" => {
                i += 1;
                window = args.get(i).cloned();
            }
            "-m" | "--message" => {
                i += 1;
                message = args.get(i).cloned().unwrap_or_default();
            }
            "--harness" => {
                i += 1;
                harness = args.get(i).cloned().unwrap_or_default();
            }
            other => return Err(format!("unknown flag '{other}'").into()),
        }
        i += 1;
    }

    // Resolve the target session: named, else the only one, else error.
    let client = DaemonClient::connect()?;
    let session = match session {
        Some(s) => s,
        None => {
            let sessions = client.list()?;
            match sessions.len() {
                1 => sessions[0].name.clone(),
                0 => return Err("no sessions; create one with `tuios run`".into()),
                _ => {
                    return Err("multiple sessions; pass -s <session>".into());
                }
            }
        }
    };

    // Send and wait for the daemon's echo/error.
    client.send(&Message::SetAgentState {
        session: Some(session.clone()),
        window,
        state: state.clone(),
        message,
        harness,
    })?;
    client.set_read_timeout(Duration::from_secs(3))?;
    loop {
        match client.recv() {
            Ok(Message::AgentStateChanged { window, state, .. }) => {
                println!("{session}: {window} → {state}");
                return Ok(());
            }
            Ok(Message::Error { message }) => return Err(message.into()),
            Ok(_) => continue,
            Err(e) => return Err(format!("no reply from daemon: {e}").into()),
        }
    }
}

fn cmd_run(name: Option<&str>) -> Result<(), Box<dyn std::error::Error>> {
    session::ensure_daemon_running()?;
    let client = DaemonClient::connect()?;
    let name = match name {
        Some(n) => {
            if !client.list()?.iter().any(|s| s.name == n) {
                client.new_session(n, "")?;
            }
            n.to_string()
        }
        None => match client.list()?.into_iter().next() {
            Some(s) => s.name,
            None => {
                let n = "session-0".to_string();
                client.new_session(&n, "")?;
                n
            }
        },
    };
    run_remote_tui(&name)
}

// ---------------------------------------------------------------------------
// Remote (daemon) TUI
// ---------------------------------------------------------------------------

/// The per-window output channels the socket reader feeds. Keyed by daemon
/// window id, shared so both the reader thread and the event loop can update
/// it across session switches.
type OutputRegistry = Arc<Mutex<HashMap<String, Sender<Vec<u8>>>>>;

/// A control event routed from the socket reader thread to the event loop.
enum RemoteEvent {
    /// The daemon acknowledged an `Attach`.
    Attached { windows: Vec<WindowInfo> },
    /// The daemon replied to a `List`.
    ListResult { sessions: Vec<SessionInfo> },
    /// A window was spawned in the attached session.
    WindowAdded(WindowInfo),
    /// A window was closed in the attached session.
    WindowClosed(String),
    /// A window's agent state changed (broadcast).
    AgentStateChanged {
        window: String,
        state: String,
        message: String,
        harness: String,
    },
    /// The daemon reported an error.
    Error(String),
}

const ATTACH_TIMEOUT: Duration = Duration::from_secs(5);

/// Run the full remote multiplexer TUI attached to a daemon session. Every
/// daemon window becomes a `Window::remote` pane; input is forwarded to the
/// daemon, output is routed back per-window, and `Ctrl+B S` switches sessions.
fn run_remote_tui(name: &str) -> Result<(), Box<dyn std::error::Error>> {
    session::ensure_daemon_running()?;
    let client = DaemonClient::connect()?;

    // Synchronous initial attach (the reader thread starts after this).
    let mut current = name.to_string();
    let windows = client.attach(&current)?;
    let sessions = client.list()?;

    let config = UserConfig::load();
    let mut os = Os::new(config);
    os.remote_session = Some(current.clone());
    os.remote_sessions = sessions.clone();
    os.fire_attached();

    // All daemon-bound messages flow through one channel so input, resize,
    // and control requests stay ordered.
    let (msg_tx, msg_rx) = unbounded::<Message>();
    os.remote_commands = Some(msg_tx.clone());
    let (event_tx, event_rx) = unbounded::<RemoteEvent>();
    let outputs: OutputRegistry = Arc::new(Mutex::new(HashMap::new()));

    // Writer thread: drain msg_tx and write frames to the daemon.
    {
        let client = client.clone();
        std::thread::spawn(move || {
            while let Ok(msg) = msg_rx.recv() {
                if client.send(&msg).is_err() {
                    break;
                }
            }
        });
    }

    // Reader thread: read frames from the daemon and route output/events.
    spawn_reader(&client, &outputs, &event_tx)?;

    // Register the session's windows as remote panes.
    register_windows(&mut os, &windows, &msg_tx, &outputs);

    // Set up the terminal.
    enable_raw_mode()?;
    let mut stdout = stdout();
    execute!(
        stdout,
        EnterAlternateScreen,
        crossterm::event::EnableMouseCapture
    )?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    set_os_size(&mut os);

    let result = run_remote_event_loop(
        &mut os,
        &mut terminal,
        &event_rx,
        &msg_tx,
        &outputs,
        &mut current,
    );

    // Cleanup: fire the after-detach hook (draining in-flight hooks), detach
    // and restore the terminal.
    os.fire_detached();
    let _ = client.send(&Message::Detach);
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        crossterm::event::DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    result
}

/// Start the socket reader thread. It owns all reads for the lifetime of the
/// TUI; control replies (`Attached`/`ListResult`/`Error`) and window events are
/// routed to `events`, and `PtyOutput` is routed to the matching window's
/// output channel.
fn spawn_reader(
    client: &DaemonClient,
    outputs: &OutputRegistry,
    events: &Sender<RemoteEvent>,
) -> std::io::Result<()> {
    let mut reader = client.reader()?;
    let outputs = Arc::clone(outputs);
    let events = events.clone();
    std::thread::spawn(move || {
        while let Ok(msg) = protocol::read_message(&mut reader) {
            match msg {
                Message::PtyOutput { window, data } => {
                    if let Some(tx) = outputs.lock().unwrap().get(&window).cloned() {
                        let _ = tx.send(data);
                    }
                }
                Message::PtyClosed { window } => {
                    outputs.lock().unwrap().remove(&window);
                    let _ = events.send(RemoteEvent::WindowClosed(window));
                }
                Message::WindowAdded { window } => {
                    let _ = events.send(RemoteEvent::WindowAdded(window));
                }
                Message::WindowClosed { window } => {
                    let _ = events.send(RemoteEvent::WindowClosed(window));
                }
                Message::AgentStateChanged {
                    window,
                    state,
                    message,
                    harness,
                } => {
                    let _ = events.send(RemoteEvent::AgentStateChanged {
                        window,
                        state,
                        message,
                        harness,
                    });
                }
                Message::Attached { windows } => {
                    let _ = events.send(RemoteEvent::Attached { windows });
                }
                Message::ListResult { sessions } => {
                    let _ = events.send(RemoteEvent::ListResult { sessions });
                }
                Message::Error { message } => {
                    let _ = events.send(RemoteEvent::Error(message));
                }
                _ => {}
            }
        }
    });
    Ok(())
}

/// Create a `Window::remote` pane for every daemon window and lay them out in
/// their workspaces.
fn register_windows(
    os: &mut Os,
    windows: &[WindowInfo],
    msg_tx: &Sender<Message>,
    outputs: &OutputRegistry,
) {
    for info in windows {
        let (out_tx, out_rx) = unbounded::<Vec<u8>>();
        outputs.lock().unwrap().insert(info.id.clone(), out_tx);
        let sink = RemoteSink::new(info.id.clone(), msg_tx.clone());
        let direction = os.pending_split.take();
        os.add_remote_window(info.clone(), Box::new(sink), out_rx, direction);
    }
    if let Some(first) = windows.first() {
        os.current_workspace = first.workspace.clamp(1, 9);
        os.focus_first_window();
    }
}

/// The remote event loop: render, poll input, and handle control events and
/// pending session switch/kill actions.
fn run_remote_event_loop(
    os: &mut Os,
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    events: &Receiver<RemoteEvent>,
    msg_tx: &Sender<Message>,
    outputs: &OutputRegistry,
    current: &mut String,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut last_render = Instant::now();
    let frame_budget = Duration::from_millis(16); // ~60 FPS

    loop {
        if handle_pending_actions(os, msg_tx, events, outputs, current)? {
            return Ok(()); // quit requested (e.g. last session killed)
        }

        // Drain control events from the reader thread.
        while let Ok(ev) = events.try_recv() {
            match ev {
                RemoteEvent::WindowAdded(info) => {
                    let (out_tx, out_rx) = unbounded::<Vec<u8>>();
                    outputs.lock().unwrap().insert(info.id.clone(), out_tx);
                    let sink = RemoteSink::new(info.id.clone(), msg_tx.clone());
                    let direction = os.pending_split.take();
                    os.add_remote_window(info, Box::new(sink), out_rx, direction);
                    os.notify("window added", "info");
                }
                RemoteEvent::WindowClosed(id) => {
                    if let Some(index) = os.windows.iter().position(|w| w.id == id) {
                        os.remove_window(index);
                    }
                    outputs.lock().unwrap().remove(&id);
                    os.notify("window closed", "info");
                }
                RemoteEvent::AgentStateChanged {
                    window,
                    state,
                    message,
                    harness,
                } => {
                    os.handle_agent_state_changed(&window, &state, &message, &harness);
                }
                RemoteEvent::Attached { .. } => {} // handled by pending actions
                RemoteEvent::ListResult { sessions } => {
                    os.remote_sessions = sessions;
                }
                RemoteEvent::Error(msg) => {
                    os.notify(msg, "error");
                }
            }
        }

        // Render + input.
        os.tick_agent_alerts();
        os.sync_window_sizes();
        if last_render.elapsed() >= frame_budget {
            terminal.draw(|frame| {
                render(os, frame.buffer_mut());
            })?;
            last_render = Instant::now();
        }

        // Flush host-terminal sequences queued by alerts.
        let host_seq = os.take_host_sequence();
        if !host_seq.is_empty() {
            use std::io::Write;
            let mut stdout = stdout();
            let _ = stdout.write_all(&host_seq);
            let _ = stdout.flush();
        }

        if poll(Duration::from_millis(8))? {
            match read()? {
                Event::Key(key) => {
                    if key.kind != KeyEventKind::Press && key.kind != KeyEventKind::Repeat {
                        continue;
                    }
                    let result = handle_key(os, &key);
                    if result == KeyResult::Quit || os.quitting {
                        break;
                    }
                }
                Event::Resize(cols, rows) => {
                    os.width = cols as i32;
                    os.height = rows as i32;
                    os.sync_window_sizes();
                }
                Event::Mouse(mouse) => {
                    handle_mouse(os, &mouse);
                }
                _ => {}
            }
        }
    }

    Ok(())
}

/// Handle the session-switch and session-kill requests set by the switcher.
/// Returns `Ok(true)` when the TUI should quit.
fn handle_pending_actions(
    os: &mut Os,
    msg_tx: &Sender<Message>,
    events: &Receiver<RemoteEvent>,
    outputs: &OutputRegistry,
    current: &mut String,
) -> Result<bool, Box<dyn std::error::Error>> {
    // Session switch (from the session switcher).
    if let Some(target) = os.pending_switch.take() {
        if &target != current {
            match switch_session(&target, msg_tx, events) {
                Ok(windows) => {
                    outputs.lock().unwrap().clear();
                    os.clear_all_windows();
                    register_windows(os, &windows, msg_tx, outputs);
                    *current = target.clone();
                    os.remote_session = Some(target.clone());
                    os.notify(format!("attached to session {target}"), "info");
                }
                Err(e) => {
                    // Restore streaming to the session we were on.
                    let _ = msg_tx.send(Message::Attach {
                        name: current.clone(),
                    });
                    let _ = wait_for_attached(events);
                    os.notify(format!("switch failed: {e}"), "error");
                }
            }
        } else {
            os.notify(format!("already on session {target}"), "info");
        }
        // Refresh the session list for the switcher.
        let _ = msg_tx.send(Message::List);
    }

    // Session kill (Ctrl+D in the session switcher).
    if let Some(target) = os.pending_kill.take() {
        let _ = msg_tx.send(Message::Kill {
            name: target.clone(),
        });
        let sessions = wait_for_list(events).unwrap_or_else(|_| os.remote_sessions.clone());
        os.remote_sessions = sessions.clone();

        if target == *current {
            // We killed the attached session; switch to another or quit.
            if let Some(next) = sessions.iter().map(|s| s.name.clone()).find(|n| n != current) {
                match switch_session(&next, msg_tx, events) {
                    Ok(windows) => {
                        outputs.lock().unwrap().clear();
                        os.clear_all_windows();
                        register_windows(os, &windows, msg_tx, outputs);
                        *current = next.clone();
                        os.remote_session = Some(next.clone());
                    }
                    Err(e) => os.notify(format!("re-attach failed: {e}"), "error"),
                }
            } else {
                os.remote_session = None;
                os.notify("last session killed — quitting", "info");
                return Ok(true);
            }
        }
    }

    Ok(false)
}

/// Detach from the current session and attach to `target`, waiting for the
/// daemon's `Attached` acknowledgement.
fn switch_session(
    target: &str,
    msg_tx: &Sender<Message>,
    events: &Receiver<RemoteEvent>,
) -> Result<Vec<WindowInfo>, String> {
    let _ = msg_tx.send(Message::Detach);
    let _ = msg_tx.send(Message::Attach {
        name: target.to_string(),
    });
    wait_for_attached(events)
}

/// Block until the next `Attached` event, returning its window list.
fn wait_for_attached(events: &Receiver<RemoteEvent>) -> Result<Vec<WindowInfo>, String> {
    let deadline = Instant::now() + ATTACH_TIMEOUT;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err("timed out waiting for the daemon".into());
        }
        match events.recv_timeout(remaining) {
            Ok(RemoteEvent::Attached { windows }) => return Ok(windows),
            Ok(RemoteEvent::Error(msg)) => return Err(msg),
            Ok(_) => continue,
            Err(RecvTimeoutError::Timeout) => {
                return Err("timed out waiting for the daemon".into())
            }
            Err(RecvTimeoutError::Disconnected) => return Err("event channel closed".into()),
        }
    }
}

/// Block until the next `ListResult` event, returning the session list.
fn wait_for_list(events: &Receiver<RemoteEvent>) -> Result<Vec<SessionInfo>, String> {
    let deadline = Instant::now() + ATTACH_TIMEOUT;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err("timed out waiting for the daemon".into());
        }
        match events.recv_timeout(remaining) {
            Ok(RemoteEvent::ListResult { sessions }) => return Ok(sessions),
            Ok(RemoteEvent::Error(msg)) => return Err(msg),
            Ok(_) => continue,
            Err(RecvTimeoutError::Timeout) => {
                return Err("timed out waiting for the daemon".into())
            }
            Err(RecvTimeoutError::Disconnected) => return Err("event channel closed".into()),
        }
    }
}

// ---------------------------------------------------------------------------
// Legacy single-process TUI
// ---------------------------------------------------------------------------

fn run_local_tui() -> Result<(), Box<dyn std::error::Error>> {
    let config = UserConfig::load();
    let mut os = Os::new(config);

    // Set up the terminal.
    enable_raw_mode()?;
    let mut stdout = stdout();
    execute!(
        stdout,
        EnterAlternateScreen,
        crossterm::event::EnableMouseCapture
    )?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    set_os_size(&mut os);

    // Spawn the first shell.
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
    let wake = Box::new(|| {}) as Box<dyn Fn() + Send + 'static>;
    if let Err(e) = os.spawn_window(&shell, wake) {
        eprintln!("failed to spawn shell: {e}");
        disable_raw_mode()?;
        execute!(
            terminal.backend_mut(),
            LeaveAlternateScreen,
            crossterm::event::DisableMouseCapture
        )?;
        return Err(e.into());
    }

    let result = run_event_loop(&mut os, &mut terminal);

    // Restore the terminal.
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        crossterm::event::DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    result
}

/// Resize `os` to the terminal's current size, with a sane headless fallback.
fn set_os_size(os: &mut Os) {
    if let Ok((cols, rows)) = crossterm::terminal::size() {
        os.width = cols.max(1) as i32;
        os.height = rows.max(1) as i32;
    }
    if os.width < 2 {
        os.width = 80;
    }
    if os.height < 2 {
        os.height = 24;
    }
}

fn run_event_loop(
    os: &mut Os,
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut last_render = Instant::now();
    let frame_budget = Duration::from_millis(16); // ~60 FPS

    loop {
        // Raise any agent alerts whose settle window has expired.
        os.tick_agent_alerts();

        // Sync window sizes to the current layout.
        os.sync_window_sizes();

        // Render at most ~60 FPS.
        if last_render.elapsed() >= frame_budget {
            terminal.draw(|frame| {
                render(os, frame.buffer_mut());
            })?;
            last_render = Instant::now();
        }

        // Flush host-terminal sequences queued by alerts (OSC 9 / BEL) after
        // the draw so they never interleave a frame.
        let host_seq = os.take_host_sequence();
        if !host_seq.is_empty() {
            use std::io::Write;
            let mut stdout = stdout();
            let _ = stdout.write_all(&host_seq);
            let _ = stdout.flush();
        }

        // Poll for input.
        if poll(Duration::from_millis(8))? {
            match read()? {
                Event::Key(key) => {
                    if key.kind != KeyEventKind::Press && key.kind != KeyEventKind::Repeat {
                        continue;
                    }
                    let result = handle_key(os, &key);
                    // `Passthrough` means the input layer already forwarded the
                    // encoded bytes to the focused PTY.
                    if result == KeyResult::Quit || os.quitting {
                        break;
                    }
                }
                Event::Resize(cols, rows) => {
                    os.width = cols as i32;
                    os.height = rows as i32;
                    os.sync_window_sizes();
                }
                Event::Mouse(mouse) => {
                    handle_mouse(os, &mouse);
                }
                _ => {}
            }
        }
    }

    Ok(())
}
