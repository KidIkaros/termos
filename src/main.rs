//! TermOS — Terminal UI Operating System, ported to Rust.
//!
//! The binary runs inside the existing terminal (like tmux/zellij): it takes
//! over the screen, spawns shell sessions in panes, and manages them with a
//! vim-like modal interface.
//!
//! Subcommands:
//!   termos daemon            run the session daemon in the foreground
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

use termos::app::effect::Effect;
use termos::app::msg::{from_remote_event, Msg};
use termos::app::render::render;
use termos::app::Os;
use termos::config::userconfig::{Overrides, UserConfig};
use termos::session::model::{SessionInfo, WindowInfo};
use termos::session::remote::RemoteEvent;
use termos::session::{self, protocol, Daemon, DaemonClient, Message, RemoteSink};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let raw_args: Vec<String> = std::env::args().collect();
    let (overrides, remaining) = Overrides::parse(&raw_args[1..]);

    // Initialize logging: daemon modes write to a file, everything else to stderr.
    let is_daemon = remaining
        .first()
        .is_some_and(|c| c == "daemon" || c == "start-server");
    if is_daemon {
        init_daemon_logging();
    } else {
        env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();
    }

    // Write a crash report on panic (Go's WriteCrashLog), so a malformed
    // guest stream or a rare UI branch leaves an artifact instead of nothing.
    let default_panic = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let location = info
            .location()
            .map(|l| format!("{}:{}", l.file(), l.line()))
            .unwrap_or_else(|| "unknown".into());
        let report = format!("panic: {info}\n  at {location}");
        let path = std::env::temp_dir().join(format!("termos-crash-{}.log", std::process::id()));
        let _ = termos::app::interaction::write_crash_report(&path, &report);
        eprintln!(
            "termos: panic at {location} (crash log: {})",
            path.display()
        );
        default_panic(info);
    }));

    if !remaining.is_empty() {
        return dispatch(&remaining, &overrides);
    }

    run_local_tui_with_overrides(&overrides)
}

/// Initialize logging for daemon mode — writes to a file in the state directory.
fn init_daemon_logging() {
    let dir = dirs::state_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("/tmp"))
        .join("termos");
    let _ = std::fs::create_dir_all(&dir);
    let log_path = dir.join("daemon.log");
    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path);
    match file {
        Ok(f) => {
            env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
                .target(env_logger::Target::Pipe(Box::new(f)))
                .init();
        }
        Err(_) => {
            // Fallback to stderr if the log file can't be opened.
            env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
                .init();
        }
    }
}

/// Route a subcommand to its handler.
fn dispatch(args: &[String], _overrides: &Overrides) -> Result<(), Box<dyn std::error::Error>> {
    match args[0].as_str() {
        "--help" | "-h" => {
            print_help();
            Ok(())
        }
        "--version" | "-v" => {
            println!("termos {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        "--skill" => {
            print!("{}", SKILL_DOC);
            Ok(())
        }
        "daemon" => {
            // `daemon --no-restore` skips auto-restoring saved sessions
            // (mirrors Go's daemon flag; `tuios resurrect` restores on demand).
            let no_restore = args.get(2).map(|a| a == "--no-restore").unwrap_or(false);
            let daemon = Arc::new(Daemon::new());
            daemon.load_hooks(&UserConfig::load().hooks);
            if !no_restore {
                daemon.restore_saved();
            } else {
                eprintln!("termos daemon: --no-restore set, skipping session restore");
            }
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
        "run" | "new" => {
            let name = args.get(2).map(|s| s.as_str());
            cmd_run(name)
        }
        "resurrect" => {
            let name = args.get(2).map(|s| s.as_str());
            cmd_resurrect(name)
        }
        "start-server" => {
            let daemon = Arc::new(Daemon::new());
            daemon.load_hooks(&UserConfig::load().hooks);
            daemon.restore_saved();
            daemon.run_default()?;
            Ok(())
        }
        "kill-server" => cmd_kill_server(),
        "session-info" => cmd_session_info(args.get(2).map(|s| s.as_str())),
        "list-windows" => cmd_list_windows(args.get(2).map(|s| s.as_str())),
        "set-session-name" => cmd_set_session_name(&args[2..]),
        "set-session-accent" => cmd_set_session_accent(&args[2..]),
        "logs" => cmd_logs(),
        "layout" => {
            let sub = args.get(2).map(|s| s.as_str()).unwrap_or("list");
            cmd_layout(sub, &args[3..])
        }
        "set-agent-state" => cmd_set_agent_state(&args[2..]),
        "get-agent-state" => cmd_agent_verb(&args[2..], Verb::GetAgentState),
        "send-keys" => cmd_agent_verb(&args[2..], Verb::SendKeys),
        "send-text" => cmd_agent_verb(&args[2..], Verb::SendText),
        "capture-pane" => cmd_agent_verb(&args[2..], Verb::CapturePane),
        "wait-for" => cmd_agent_verb(&args[2..], Verb::WaitFor),
        "list-verbs" => {
            for v in VERBS {
                println!("{v}");
            }
            Ok(())
        }
        "config" => {
            let sub = args.get(2).map(|s| s.as_str()).unwrap_or("show");
            match termos::cli::ConfigCommand::parse(sub) {
                Some(termos::cli::ConfigCommand::Show) => {
                    let path = termos::cli::config_path();
                    let text = std::fs::read_to_string(&path).unwrap_or_default();
                    println!("{}", termos::cli::format_config_show(&text));
                    Ok(())
                }
                Some(termos::cli::ConfigCommand::Path) => {
                    println!("{}", termos::cli::config_path().display());
                    Ok(())
                }
                Some(termos::cli::ConfigCommand::Validate) => {
                    let cfg = UserConfig::load();
                    let result = termos::config::validation::validate_config(&cfg);
                    if result.errors.is_empty() {
                        println!("config OK");
                    } else {
                        for e in &result.errors {
                            println!("error: {}", e.message);
                        }
                    }
                    for w in &result.warnings {
                        println!("warning: {}", w.message);
                    }
                    Ok(())
                }
                Some(termos::cli::ConfigCommand::Edit) => {
                    let path = termos::cli::config_path();

                    // Create config with defaults if it doesn't exist.
                    if !path.exists() {
                        if let Some(parent) = path.parent() {
                            std::fs::create_dir_all(parent)?;
                        }
                        std::fs::write(&path, termos::cli::default_config_toml())?;
                        println!("Created default config at {}", path.display());
                    }

                    // Find an editor: $EDITOR, $VISUAL, then fallback list.
                    let editor = std::env::var("EDITOR")
                        .or_else(|_| std::env::var("VISUAL"))
                        .ok()
                        .or_else(|| {
                            for e in &["vim", "vi", "nano", "emacs"] {
                                if std::process::Command::new("which")
                                    .arg(e)
                                    .output()
                                    .is_ok_and(|o| o.status.success())
                                {
                                    return Some((*e).to_string());
                                }
                            }
                            None
                        })
                        .ok_or("no editor found. Set $EDITOR or install vim/vi/nano/emacs")?;

                    std::process::Command::new(&editor)
                        .arg(&path)
                        .status()?;
                    Ok(())
                }
                Some(termos::cli::ConfigCommand::Reset) => {
                    let path = termos::cli::config_path();
                    if path.exists() {
                        println!("Warning: This will overwrite your existing configuration at:");
                        println!("  {}", path.display());
                        print!("Are you sure you want to reset to defaults? (yes/no): ");
                        std::io::Write::flush(&mut std::io::stdout())?;
                        let mut response = String::new();
                        std::io::stdin().read_line(&mut response)?;
                        let response = response.trim().to_lowercase();
                        if response != "yes" && response != "y" {
                            println!("Reset cancelled.");
                            return Ok(());
                        }
                    }
                    if let Some(parent) = path.parent() {
                        std::fs::create_dir_all(parent)?;
                    }
                    std::fs::write(&path, termos::cli::default_config_toml())?;
                    println!("Configuration reset to defaults at {}", path.display());
                    Ok(())
                }
                None => Err(format!("unknown config command '{sub}'").into()),
            }
        }
        "keybinds" => {
            let sub = args.get(2).map(|s| s.as_str()).unwrap_or("list");
            match termos::cli::KeybindCommand::parse(sub) {
                Some(termos::cli::KeybindCommand::List) => {
                    let registry = termos::config::registry::KeybindRegistry::new();
                    let bindings = termos::config::keybindings::get_prefix_keybindings("", false);
                    let mut entries: Vec<termos::cli::KeybindEntry> = Vec::new();
                    for b in bindings {
                        entries.push(termos::cli::KeybindEntry {
                            key: b.key.clone(),
                            action: registry
                                .get_action(&b.key)
                                .unwrap_or("")
                                .to_string(),
                            description: b.description.clone(),
                        });
                    }
                    print!("{}", termos::cli::format_keybind_list(&entries));
                    Ok(())
                }
                Some(termos::cli::KeybindCommand::Describe) => {
                    let name = args.get(3).ok_or("usage: tuios keybinds describe <action>")?;
                    let registry = termos::config::registry::KeybindRegistry::new();
                    let bindings = termos::config::keybindings::get_prefix_keybindings("", false);
                    let hit = bindings
                        .iter()
                        .find(|b| registry.get_action(&b.key) == Some(name))
                        .or_else(|| bindings.iter().find(|b| &b.key == name));
                    match hit {
                        Some(b) => {
                            println!("{} ({})", b.description, b.key);
                            Ok(())
                        }
                        None => Err(format!("unknown action or key '{name}'").into()),
                    }
                }
                None => Err(format!("unknown keybinds command '{sub}'").into()),
            }
        }
        "tape" => {
            let sub = args.get(2).map(|s| s.as_str()).unwrap_or("");
            match sub {
                "play" => {
                    let file = args.get(3).ok_or("usage: tuios tape play <file.tape>")?;
                    cmd_tape_play(file)
                }
                "validate" => {
                    let file = args.get(3).ok_or("usage: tuios tape validate <file.tape>")?;
                    cmd_tape_validate(file)
                }
                "list" | "ls" => cmd_tape_list(),
                "show" => {
                    let name = args.get(3).ok_or("usage: tuios tape show <name>")?;
                    cmd_tape_show(name)
                }
                "delete" | "rm" => {
                    let name = args.get(3).ok_or("usage: tuios tape delete <name>")?;
                    cmd_tape_delete(name)
                }
                "dir" => {
                    let dir = termos::tape::tapes::tape_dir()?;
                    println!("{}", dir.display());
                    Ok(())
                }
                "exec" => {
                    let mut session: Option<String> = None;
                    let mut file: Option<String> = None;
                    let mut i = 3;
                    while i < args.len() {
                        match args[i].as_str() {
                            "-s" | "--session" => {
                                i += 1;
                                session = args.get(i).cloned();
                            }
                            a if a.starts_with('-') => {
                                return Err(format!("unknown flag '{a}'").into());
                            }
                            a => file = Some(a.to_string()),
                        }
                        i += 1;
                    }
                    let file = file.ok_or("usage: tuios tape exec -s <session> <file.tape>")?;
                    cmd_tape_exec(session.as_deref(), &file)
                }
                other => Err(format!(
                    "unknown tape subcommand '{other}' (try: play, validate, list, show, delete, dir, exec)"
                )
                .into()),
            }
        }
        other => Err(format!("unknown command '{other}' (try: daemon, run, attach, list, kill, set-agent-state, get-agent-state, send-keys, send-text, capture-pane, wait-for, list-verbs, tape)").into()),
    }
}

/// The embedded agent skill document (`tuios --skill`).
const SKILL_DOC: &str = include_str!("../skills/termos/SKILL.md");

/// The verbs the embedded skill documents, in a stable order.
const VERBS: [&str; 7] = [
    "list-verbs",
    "send-keys",
    "send-text",
    "capture-pane",
    "wait-for",
    "get-agent-state",
    "set-agent-state",
];

/// Which agent verb to run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Verb {
    GetAgentState,
    SendKeys,
    SendText,
    CapturePane,
    WaitFor,
}

/// Print CLI usage information.
fn print_help() {
    println!(
        "termos {} — terminal multiplexer and window manager",
        env!("CARGO_PKG_VERSION")
    );
    println!();
    println!("USAGE:");
    println!("    termos [OPTIONS]            Run the TUI");
    println!("    termos <SUBCOMMAND> [ARGS]  Run a subcommand");
    println!();
    println!("OPTIONS:");
    println!("    --border-style <style>     Override border style (rounded/plain/double/none)");
    println!("    --dockbar-position <pos>   Override dockbar position (top/bottom)");
    println!("    --ascii-only               Use ASCII characters instead of Nerd Font icons");
    println!("    --theme <name>             Override theme (e.g. catppuccin-mocha, dracula)");
    println!("    --no-which-key             Disable which-key overlay");
    println!("    --help, -h                 Print this help");
    println!("    --version, -v              Print version");
    println!("    --skill                    Print the agent skill document");
    println!();
    println!("SUBCOMMANDS:");
    println!("    daemon                     Start the session daemon");
    println!("    run, new [name]            Create and attach a new session");
    println!("    attach <name>              Attach to an existing session");
    println!("    list, ls                   List sessions");
    println!("    kill <name>                Kill a session");
    println!("    resurrect [name]           Restore saved session(s)");
    println!("    start-server               Start the daemon (alias for daemon)");
    println!("    kill-server                Stop the daemon");
    println!("    session-info [name]        Show session details");
    println!("    list-windows [session]     List windows in a session");
    println!("    set-session-name <s> <n>   Rename a session");
    println!("    set-session-accent <s> <a> Set session accent color");
    println!("    logs                       Show daemon logs");
    println!("    layout <list|delete|dir|export>  Manage saved layouts");
    println!("    config <show|path|edit|reset|validate>  Config management");
    println!("    keybinds <list|describe>   Keybind reference");
    println!("    tape <play|validate|list|show|delete|dir|exec>  Tape scripting");
    println!("    set-agent-state <args>     Set agent state on a pane");
    println!("    get-agent-state <args>     Get agent state from a pane");
    println!("    send-keys <args>           Send keys to a pane");
    println!("    send-text <args>           Send text to a pane");
    println!("    capture-pane <args>        Capture pane content");
    println!("    wait-for <args>            Wait for a condition");
    println!("    list-verbs                 List control protocol verbs");
    println!();
    println!("EXAMPLES:");
    println!("    termos                         # Start the TUI");
    println!("    termos --theme dracula         # Start with dracula theme");
    println!("    termos --ascii-only            # Start with ASCII-only mode");
    println!("    termos daemon                  # Start the daemon");
    println!("    termos run my-session          # Create and attach a session");
    println!("    termos attach my-session       # Attach to existing session");
    println!("    termos tape play demo.tape     # Play a tape script");
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

/// `tuios tape validate <file.tape>` — parse-check without running.
fn cmd_tape_validate(path: &str) -> Result<(), Box<dyn std::error::Error>> {
    let content =
        std::fs::read_to_string(path).map_err(|e| format!("failed to read tape file: {e}"))?;
    let (commands, parse_errors) = termos::tape::parser::parse_file(&content);
    if !parse_errors.is_empty() {
        eprintln!("Parsing errors found:");
        for err in &parse_errors {
            eprintln!("  ✗ {err}");
        }
        return Err("tape file has parsing errors".into());
    }
    println!("✓ Tape file is valid");
    println!("  File: {path}");
    println!("  Commands: {}", commands.len());
    Ok(())
}

/// `tuios tape list` — list recorded tapes.
fn cmd_tape_list() -> Result<(), Box<dyn std::error::Error>> {
    let dir = termos::tape::tapes::tape_dir()?;
    println!("Tape Recordings");
    println!("Location: {}\n", dir.display());
    let files = termos::tape::tapes::list_tapes()?;
    if files.is_empty() {
        println!("No tape recordings found");
        println!("Use Ctrl+B, T, r in TermOS to start recording");
        return Ok(());
    }
    for f in files {
        let name = f.file_name().unwrap_or_default().to_string_lossy();
        let size = std::fs::metadata(&f).map(|m| m.len()).unwrap_or(0);
        println!("  {name:<40} {size:>8} bytes");
    }
    Ok(())
}

/// `tuios tape show <name>` — print a recording's content.
fn cmd_tape_show(name: &str) -> Result<(), Box<dyn std::error::Error>> {
    let path = termos::tape::tapes::resolve_tape_path(name);
    let content = std::fs::read_to_string(&path).map_err(|_| format!("no such tape: {name}"))?;
    print!("{content}");
    Ok(())
}

/// `tuios tape delete <name>` — delete a recording.
fn cmd_tape_delete(name: &str) -> Result<(), Box<dyn std::error::Error>> {
    let path = termos::tape::tapes::delete_tape(name)?;
    println!("deleted {}", path.display());
    Ok(())
}

/// `tuios tape exec -s <session> <file.tape>` — run a tape headlessly against
/// a running daemon session's attached clients.
fn cmd_tape_exec(session: Option<&str>, path: &str) -> Result<(), Box<dyn std::error::Error>> {
    let script =
        std::fs::read_to_string(path).map_err(|e| format!("failed to read tape file: {e}"))?;
    let (commands, errors) = termos::tape::parser::parse_file(&script);
    if !errors.is_empty() || commands.is_empty() {
        return Err("tape script has no commands or contains errors".into());
    }

    let client = DaemonClient::connect()?;
    let session = match session {
        Some(s) => s.to_string(),
        None => {
            let sessions = client.list()?;
            match sessions.len() {
                1 => sessions[0].name.clone(),
                0 => return Err("no sessions; create one with `tuios run`".into()),
                _ => return Err("multiple sessions; pass -s <session>".into()),
            }
        }
    };
    client.send(&Message::TapeExecute {
        session: session.clone(),
        script,
    })?;
    client.set_read_timeout(Duration::from_secs(60))?;
    loop {
        match client.recv() {
            Ok(Message::Error { message }) => return Err(message.into()),
            Ok(Message::TapeFinished { total }) => {
                println!("{session}: tape finished ({total} commands)");
                return Ok(());
            }
            Ok(_) => continue,
            Err(e) => return Err(format!("no reply from daemon: {e}").into()),
        }
    }
}

/// The trust gate for playing a tape file: trusted tapes run, denied or
/// ineligible tapes explain why, and untrusted tapes prompt before the first
/// run. Returns the tape content when it may run.
fn trust_gate(path: &str) -> Result<String, String> {
    use std::io::Write;
    use termos::tape::trust::Status;

    let mut store = termos::tape::trust::Store::load()?;
    let result = store.check(path)?;
    match result.status {
        Status::Trusted => Ok(String::from_utf8_lossy(&result.content).into_owned()),
        Status::Denied => Err("this tape was denied; move or rename it to run it again".into()),
        Status::Ineligible => Err(format!("tape is ineligible: {}", result.reason)),
        Status::Untrusted => {
            eprintln!(
                "This tape has not been trusted yet: {} (sha256 {})",
                result.path, result.hash
            );
            eprint!("Trust and run it? [y/N/d(eny)] ");
            std::io::stdout().flush().ok();
            let mut answer = String::new();
            std::io::stdin().read_line(&mut answer).ok();
            match answer.trim().to_ascii_lowercase().as_str() {
                "y" | "yes" => {
                    store.trust(&result.path, &result.hash)?;
                    Ok(String::from_utf8_lossy(&result.content).into_owned())
                }
                "d" | "deny" => {
                    store.deny(&result.path)?;
                    Err("tape denied".into())
                }
                _ => Err("aborted".into()),
            }
        }
    }
}

/// `tuios tape play <file.tape>` — run the TUI with the tape driving it.
fn cmd_tape_play(path: &str) -> Result<(), Box<dyn std::error::Error>> {
    // Trust gate: content is the exact hashed bytes, so no re-read after the
    // check (TOCTOU-safe).
    let content = trust_gate(path)?;
    let (commands, parse_errors) = termos::tape::parser::parse_file(&content);
    if !parse_errors.is_empty() {
        eprintln!("Tape parsing errors:");
        for err in &parse_errors {
            eprintln!("  {err}");
        }
        return Err("failed to parse tape file".into());
    }
    if commands.is_empty() {
        return Err("tape has no commands".into());
    }

    println!("Preparing tape script: {path}");
    println!("Total commands: {}", commands.len());
    println!("Press Ctrl+C to cancel, Ctrl+P to pause/resume playback");

    let config = UserConfig::load();
    let mut os = Os::new(config);
    os.init_graphics();
    // Force animations off for deterministic playback (matching recorded
    // tapes, which prepend DisableAnimations).
    os.config.appearance.animations_enabled = false;
    os.script_mode = true;
    os.script_player = Some(termos::tape::player::Player::new(commands));

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
    os.queue_keyboard_enhancements();

    let result = run_event_loop(&mut os, &mut terminal, None);

    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        crossterm::event::DisableMouseCapture
    )?;
    terminal.show_cursor()?;
    result
}

/// `tuios set-agent-state <state> [-s session] [-w window] [-m message]
/// [--harness H]` — report a pane's agent state to the daemon.
fn cmd_set_agent_state(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let state = args.first().ok_or(
        "usage: tuios set-agent-state <state> [-s session] [-w window] [-m message] [--harness H]",
    )?;
    if termos::app::agent_alert::parse_agent_state(state).is_none() {
        return Err(format!(
            "invalid state '{state}' (valid: {})",
            termos::app::agent_alert::AGENT_STATE_NAMES.join(", ")
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

/// Run one of the agent verbs: `[-s session] [-w window] [args...]`.
fn cmd_agent_verb(args: &[String], verb: Verb) -> Result<(), Box<dyn std::error::Error>> {
    let mut session: Option<String> = None;
    let mut window: Option<String> = None;
    let mut positional: Vec<String> = Vec::new();
    let mut i = 0;
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
            a if a.starts_with('-') && a != "-" => {
                return Err(format!("unknown flag '{a}'").into());
            }
            a => positional.push(a.to_string()),
        }
        i += 1;
    }

    let client = DaemonClient::connect()?;
    // Resolve the target session: named, else the only one, else error.
    let session = match session {
        Some(s) => s,
        None => {
            let sessions = client.list()?;
            match sessions.len() {
                1 => sessions[0].name.clone(),
                0 => return Err("no sessions; create one with `tuios run`".into()),
                _ => return Err("multiple sessions; pass -s <session>".into()),
            }
        }
    };

    match verb {
        Verb::GetAgentState => {
            client.send(&Message::GetAgentState {
                session: Some(session.clone()),
                window,
            })?;
            match recv_reply(&client)? {
                Message::AgentStateResult {
                    window,
                    state,
                    message,
                    harness,
                } => {
                    let state = if state.is_empty() { "none" } else { &state };
                    println!("{session}:{window} {state}");
                    if !message.is_empty() {
                        println!("  message: {message}");
                    }
                    if !harness.is_empty() {
                        println!("  harness: {harness}");
                    }
                    Ok(())
                }
                _ => Err("unexpected reply".into()),
            }
        }
        Verb::SendKeys => {
            if positional.is_empty() {
                return Err(
                    "usage: tuios send-keys <key> [key...] (e.g. \"ctrl+b\" \"c\", \"enter\")"
                        .into(),
                );
            }
            let mut data = Vec::new();
            for key in &positional {
                match termos::keys::encode_key_name(key) {
                    Some(bytes) => data.extend_from_slice(&bytes),
                    None => return Err(format!("unknown key '{key}'").into()),
                }
            }
            client.send(&Message::WriteInput {
                session: Some(session.clone()),
                window,
                data,
            })?;
            Ok(())
        }
        Verb::SendText => {
            let text = positional.join(" ");
            if text.is_empty() {
                return Err("usage: tuios send-text <text>".into());
            }
            client.send(&Message::WriteInput {
                session: Some(session.clone()),
                window,
                data: text.into_bytes(),
            })?;
            Ok(())
        }
        Verb::CapturePane => {
            client.send(&Message::CapturePane {
                session: Some(session.clone()),
                window,
            })?;
            match recv_reply(&client)? {
                Message::PaneCapture { content, .. } => {
                    print!("{content}");
                    Ok(())
                }
                _ => Err("unexpected reply".into()),
            }
        }
        Verb::WaitFor => {
            let pattern = positional
                .first()
                .ok_or("usage: tuios wait-for <regex> [timeout_ms]")?
                .clone();
            let timeout_ms = positional
                .get(1)
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(5000);
            client.send(&Message::WaitFor {
                session: Some(session.clone()),
                window,
                pattern,
                timeout_ms,
            })?;
            match recv_reply(&client)? {
                Message::WaitResult { window, matched } => {
                    println!(
                        "{session}:{window} {}",
                        if matched { "matched" } else { "timeout" }
                    );
                    Ok(())
                }
                _ => Err("unexpected reply".into()),
            }
        }
    }
}

/// Format a Unix timestamp as `YYYY-MM-DD HH:MM` (UTC, no external deps).
fn format_unix_timestamp(secs: u64) -> String {
    const DAYS_IN_MONTH: [u64; 12] = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let mut days = secs / 86400;
    let rem = secs % 86400;
    let hour = rem / 3600;
    let minute = (rem % 3600) / 60;

    let mut year = 1970u64;
    loop {
        let leap = (year.is_multiple_of(4) && !year.is_multiple_of(100))
            || year.is_multiple_of(400);
        let year_days = if leap { 366 } else { 365 };
        if days < year_days {
            break;
        }
        days -= year_days;
        year += 1;
    }

    let leap = (year.is_multiple_of(4) && !year.is_multiple_of(100))
        || year.is_multiple_of(400);
    let mut month = 0usize;
    for (i, &dm) in DAYS_IN_MONTH.iter().enumerate() {
        let dm = if i == 1 && leap { 29 } else { dm };
        if days < dm {
            month = i;
            break;
        }
        days -= dm;
    }

    format!("{year:04}-{month:02}-{:02} {hour:02}:{minute:02}", days + 1)
}

/// Read until a non-echo reply (or an error) arrives.
fn recv_reply(client: &DaemonClient) -> Result<Message, Box<dyn std::error::Error>> {
    client.set_read_timeout(Duration::from_secs(10))?;
    match client.recv() {
        Ok(Message::Error { message }) => Err(message.into()),
        Ok(other) => Ok(other),
        Err(e) => Err(format!("no reply from daemon: {e}").into()),
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
    os.init_graphics();
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
    os.queue_keyboard_enhancements();
    os.queue_keyboard_enhancements();

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
                Message::TapeCommand {
                    index,
                    total,
                    command,
                } => {
                    let _ = events.send(RemoteEvent::TapeCommand {
                        index,
                        total,
                        command,
                    });
                }
                Message::TapeFinished { total } => {
                    let _ = events.send(RemoteEvent::TapeFinished { total });
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

    let mut host_buf: Vec<u8> = Vec::new();

    loop {
        // Drain control events from the reader thread.
        while let Ok(ev) = events.try_recv() {
            match ev {
                // These two touch the output-channel registry, so they stay
                // at the loop level rather than becoming `Msg`s.
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
                other => {
                    let effects = os.update(from_remote_event(other));
                    if drain_remote_effects(
                        os,
                        msg_tx,
                        events,
                        outputs,
                        current,
                        effects,
                        &mut host_buf,
                    )? {
                        return Ok(()); // quit requested
                    }
                }
            }
        }

        // Maintenance tick.
        let effects = os.update(Msg::Tick);
        if drain_remote_effects(os, msg_tx, events, outputs, current, effects, &mut host_buf)? {
            return Ok(());
        }

        // Render.
        if last_render.elapsed() >= frame_budget {
            terminal.draw(|frame| {
                render(os, frame.buffer_mut());
            })?;
            last_render = Instant::now();
        }

        // Flush host-terminal sequences queued by alerts after the draw so
        // they never interleave a frame.
        if !host_buf.is_empty() {
            use std::io::Write;
            let mut stdout = stdout();
            let _ = stdout.write_all(&host_buf);
            let _ = stdout.flush();
            host_buf.clear();
        }

        // Poll for input.
        if poll(Duration::from_millis(8))? {
            match read()? {
                Event::Key(key) => {
                    if key.kind == KeyEventKind::Release {
                        os.update(Msg::KeyRelease(key));
                        continue;
                    }
                    if key.kind != KeyEventKind::Press && key.kind != KeyEventKind::Repeat {
                        continue;
                    }
                    let effects = os.update(Msg::Key(key));
                    if drain_remote_effects(
                        os,
                        msg_tx,
                        events,
                        outputs,
                        current,
                        effects,
                        &mut host_buf,
                    )? {
                        return Ok(());
                    }
                }
                Event::Resize(cols, rows) => {
                    os.update(Msg::Resize { cols, rows });
                }
                Event::Mouse(mouse) => {
                    os.update(Msg::Mouse(mouse));
                }
                _ => {}
            }
        }
    }
}

/// Execute effects produced by `Os::update` in the remote loop.
///
/// Host sequences are accumulated into `host_buf` so the loop can flush them
/// after the frame; session attach/kill requests run their (synchronous)
/// socket flows; `Quit` returns `Ok(true)`.
fn drain_remote_effects(
    os: &mut Os,
    msg_tx: &Sender<Message>,
    events: &Receiver<RemoteEvent>,
    outputs: &OutputRegistry,
    current: &mut String,
    effects: Vec<Effect>,
    host_buf: &mut Vec<u8>,
) -> Result<bool, Box<dyn std::error::Error>> {
    for effect in effects {
        match effect {
            Effect::Quit => return Ok(true),
            Effect::WriteHost(seq) => host_buf.extend_from_slice(&seq),
            Effect::RequestAttach(target) => {
                if execute_attach(os, msg_tx, events, outputs, current, &target)? {
                    return Ok(true);
                }
            }
            Effect::RequestKill(target) => {
                if execute_kill(os, msg_tx, events, outputs, current, &target)? {
                    return Ok(true);
                }
            }
            Effect::None => {}
        }
    }
    Ok(false)
}

/// Attach to `target` (from the session switcher). Returns `Ok(true)` when
/// the TUI should quit.
fn execute_attach(
    os: &mut Os,
    msg_tx: &Sender<Message>,
    events: &Receiver<RemoteEvent>,
    outputs: &OutputRegistry,
    current: &mut String,
    target: &str,
) -> Result<bool, Box<dyn std::error::Error>> {
    if target != current {
        match switch_session(target, msg_tx, events) {
            Ok(windows) => {
                outputs.lock().unwrap().clear();
                os.clear_all_windows();
                register_windows(os, &windows, msg_tx, outputs);
                *current = target.to_string();
                os.remote_session = Some(target.to_string());
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
    Ok(false)
}

/// Kill `target` (Ctrl+D in the session switcher). Returns `Ok(true)` when
/// the TUI should quit (last session killed).
fn execute_kill(
    os: &mut Os,
    msg_tx: &Sender<Message>,
    events: &Receiver<RemoteEvent>,
    outputs: &OutputRegistry,
    current: &mut String,
    target: &str,
) -> Result<bool, Box<dyn std::error::Error>> {
    let _ = msg_tx.send(Message::Kill {
        name: target.to_string(),
    });
    let sessions = wait_for_list(events).unwrap_or_else(|_| os.remote_sessions.clone());
    os.remote_sessions = sessions.clone();

    // Kill-and-quit from the quit menu: quit the client even when other
    // sessions exist.
    let kill_and_quit = std::mem::take(&mut os.quit_after_kill);

    if target == current {
        // We killed the attached session; switch to another or quit.
        if kill_and_quit {
            os.remote_session = None;
            os.notify(format!("session '{target}' killed — quitting"), "info");
            return Ok(true);
        }
        if let Some(next) = sessions
            .iter()
            .map(|s| s.name.clone())
            .find(|n| n != current)
        {
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
            Err(RecvTimeoutError::Timeout) => return Err("timed out waiting for the daemon".into()),
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
            Err(RecvTimeoutError::Timeout) => return Err("timed out waiting for the daemon".into()),
            Err(RecvTimeoutError::Disconnected) => return Err("event channel closed".into()),
        }
    }
}

// ---------------------------------------------------------------------------
// Legacy single-process TUI
// ---------------------------------------------------------------------------

fn run_local_tui_with_overrides(overrides: &Overrides) -> Result<(), Box<dyn std::error::Error>> {
    let mut config = UserConfig::load();
    overrides.apply(&mut config);
    let mut os = Os::new(config);
    os.init_graphics();

    // Spawn a config file watcher for hot-reload.
    let config_rx = UserConfig::config_path().and_then(|p| UserConfig::watch(p).ok());

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
    os.queue_keyboard_enhancements();
    os.queue_mac_option_advice();

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

    let result = run_event_loop(&mut os, &mut terminal, config_rx);

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
    config_rx: Option<std::sync::mpsc::Receiver<UserConfig>>,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut last_render = Instant::now();
    let frame_budget = Duration::from_millis(16); // ~60 FPS
    let mut host_buf: Vec<u8> = Vec::new();

    loop {
        // Hot-reload config if the watcher sent an update.
        if let Some(rx) = &config_rx {
            if let Ok(new_config) = rx.try_recv() {
                let effects = os.update(Msg::ConfigReloaded(Box::new(new_config)));
                if drain_local_effects(effects, &mut host_buf) {
                    break;
                }
            }
        }

        // Maintenance tick: agent alerts, script playback, layout sync.
        let effects = os.update(Msg::Tick);
        if drain_local_effects(effects, &mut host_buf) {
            break;
        }

        // Render at most ~60 FPS.
        if last_render.elapsed() >= frame_budget {
            terminal.draw(|frame| {
                render(os, frame.buffer_mut());
            })?;
            last_render = Instant::now();
        }

        // Flush host-terminal sequences queued by alerts (OSC 9 / BEL) after
        // the draw so they never interleave a frame.
        if !host_buf.is_empty() {
            use std::io::Write;
            let mut stdout = stdout();
            let _ = stdout.write_all(&host_buf);
            let _ = stdout.flush();
            host_buf.clear();
        }

        // Poll for input.
        if poll(Duration::from_millis(8))? {
            match read()? {
                Event::Key(key) => {
                    if key.kind == KeyEventKind::Release {
                        // Releases only end a hold; the press was already
                        // forwarded when it went down.
                        os.update(Msg::KeyRelease(key));
                        continue;
                    }
                    if key.kind != KeyEventKind::Press && key.kind != KeyEventKind::Repeat {
                        continue;
                    }
                    let effects = os.update(Msg::Key(key));
                    // `Passthrough` means the input layer already forwarded the
                    // encoded bytes to the focused PTY.
                    if drain_local_effects(effects, &mut host_buf) {
                        break;
                    }
                }
                Event::Resize(cols, rows) => {
                    os.update(Msg::Resize { cols, rows });
                }
                Event::Mouse(mouse) => {
                    os.update(Msg::Mouse(mouse));
                }
                _ => {}
            }
        }
    }

    Ok(())
}

/// Execute effects produced by `Os::update` in the local loop.
///
/// Host sequences are accumulated so the loop can flush them after the draw;
/// session attach/kill requests never occur in local mode and are ignored.
/// Returns `true` when the application should quit.
fn drain_local_effects(effects: Vec<Effect>, host_buf: &mut Vec<u8>) -> bool {
    let mut quit = false;
    for effect in effects {
        match effect {
            Effect::Quit => quit = true,
            Effect::WriteHost(seq) => host_buf.extend_from_slice(&seq),
            Effect::RequestAttach(_) | Effect::RequestKill(_) => {}
            Effect::None => {}
        }
    }
    quit
}

// ─── New CLI commands ────────────────────────────────────────────────────

fn cmd_resurrect(name: Option<&str>) -> Result<(), Box<dyn std::error::Error>> {
    let client = DaemonClient::connect()?;
    if let Some(n) = name {
        client.send(&Message::Resurrect {
            name: n.to_string(),
        })?;
        let _ = recv_reply(&client)?;
        println!("resurrected session '{n}'");
    } else {
        client.send(&Message::ResurrectAll)?;
        let _ = recv_reply(&client)?;
        println!("resurrected all saved sessions");
    }
    Ok(())
}

fn cmd_kill_server() -> Result<(), Box<dyn std::error::Error>> {
    let client = DaemonClient::connect()?;
    client.send(&Message::KillServer)?;
    // The daemon sends Pong before shutting down; read but don't fail on error
    // (the daemon may close the connection before we read).
    let _ = client.recv();
    println!("daemon stopped");
    Ok(())
}

fn cmd_session_info(name: Option<&str>) -> Result<(), Box<dyn std::error::Error>> {
    let client = DaemonClient::connect()?;
    let sessions = client.list()?;
    let target = match name {
        Some(n) => sessions.into_iter().find(|s| s.name == n),
        None => sessions.into_iter().next(),
    };
    match target {
        Some(s) => {
            println!("Session: {}", s.name);
            println!("  Windows: {}", s.windows);
            println!("  Created: {}", format_unix_timestamp(s.created_at));
            println!("  Attached: {}", s.attached);
            println!("  Restored: {}", s.restored);
            Ok(())
        }
        None => Err("no session found".into()),
    }
}

fn cmd_list_windows(name: Option<&str>) -> Result<(), Box<dyn std::error::Error>> {
    let client = DaemonClient::connect()?;
    let sessions = client.list()?;
    let target = match name {
        Some(n) => sessions.into_iter().find(|s| s.name == n),
        None => sessions.into_iter().next(),
    };
    match target {
        Some(s) => {
            println!("Session '{}' has {} window(s)", s.name, s.windows);
            // Window details require attaching; the list endpoint only gives a count.
            Ok(())
        }
        None => Err("no session found".into()),
    }
}

fn cmd_set_session_name(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let session = args
        .first()
        .ok_or("usage: tuios set-session-name <session> <new-name>")?;
    let new_name = args
        .get(1)
        .ok_or("usage: tuios set-session-name <session> <new-name>")?;
    let client = DaemonClient::connect()?;
    client.send(&Message::SetSessionName {
        session: session.clone(),
        name: new_name.clone(),
    })?;
    let _ = recv_reply(&client)?;
    println!("renamed session '{session}' to '{new_name}'");
    Ok(())
}

fn cmd_set_session_accent(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let session = args
        .first()
        .ok_or("usage: tuios set-session-accent <session> <accent>")?;
    let accent = args
        .get(1)
        .ok_or("usage: tuios set-session-accent <session> <accent>")?;
    let client = DaemonClient::connect()?;
    client.send(&Message::SetSessionAccent {
        session: session.clone(),
        accent: accent.clone(),
    })?;
    let _ = recv_reply(&client)?;
    println!("set accent for session '{session}' to '{accent}'");
    Ok(())
}

fn cmd_logs() -> Result<(), Box<dyn std::error::Error>> {
    let path = dirs::state_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("/tmp"))
        .join("termos")
        .join("daemon.log");
    if path.exists() {
        let content = std::fs::read_to_string(&path)?;
        print!("{content}");
    } else {
        println!("(no daemon log at {})", path.display());
    }
    Ok(())
}

fn cmd_layout(sub: &str, args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    use termos::app::layout_templates;
    match sub {
        "list" | "ls" => {
            let templates = layout_templates::list_layout_templates();
            if templates.is_empty() {
                println!(
                    "No saved layouts. Use 'tuios layout save <name>' or the command palette."
                );
                return Ok(());
            }
            for (name, created_at) in templates {
                println!("  {name}  (created: {})", format_unix_timestamp(created_at));
            }
            Ok(())
        }
        "delete" | "rm" => {
            let name = args.first().ok_or("usage: tuios layout delete <name>")?;
            layout_templates::delete_layout_template(name)
                .map_err(|e| -> Box<dyn std::error::Error> { e.into() })?;
            println!("Deleted layout '{name}'");
            Ok(())
        }
        "dir" => {
            println!("{}", layout_templates::layouts_dir().display());
            Ok(())
        }
        "export" => {
            let name = args.first().ok_or("usage: tuios layout export <name>")?;
            let tmpl = layout_templates::load_layout_template(name)
                .map_err(|e| -> Box<dyn std::error::Error> { e.into() })?;
            print!("{}", layout_templates::generate_tape_script(&tmpl));
            Ok(())
        }
        other => Err(format!(
            "unknown layout subcommand '{other}' (try: list, delete, dir, export)"
        )
        .into()),
    }
}

#[cfg(test)]
mod tests {
    /// The embedded skill must match the on-disk file, so the printed copy
    /// always matches the build (mirrors Go's `skill_test.go`).
    #[test]
    fn embedded_skill_matches_disk() {
        let on_disk = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/skills/termos/SKILL.md"
        ))
        .expect("read skills/termos/SKILL.md");
        assert_eq!(
            super::SKILL_DOC,
            on_disk,
            "the embedded skill differs from skills/termos/SKILL.md"
        );
    }
}
