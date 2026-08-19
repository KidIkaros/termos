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
use std::io::{stdout, Write};
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
use termos::config::overrides::Overrides;
use termos::config::userconfig::UserConfig;
use termos::session::model::{SessionInfo, WindowInfo};
use termos::session::remote::RemoteEvent;
use termos::session::{self, protocol, Daemon, DaemonClient, Message, RemoteSink};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let raw_args: Vec<String> = std::env::args().collect();
    let (overrides, remaining) = Overrides::parse(&raw_args[1..]);

    // Initialize logging: daemon modes write to a file, everything else to stderr.
    // `--debug` and `--log-level` override the default filter.
    let is_daemon = remaining
        .first()
        .is_some_and(|c| c == "daemon" || c == "start-server");
    let log_filter = if let Some(ref level) = overrides.log_level {
        level.clone()
    } else if overrides.debug == Some(true) {
        "debug".to_string()
    } else if is_daemon {
        "info".to_string()
    } else {
        String::from("warn")
    };
    if is_daemon {
        init_daemon_logging(&log_filter);
    } else {
        env_logger::Builder::from_env(env_logger::Env::default().default_filter_or(&log_filter))
            .init();
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

    // Handle root-level action flags that print and exit.
    if overrides.list_themes == Some(true) {
        let themes = termos::config::theme::list_theme_names();
        for t in themes {
            println!("{t}");
        }
        return Ok(());
    }
    if let Some(ref name) = overrides.preview_theme {
        return preview_theme_colors(name);
    }

    if !remaining.is_empty() {
        return dispatch(&remaining, &overrides);
    }

    run_local_tui_with_overrides(&overrides)
}

/// Initialize logging for daemon mode — writes to a file in the state directory.
fn init_daemon_logging(filter: &str) {
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
            env_logger::Builder::from_env(env_logger::Env::default().default_filter_or(filter))
                .target(env_logger::Target::Pipe(Box::new(f)))
                .init();
        }
        Err(_) => {
            // Fallback to stderr if the log file can't be opened.
            env_logger::Builder::from_env(env_logger::Env::default().default_filter_or(filter))
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
        "--list-themes" => {
            let themes = termos::config::theme::list_theme_names();
            for t in themes {
                println!("{t}");
            }
            Ok(())
        }
        "daemon" => {
            // `daemon --no-restore` skips auto-restoring saved sessions
            // (mirrors Go's daemon flag; `tuios resurrect` restores on demand).
            // `daemon --log-level <level>` sets the debug log level.
            let daemon_opts = termos::cli::parse_daemon_args(&args[1..])?;
            let daemon = Arc::new(Daemon::new());
            daemon.load_hooks(&UserConfig::load().hooks);
            if !daemon_opts.no_restore {
                daemon.restore_saved();
            } else {
                eprintln!("termos daemon: --no-restore set, skipping session restore");
            }
            daemon.run_default()?;
            Ok(())
        }
        "list" | "ls" => cmd_list(&args[1..]),
        "kill" => {
            let name = args.get(1).ok_or("usage: tuios kill <name>")?;
            cmd_kill(name)
        }
        "attach" => {
            let mut name: Option<&str> = None;
            let mut create = false;
            let mut i = 1;
            while i < args.len() {
                match args[i].as_str() {
                    "-c" | "--create" => create = true,
                    "-s" | "--session" => {
                        i += 1;
                        name = args.get(i).map(|s| s.as_str());
                    }
                    a if a.starts_with('-') => {
                        return Err(format!("unknown flag '{a}'").into());
                    }
                    a => name = Some(a),
                }
                i += 1;
            }
            cmd_attach(name, create)
        }
        "run" | "new" => {
            let mut name: Option<&str> = None;
            let mut detach = false;
            let mut i = 1;
            while i < args.len() {
                match args[i].as_str() {
                    "-d" | "--detach" => detach = true,
                    "-s" | "--session" => {
                        i += 1;
                        name = args.get(i).map(|s| s.as_str());
                    }
                    a if a.starts_with('-') => {
                        return Err(format!("unknown flag '{a}'").into());
                    }
                    a => name = Some(a),
                }
                i += 1;
            }
            if detach {
                cmd_new_detached(name)
            } else {
                cmd_run(name)
            }
        }
        "resurrect" => {
            let name = args.get(1).map(|s| s.as_str());
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
        "session-info" => cmd_session_info(&args[1..]),
        "list-windows" => cmd_list_windows(&args[1..]),
        "set-session-name" => cmd_set_session_name(&args[1..]),
        "set-session-accent" => cmd_set_session_accent(&args[1..]),
        "logs" => cmd_logs(&args[1..]),
        "layout" => {
            let sub = args.get(1).map(|s| s.as_str()).unwrap_or("list");
            cmd_layout(sub, &args[2..])
        }
        "set-agent-state" => cmd_set_agent_state(&args[1..]),
        "get-agent-state" => cmd_agent_verb(&args[1..], Verb::GetAgentState),
        "send-keys" => cmd_agent_verb(&args[1..], Verb::SendKeys),
        "send-text" => cmd_agent_verb(&args[1..], Verb::SendText),
        "capture-pane" => cmd_agent_verb(&args[1..], Verb::CapturePane),
        "wait-for" => cmd_agent_verb(&args[1..], Verb::WaitFor),
        "list-verbs" => cmd_list_verbs(&args[1..]),
        "action" => cmd_action(&args[1..]),
        "subscribe" => cmd_subscribe(&args[1..]),
        "block-until-exit" => cmd_block_until_exit(&args[1..]),
        "exec" => cmd_exec(&args[1..]),
        "ssh" => cmd_ssh(&args[1..]),
        "new-window" => cmd_new_window(&args[1..]),
        "run-command" => cmd_run_command(&args[1..]),
        "set-config" => cmd_set_config(&args[1..]),
        "get-config" => cmd_get_config(&args[1..]),
        "explain-agent-screen" => cmd_explain_agent_screen(&args[1..]),
        "set-workspace-name" => cmd_set_workspace_name(&args[1..]),
        "get-window" => cmd_get_window(&args[1..]),
        "completion" => {
            let shell = args
                .get(1)
                .ok_or("usage: tuios completion <bash|zsh|fish>")?;
            cmd_completion(shell)
        }
        "config" => {
            let sub = args.get(1).map(|s| s.as_str()).unwrap_or("show");
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
            let sub = args.get(1).map(|s| s.as_str()).unwrap_or("list");
            match sub {
                "list" => {
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
                "list-custom" => cmd_keybinds_list_custom(),
                "describe" => {
                    let name = args.get(2).ok_or("usage: tuios keybinds describe <action>")?;
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
                other => Err(format!("unknown keybinds command '{other}'").into()),
            }
        }
        "tape" => {
            let sub = args.get(1).map(|s| s.as_str()).unwrap_or("");
            match sub {
                "play" => {
                    let file = args.get(2).ok_or("usage: tuios tape play <file.tape>")?;
                    cmd_tape_play(file)
                }
                "validate" => {
                    let file = args.get(2).ok_or("usage: tuios tape validate <file.tape>")?;
                    cmd_tape_validate(file)
                }
                "list" | "ls" => cmd_tape_list(),
                "show" => {
                    let name = args.get(2).ok_or("usage: tuios tape show <name>")?;
                    cmd_tape_show(name)
                }
                "delete" | "rm" => {
                    let name = args.get(2).ok_or("usage: tuios tape delete <name>")?;
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
                    let mut i = 2;
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
        other => Err(format!("unknown command '{other}' (try: daemon, run, attach, list, kill, exec, ssh, new-window, run-command, set-config, get-config, explain-agent-screen, set-workspace-name, get-window, completion, set-agent-state, get-agent-state, send-keys, send-text, capture-pane, wait-for, list-verbs, action, subscribe, block-until-exit, tape)").into()),
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
    println!("    --list-themes              List available themes and exit");
    println!("    --preview-theme <name>     Preview a theme's colors and exit");
    println!("    --hide-scrollbar           Hide the window scrollbar");
    println!("    --confirm-quit             Always show quit confirmation dialog");
    println!("    --window-title-position <pos>  Window title position (top/bottom/hidden)");
    println!("    --hide-clock               Hide the clock overlay");
    println!("    --show-clock               Show the clock overlay");
    println!("    --show-cpu                 Show CPU graph in the dock");
    println!("    --show-ram                 Show RAM usage in the dock");
    println!("    --shared-borders           Share borders between adjacent tiled windows");
    println!("    --zoom-max-width <n>       Max width in cells for zoom mode (0 = fullscreen)");
    println!("    --debug                    Enable debug logging");
    println!("    --log-level <level>        Set log level (off, error, warn, info, debug, trace)");
    println!("    --show-keys                Enable the showkeys overlay");
    println!("    --help, -h                 Print this help");
    println!("    --version, -v              Print version");
    println!("    --skill                    Print the agent skill document");
    println!();
    println!("SUBCOMMANDS:");
    println!("    daemon                     Start the session daemon");
    println!("    run, new [name] [-d]       Create and attach a new session (-d: detach)");
    println!("    attach [name] [-c]         Attach to a session (-c: create if missing)");
    println!("    list, ls [--json]          List sessions");
    println!("    kill <name>                Kill a session");
    println!("    resurrect [name]           Restore saved session(s)");
    println!("    start-server               Start the daemon (alias for daemon)");
    println!("    kill-server                Stop the daemon");
    println!("    ssh [--host H] [--port P]  Run as SSH server (requires --features network)");
    println!("    session-info [name] [--json]  Show session details");
    println!("    list-windows [session] [--json]  List windows in a session");
    println!("    get-window [id] [--json]   Get detailed window info");
    println!("    set-session-name <s> <n>   Rename a session");
    println!("    set-session-accent <s> <a> Set session accent color");
    println!("    set-workspace-name <ws> [name]  Name a workspace");
    println!("    logs [-n N] [--clear] [-f] [--all]  Show daemon logs");
    println!("    layout <list|delete|dir|export>  Manage saved layouts");
    println!("    config <show|path|edit|reset|validate>  Config management");
    println!("    keybinds <list|list-custom|describe>  Keybind reference");
    println!("    tape <play|validate|list|show|delete|dir|exec>  Tape scripting");
    println!("    set-agent-state <args> [--source S]  Set agent state on a pane");
    println!("    get-agent-state <args> [--json]  Get agent state from a pane");
    println!("    send-keys <args> [-l] [-r] Send keys to a pane");
    println!("    send-text <args>           Send text to a pane");
    println!("    capture-pane <args> [-S] [--ansi] [--lines N]  Capture pane content");
    println!("    wait-for <args> [--idle N] [--timeout N] [--json]  Wait for a condition");
    println!("    list-verbs [verb] [--json] List control protocol verbs");
    println!("    action <verb> [k=v ...] [--json]  Call any control-protocol verb (see docs/CONTROL_SURFACE.md)");
    println!("    subscribe [-s S] [-w W] [--json]  Tail a pane's output stream");
    println!("    block-until-exit [-s S] [-w W] [--success|--failure] [--timeout ms]  Wait for a pane to exit");
    println!("    exec [-s S] [--timeout ms] [--json] [--keep] <cmd...>  Run a command in a session and report output + exit code");
    println!("    new-window [name] [--json]  Open a new window in a session");
    println!("    run-command <cmd> [args] [--list] [--json]  Execute tape commands remotely");
    println!("    set-config <path> <value>  Set a runtime config option");
    println!("    get-config <path>          Get a runtime config option");
    println!("    explain-agent-screen [--harness H] [--lines N]  Explain screen rules");
    println!("    completion <bash|zsh|fish> Generate shell completion scripts");
    println!();
    println!("EXAMPLES:");
    println!("    termos                         # Start the TUI");
    println!("    termos --theme dracula         # Start with dracula theme");
    println!("    termos --ascii-only            # Start with ASCII-only mode");
    println!("    termos --list-themes           # List available themes");
    println!("    termos daemon                  # Start the daemon");
    println!("    termos run my-session          # Create and attach a session");
    println!("    termos run my-session -d       # Create a detached session");
    println!("    termos attach my-session -c    # Attach, creating if missing");
    println!("    termos new-window build        # Open a named window");
    println!("    termos send-keys -l --raw 'echo hello'  # Send literal text to PTY");
    println!("    termos ls --json -W          # List sessions and windows as JSON");
    println!("    termos subscribe -s dev -w w0  # Tail w0's output until exit");
    println!("    termos block-until-exit -s dev -w w0 --failure --timeout 10000");
    println!("    termos capture-pane -S --lines 40  # Capture last 40 lines of scrollback");
    println!("    termos tape play demo.tape     # Play a tape script");
    println!("    termos completion bash         # Generate bash completions");
}

fn cmd_list(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let mut json = false;
    let mut windows = false;
    for a in args {
        match a.as_str() {
            "--json" => json = true,
            "--windows" | "-W" => windows = true,
            a if a.starts_with('-') => return Err(format!("unknown flag '{a}'").into()),
            _ => return Err(format!("unexpected argument '{a}'").into()),
        }
    }
    let client = DaemonClient::connect()?;
    let sessions = client.list()?;
    if json {
        if windows {
            let mut out = serde_json::Map::new();
            let mut list = Vec::new();
            let mut vc = connect_verb_client()?;
            for s in &sessions {
                let info = serde_json::json!({
                    "name": s.name,
                    "id": s.id,
                    "attached": s.attached,
                    "windows": vc
                        .request_json(
                            "list-windows",
                            serde_json::json!({ "session": s.name }),
                        )
                        .map(|w| w.get("windows").cloned().unwrap_or(serde_json::json!([])))
                        .unwrap_or(serde_json::json!([])),
                });
                list.push(info);
            }
            out.insert("sessions".into(), serde_json::Value::Array(list));
            println!("{}", serde_json::to_string_pretty(&serde_json::Value::Object(out))?);
        } else {
            let val = serde_json::json!({ "sessions": sessions });
            println!("{}", serde_json::to_string_pretty(&val)?);
        }
        return Ok(());
    }
    if sessions.is_empty() {
        println!("no sessions");
        return Ok(());
    }
    let mut vc = windows.then_some(()).map(|_| connect_verb_client()).transpose()?;
    for s in sessions {
        let mut line = format!("{}\t{} window(s)", s.name, s.windows);
        if s.attached {
            line.push_str("\t(attached)");
        }
        if s.restored {
            line.push_str("\t(restored)");
        }
        println!("{line}");
        if windows {
            if let Some(vc) = vc.as_mut() {
                if let Ok(w) = vc.request_json(
                    "list-windows",
                    serde_json::json!({ "session": s.name }),
                ) {
                    if let Some(ws) = w.get("windows").and_then(|x| x.as_array()) {
                        for win in ws {
                            let id = win.get("id").and_then(|x| x.as_str()).unwrap_or("?");
                            let title = win.get("title").and_then(|x| x.as_str()).unwrap_or("?");
                            let cols = win.get("cols").and_then(|x| x.as_u64()).unwrap_or(0);
                            let rows = win.get("rows").and_then(|x| x.as_u64()).unwrap_or(0);
                            let ws_n = win.get("workspace").and_then(|x| x.as_i64()).unwrap_or(1);
                            println!("  {id}\t{title}\t{cols}x{rows}\tws {ws_n}");
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

/// `termos action <verb> [key=value ...] [--json]` — drive any verb of the
/// daemon's public control protocol (the same surface `list-verbs`
/// documents). Parameters are passed as `key=value` pairs; values are
/// strings (the daemon coerces as needed).
fn cmd_action(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let mut json = false;
    let mut verb: Option<String> = None;
    let mut params = serde_json::Map::new();
    for a in args {
        match a.as_str() {
            "--json" => json = true,
            a if a.starts_with('-') && a != "-" => {
                return Err(format!("unknown flag '{a}'").into());
            }
            _ => {
                if verb.is_none() {
                    verb = Some(a.to_string());
                } else if let Some((k, v)) = a.split_once('=') {
                    params.insert(k.to_string(), parse_param_value(v));
                } else {
                    return Err(
                        format!("parameters must be key=value pairs (got '{a}')").into(),
                    );
                }
            }
        }
    }
    let verb = verb.ok_or("usage: termos action <verb> [key=value ...] [--json]")?;
    let mut client = connect_verb_client()?;
    let resp = match client.request(&verb, serde_json::Value::Object(params)) {
        Ok(r) => r,
        Err(termos::session::verb_client::VerbClientError::Verb(e)) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
        Err(termos::session::verb_client::VerbClientError::Io(e)) => {
            return Err(e.into());
        }
    };
    print_verb_response(&resp, json);
    Ok(())
}

/// Parse a `key=value` parameter: JSON-shaped values (numbers, booleans,
/// null, arrays, objects, quoted strings) become typed JSON so the daemon
/// receives `timeout=5000` as a number, while anything else stays a plain
/// string (`text=exit 0`, `pattern=test.*`).
fn parse_param_value(v: &str) -> serde_json::Value {
    if v.starts_with('{') || v.starts_with('[') || v.starts_with('"')
        || v == "true" || v == "false" || v == "null"
    {
        return serde_json::from_str(v).unwrap_or_else(|_| serde_json::Value::String(v.into()));
    }
    if let Ok(n) = v.parse::<i64>() {
        return serde_json::Value::from(n);
    }
    if let Ok(f) = v.parse::<f64>() {
        return serde_json::Value::from(f);
    }
    serde_json::Value::String(v.to_string())
}

/// Connect a verb-protocol client, with a helpful message when the daemon
/// is not reachable.
fn connect_verb_client() -> Result<termos::session::VerbClient, Box<dyn std::error::Error>> {
    termos::session::VerbClient::connect().map_err(|e| {
        format!(
            "cannot connect to the daemon at {}: {e}\n\nstart it with `termos daemon` (or point TERMOS_SOCKET at it)",
            termos::session::default_socket_path().display()
        )
        .into()
    })
}

/// Print a verb response: pretty JSON with `--json`, compact JSON otherwise.
/// The caller is responsible for turning verb errors into exit codes.
fn print_verb_response(resp: &termos::session::VerbResponse, json: bool) {
    if let Some(result) = &resp.result {
        if json {
            println!("{}", serde_json::to_string_pretty(result).unwrap_or_default());
        } else {
            println!("{}", serde_json::to_string(result).unwrap_or_default());
        }
    } else if let Some(err) = &resp.error {
        eprintln!("error: {err}");
    }
}

/// `termos subscribe [-s session] [-w window] [--json]` — tail a pane's raw
/// output as it is produced (plain mode prints just the data; `--json` prints
/// each streamed event). Ends when the window's shell exits or is closed.
fn cmd_subscribe(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let mut session: Option<String> = None;
    let mut window: Option<String> = None;
    let mut json = false;
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
            "--json" => json = true,
            a if a.starts_with('-') => {
                return Err(format!("unknown flag '{a}'").into());
            }
            a => {
                return Err(format!("unexpected argument '{a}'").into());
            }
        }
        i += 1;
    }
    let mut params = serde_json::Map::new();
    if let Some(s) = session {
        params.insert("session".into(), serde_json::Value::String(s));
    }
    if let Some(w) = window {
        params.insert("window".into(), serde_json::Value::String(w));
    }
    let mut client = connect_verb_client()?;
    client.stream(
        "subscribe",
        serde_json::Value::Object(params),
        |line| {
            if json {
                println!("{}", serde_json::to_string_pretty(line).unwrap_or_default());
            } else if let Some(data) = line.get("data").and_then(|d| d.as_str()) {
                print!("{data}");
                let _ = std::io::Write::flush(&mut std::io::stdout());
            }
            // Stop after the terminal closed event.
            !line.get("closed").and_then(|c| c.as_bool()).unwrap_or(false)
        },
    )?;
    Ok(())
}

/// `termos block-until-exit [-s session] [-w window] [--success|--failure]
/// [--timeout ms]` — block until the pane's shell exits, then report the
/// exit code. The process exit status is 0 when the requested condition is
/// met, 1 when it is not, and 2 on timeout or error (so scripts can chain
/// retries).
fn cmd_block_until_exit(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let mut session: Option<String> = None;
    let mut window: Option<String> = None;
    let mut want: Option<bool> = None; // None = plain success
    let mut timeout: u64 = 30_000;
    let mut json = false;
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
            "--success" => want = Some(true),
            "--failure" => want = Some(false),
            "--timeout" => {
                i += 1;
                if let Some(v) = args.get(i) {
                    timeout = v.parse().unwrap_or(30_000);
                }
            }
            "--json" => json = true,
            a if a.starts_with('-') => {
                return Err(format!("unknown flag '{a}'").into());
            }
            a => {
                return Err(format!("unexpected argument '{a}'").into());
            }
        }
        i += 1;
    }
    let mut params = serde_json::Map::new();
    if let Some(s) = session {
        params.insert("session".into(), serde_json::Value::String(s));
    }
    if let Some(w) = window {
        params.insert("window".into(), serde_json::Value::String(w));
    }
    params.insert("timeout".into(), serde_json::Value::from(timeout.to_string()));

    let mut client = connect_verb_client()?;
    let result = match client.request_json("block-until-exit", serde_json::Value::Object(params)) {
        Ok(r) => r,
        Err(termos::session::verb_client::VerbClientError::Verb(e)) => {
            eprintln!("error: {e}");
            std::process::exit(2);
        }
        Err(termos::session::verb_client::VerbClientError::Io(e)) => {
            return Err(e.into());
        }
    };
    let exit_code = result.get("exit_code").and_then(|c| c.as_i64()).unwrap_or(-1);
    let success = result.get("success").and_then(|s| s.as_bool()).unwrap_or(false);
    if json {
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else {
        println!("exit code {exit_code}");
    }
    let want = want.unwrap_or(true);
    if success == want {
        std::process::exit(0);
    } else {
        std::process::exit(1);
    }
}

/// `termos exec [-s session] [--timeout ms] [--json] [--keep] <cmd...>`
/// — create a session, run a command in it, and report the output and exit
/// code (tmux `new-window 'cmd'` analogue). The default is a throwaway
/// session (`exec-<pid>`); `-s` reuses a named one (creating it if missing,
/// with a dedicated window for the command). The command runs via the pane
/// shell as `cmd; exit $?`, so the reported exit code is the command's own.
/// The process exits with the command's code (0 = success; a signal maps to
/// 128+signum; 2 = timeout or daemon error).
///
/// Cleanup is deliberately conservative: a session `exec` created itself is
/// killed afterward unless `--keep`; a pre-existing session targeted with
/// `-s` is **never** killed — it belongs to the caller.
/// Parsed `termos exec` flags. `shell`/`cwd` are optional: exec defaults the
/// shell to `/bin/sh` (deterministic scripting) and the cwd to the daemon's
/// working directory unless `--cwd` is given.
#[derive(Debug)]
struct ExecFlags {
    session: Option<String>,
    timeout_ms: u64,
    json: bool,
    keep: bool,
    shell: Option<String>,
    cwd: Option<String>,
    cmd: Vec<String>,
}

fn parse_exec_flags(args: &[String]) -> Result<ExecFlags, String> {
    let mut f = ExecFlags {
        session: None,
        timeout_ms: 30_000,
        json: false,
        keep: false,
        shell: None,
        cwd: None,
        cmd: Vec::new(),
    };
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-s" | "--session" => {
                i += 1;
                f.session = args.get(i).cloned();
            }
            "--timeout" => {
                i += 1;
                if let Some(v) = args.get(i) {
                    f.timeout_ms = v.parse().unwrap_or(30_000);
                }
            }
            "--json" => f.json = true,
            "--keep" => f.keep = true,
            "--shell" => {
                i += 1;
                f.shell = args.get(i).cloned();
            }
            "--cwd" => {
                i += 1;
                f.cwd = args.get(i).cloned();
            }
            a if a.starts_with('-') && a != "-" => return Err(format!("unknown flag '{a}'")),
            a => f.cmd.push(a.to_string()),
        }
        i += 1;
    }
    Ok(f)
}

fn cmd_exec(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let flags = parse_exec_flags(args).map_err(|e| e.to_string())?;
    let session = flags.session;
    let timeout_ms = flags.timeout_ms;
    let json = flags.json;
    let keep = flags.keep;
    // Deterministic shell by default; `--shell` picks the pane shell.
    let shell = flags.shell.unwrap_or_else(|| "/bin/sh".to_string());
    let cwd = flags.cwd;
    let command = flags.cmd.join(" ");
    if command.is_empty() {
        return Err(
            "usage: tuios exec [-s session] [--shell sh] [--cwd dir] [--timeout ms] [--json] [--keep] <command> [args...]"
                .into(),
        );
    }

    let mut client = connect_verb_client()?;

    // Resolve the target session: named (reuse or create), else a fresh one.
    // `created` is true only when WE created the session — cleanup kills
    // only those; a reused pre-existing session is never touched.
    let fresh_session = session.is_none();
    let (session, created) = match &session {
        Some(s) => {
            let exists = client
                .request_json("list-sessions", serde_json::json!({}))?
                .get("sessions")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .any(|x| x.get("name").and_then(|n| n.as_str()) == Some(s.as_str()))
                })
                .unwrap_or(false);
            if exists {
                (s.clone(), false)
            } else {
                client.request_json(
                    "new-session",
                    serde_json::json!({ "name": s, "shell": shell, "cwd": cwd }),
                )?;
                (s.clone(), true)
            }
        }
        None => {
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.subsec_nanos())
                .unwrap_or(0);
            let name = format!("exec-{}-{nanos}", std::process::id());
            client.request_json(
                "new-session",
                serde_json::json!({ "name": name, "shell": shell, "cwd": cwd }),
            )?;
            (name, true)
        }
    };

    // Window: a fresh session's default window is w0; a reused session gets
    // a dedicated window so we never clobber whatever is running there (and
    // that window is closed again when the command exits).
    let (window, window_created) = if fresh_session {
        ("w0".to_string(), false)
    } else {
        let resp = client.request_json(
            "new-window",
            serde_json::json!({ "session": session, "shell": shell, "cwd": cwd }),
        )?;
        let id = resp
            .get("window")
            .and_then(|w| w.get("id"))
            .and_then(|i| i.as_str())
            .unwrap_or("w0")
            .to_string();
        (id, true)
    };

    // Tail the pane on a second connection (subscribe takes over its
    // connection). Signal on the ack so we send text only once the stream is
    // live — nothing is missed and nothing earlier is replayed.
    let (ack_tx, ack_rx) = unbounded();
    let sub_session = session.clone();
    let sub_window = window.clone();
    let output: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
    let sub_out = Arc::clone(&output);
    let subscriber = std::thread::spawn(move || -> Result<(), String> {
        let mut sub = connect_verb_client().map_err(|e| e.to_string())?;
        sub.stream(
            "subscribe",
            serde_json::json!({ "session": sub_session, "window": sub_window }),
            |line| {
                if line.get("subscribed").and_then(|s| s.as_bool()).unwrap_or(false) {
                    let _ = ack_tx.send(());
                }
                if let Some(d) = line.get("data").and_then(|d| d.as_str()) {
                    sub_out.lock().unwrap_or_else(|e| e.into_inner()).extend_from_slice(d.as_bytes());
                }
                        // Keep streaming until the window closes (shell exited).
                !line.get("closed").and_then(|c| c.as_bool()).unwrap_or(false)
            },
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    });
    match ack_rx.recv_timeout(Duration::from_secs(5)) {
        Ok(()) => {}
        Err(_) => {
            if created && !keep {
                let _ = client.request_json("kill-session", serde_json::json!({ "session": session }));
            }
            return Err("timed out waiting for subscribe ack".into());
        }
    }

    // Run the command; `exit $?` makes the shell report its exit code.
    let text = format!("{command}; exit $?\r");
    client.request_json(
        "send-text",
        serde_json::json!({ "session": session, "window": window, "text": text }),
    )?;

    let exit_code = match client.request_json(
        "block-until-exit",
        serde_json::json!({
            "session": session,
            "window": window,
            "timeout": timeout_ms.to_string(),
        }),
    ) {
        Ok(r) => r.get("exit_code").and_then(|c| c.as_i64()).unwrap_or(-1) as i32,
        Err(termos::session::verb_client::VerbClientError::Verb(e)) => {
            eprintln!("error: {e}");
            if created && !keep {
                let _ = client.request_json("kill-session", serde_json::json!({ "session": session }));
            } else if window_created {
                let _ = client.request_json(
                    "close-window",
                    serde_json::json!({ "session": session, "window": window }),
                );
            }
            std::process::exit(2);
        }
        Err(termos::session::verb_client::VerbClientError::Io(e)) => return Err(e.into()),
    };
    let _ = subscriber.join();

    let out = output.lock().unwrap_or_else(|e| e.into_inner()).clone();
    if json {
        let val = serde_json::json!({
            "session": session,
            "window": window,
            "output": String::from_utf8_lossy(&out),
            "exit_code": exit_code,
        });
        println!("{}", serde_json::to_string_pretty(&val)?);
    } else {
        // Raw pane stream (prompt, echoed command, output) — like tmux.
        let _ = stdout().write_all(&out);
        let _ = stdout().flush();
        println!("exit code {exit_code}");
    }

    // Cleanup: kill the session only if we created it; close the dedicated
    // window (not the session) if we created one inside a reused session.
    if created && !keep {
        let _ = client.request_json("kill-session", serde_json::json!({ "session": session }));
    } else if window_created {
        let _ = client.request_json(
            "close-window",
            serde_json::json!({ "session": session, "window": window }),
        );
    }
    if exit_code >= 0 {
        std::process::exit(exit_code);
    }
    // Signal death: 128 + signum (shell convention; e.g. SIGTERM -> 143).
    std::process::exit(128 + (-exit_code));
}

fn cmd_kill(name: &str) -> Result<(), Box<dyn std::error::Error>> {
    let client = DaemonClient::connect()?;
    client.kill(name)?;
    println!("killed session '{name}'");
    Ok(())
}

fn cmd_attach(name: Option<&str>, create: bool) -> Result<(), Box<dyn std::error::Error>> {
    session::ensure_daemon_running()?;
    let client = DaemonClient::connect()?;
    let name = match name {
        Some(n) => {
            if create && !client.list()?.iter().any(|s| s.name == n) {
                client.new_session(n, "")?;
            }
            n.to_string()
        }
        None => match client.list()?.into_iter().next() {
            Some(s) => s.name,
            None => {
                if create {
                    let n = "session-0".to_string();
                    client.new_session(&n, "")?;
                    n
                } else {
                    return Err("no sessions; create one with `tuios run` or pass -c".into());
                }
            }
        },
    };
    run_remote_tui(&name)
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
/// [--source S] [--harness H]` — report a pane's agent state to the daemon.
fn cmd_set_agent_state(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let state = args.first().ok_or(
        "usage: tuios set-agent-state <state> [-s session] [-w window] [-m message] [--source S] [--harness H]",
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
    let mut _source = String::new();
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
            "--source" => {
                i += 1;
                _source = args.get(i).cloned().unwrap_or_default();
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
    // Verb-specific flags.
    let mut literal = false; // send-keys --literal
    let mut raw = false; // send-keys --raw
    let mut scrollback = false; // capture-pane --scrollback
    let mut _ansi = false; // capture-pane --ansi
    let mut capture_lines: i32 = 0; // capture-pane --lines
    let mut json = false; // get-agent-state/wait-for --json
    let mut idle_ms: u64 = 0; // wait-for --idle
    let mut timeout_ms: u64 = 30_000; // wait-for --timeout
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
            "-l" | "--literal" => literal = true,
            "-r" | "--raw" => raw = true,
            "-S" | "--scrollback" => scrollback = true,
            "--ansi" => _ansi = true,
            "--lines" => {
                i += 1;
                if let Some(v) = args.get(i) {
                    capture_lines = v.parse().unwrap_or(0);
                }
            }
            "--json" => json = true,
            "--idle" => {
                i += 1;
                if let Some(v) = args.get(i) {
                    idle_ms = v.parse().unwrap_or(0);
                }
            }
            "--timeout" => {
                i += 1;
                if let Some(v) = args.get(i) {
                    timeout_ms = v.parse().unwrap_or(30_000);
                }
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
                    if json {
                        let val = serde_json::json!({
                            "window": window,
                            "state": if state.is_empty() { "none" } else { &state },
                            "message": message,
                            "harness": harness,
                        });
                        println!("{}", serde_json::to_string_pretty(&val)?);
                    } else {
                        let state = if state.is_empty() { "none" } else { &state };
                        println!("{session}:{window} {state}");
                        if !message.is_empty() {
                            println!("  message: {message}");
                        }
                        if !harness.is_empty() {
                            println!("  harness: {harness}");
                        }
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
            // --literal: send the raw text directly to the PTY (bypass TUIOS
            // key parsing). --raw: treat each character as a separate key
            // (no splitting on spaces/commas).
            let data: Vec<u8> = if literal {
                let text = positional.join(" ");
                text.into_bytes()
            } else if raw {
                // Each character is a separate key.
                let mut d = Vec::new();
                for key in &positional {
                    for ch in key.chars() {
                        match termos::keys::encode_key_name(&ch.to_string()) {
                            Some(bytes) => d.extend_from_slice(&bytes),
                            None => d.push(ch as u8),
                        }
                    }
                }
                d
            } else {
                let mut d = Vec::new();
                for key in &positional {
                    match termos::keys::encode_key_name(key) {
                        Some(bytes) => d.extend_from_slice(&bytes),
                        None => return Err(format!("unknown key '{key}'").into()),
                    }
                }
                d
            };
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
                    let content = if capture_lines > 0 {
                        let lines: Vec<&str> = content.lines().collect();
                        let start = lines.len().saturating_sub(capture_lines as usize);
                        lines[start..].join("\n")
                    } else if scrollback {
                        // The daemon's ring already includes scrollback.
                        content
                    } else {
                        content
                    };
                    print!("{content}");
                    Ok(())
                }
                _ => Err("unexpected reply".into()),
            }
        }
        Verb::WaitFor => {
            let pattern = positional
                .first()
                .ok_or("usage: tuios wait-for <regex> [--timeout N] [--idle N]")?
                .clone();
            let _ = idle_ms; // idle is accepted; the daemon polls the ring
            client.send(&Message::WaitFor {
                session: Some(session.clone()),
                window,
                pattern,
                timeout_ms,
            })?;
            match recv_reply(&client)? {
                Message::WaitResult { window, matched } => {
                    if json {
                        let val = serde_json::json!({
                            "window": window,
                            "matched": matched,
                        });
                        println!("{}", serde_json::to_string_pretty(&val)?);
                    } else {
                        println!(
                            "{session}:{window} {}",
                            if matched { "matched" } else { "timeout" }
                        );
                    }
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

/// `tuios new -d [name]` — create a headless session without attaching.
fn cmd_new_detached(name: Option<&str>) -> Result<(), Box<dyn std::error::Error>> {
    session::ensure_daemon_running()?;
    let client = DaemonClient::connect()?;
    let name = match name {
        Some(n) => n.to_string(),
        None => {
            let sessions = client.list()?;
            let existing: std::collections::HashSet<&str> =
                sessions.iter().map(|s| s.name.as_str()).collect();
            let mut i = 0;
            loop {
                let candidate = format!("session-{i}");
                if !existing.contains(candidate.as_str()) {
                    break candidate;
                }
                i += 1;
            }
        }
    };
    client.new_session(&name, "")?;
    println!(
        "Created detached session '{name}'. Attach with 'tuios attach {name}'."
    );
    Ok(())
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
                    if let Some(tx) = outputs.lock().unwrap_or_else(|e| e.into_inner()).get(&window).cloned() {
                        let _ = tx.send(data);
                    }
                }
                Message::PtyClosed { window } => {
                    outputs.lock().unwrap_or_else(|e| e.into_inner()).remove(&window);
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
        outputs.lock().unwrap_or_else(|e| e.into_inner()).insert(info.id.clone(), out_tx);
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
                    outputs.lock().unwrap_or_else(|e| e.into_inner()).insert(info.id.clone(), out_tx);
                    let sink = RemoteSink::new(info.id.clone(), msg_tx.clone());
                    let direction = os.pending_split.take();
                    os.add_remote_window(info, Box::new(sink), out_rx, direction);
                    os.notify("window added", "info");
                }
                RemoteEvent::WindowClosed(id) => {
                    if let Some(index) = os.windows.iter().position(|w| w.id == id) {
                        os.remove_window(index);
                    }
                    outputs.lock().unwrap_or_else(|e| e.into_inner()).remove(&id);
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
                outputs.lock().unwrap_or_else(|e| e.into_inner()).clear();
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
                    outputs.lock().unwrap_or_else(|e| e.into_inner()).clear();
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

fn cmd_session_info(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let opts = termos::cli::parse_session_info_args(args);
    let client = DaemonClient::connect()?;
    let sessions = client.list()?;
    let target = match opts.session {
        Some(n) => sessions.into_iter().find(|s| s.name == n),
        None => sessions.into_iter().next(),
    };
    match target {
        Some(s) => {
            if opts.json {
                let val = serde_json::json!({
                    "name": s.name,
                    "id": s.id,
                    "created_at": s.created_at,
                    "attached": s.attached,
                    "windows": s.windows,
                    "restored": s.restored,
                });
                println!("{}", serde_json::to_string_pretty(&val)?);
            } else {
                println!("Session: {}", s.name);
                println!("  Windows: {}", s.windows);
                println!("  Created: {}", format_unix_timestamp(s.created_at));
                println!("  Attached: {}", s.attached);
                println!("  Restored: {}", s.restored);
            }
            Ok(())
        }
        None => Err("no session found".into()),
    }
}

fn cmd_list_windows(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let opts = termos::cli::parse_session_info_args(args);
    let client = DaemonClient::connect()?;
    let sessions = client.list()?;
    let target = match opts.session {
        Some(n) => sessions.into_iter().find(|s| s.name == n),
        None => sessions.into_iter().next(),
    };
    match target {
        Some(s) => {
            if opts.json {
                // Full per-window detail (id, geometry, workspace) comes from
                // the daemon's `list-windows` verb, not the session count.
                let mut vc = connect_verb_client()?;
                let windows = vc
                    .request_json("list-windows", serde_json::json!({ "session": s.name }))
                    .map(|w| w.get("windows").cloned().unwrap_or(serde_json::json!([])))
                    .unwrap_or(serde_json::json!([]));
                let val = serde_json::json!({
                    "session": s.name,
                    "windows": windows,
                });
                println!("{}", serde_json::to_string_pretty(&val)?);
            } else {
                println!("Session '{}' has {} window(s)", s.name, s.windows);
            }
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

fn cmd_logs(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let mut lines: usize = 50;
    let mut clear = false;
    let mut follow = false;
    let mut all = false;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-n" | "--lines" => {
                i += 1;
                if let Some(v) = args.get(i) {
                    lines = v.parse().unwrap_or(50);
                }
            }
            "--clear" => clear = true,
            "-f" | "--follow" => follow = true,
            "--all" => {
                all = true;
                lines = 0;
            }
            _ => {}
        }
        i += 1;
    }
    let _ = follow; // follow is accepted but not implemented in the port
    let path = dirs::state_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("/tmp"))
        .join("termos")
        .join("daemon.log");
    if path.exists() {
        let content = std::fs::read_to_string(&path)?;
        if all || lines == 0 {
            print!("{content}");
        } else {
            let collected: Vec<&str> = content.lines().collect();
            let start = collected.len().saturating_sub(lines);
            for line in &collected[start..] {
                println!("{line}");
            }
        }
        if clear {
            std::fs::write(&path, "")?;
        }
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

// ─── New CLI commands ────────────────────────────────────────────────────

/// Resolve a session name: explicit, else the only one, else error.
fn resolve_session_name(client: &DaemonClient, session: &Option<String>) -> Result<String, Box<dyn std::error::Error>> {
    match session {
        Some(s) => Ok(s.clone()),
        None => {
            let sessions = client.list()?;
            match sessions.len() {
                1 => Ok(sessions[0].name.clone()),
                0 => Err("no sessions; create one with `tuios run`".into()),
                _ => Err("multiple sessions; pass -s <session>".into()),
            }
        }
    }
}

/// `tuios ssh` — SSH server mode (requires the `network` feature).
#[cfg(feature = "network")]
fn cmd_ssh(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let opts = termos::cli::parse_ssh_args(args)
        .map_err(|e| -> Box<dyn std::error::Error> { e.into() })?;
    let _ = &opts.default_session;
    let _ = opts.ephemeral;

    let config = UserConfig::load();
    let server = termos::network::ssh::TermosSshServer::new(config);
    let addr = format!("{}:{}", opts.host, opts.port);
    let cfg = termos::network::ssh::SshServerConfig {
        addr,
        host_key_path: opts.key_path,
    };
    // The SSH server runs a tokio runtime internally.
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(server.run(cfg))?;
    Ok(())
}

#[cfg(not(feature = "network"))]
fn cmd_ssh(_args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    Err("ssh server requires building with --features network".into())
}

/// `tuios new-window [name]` — create a new window in a session.
fn cmd_new_window(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let opts = termos::cli::parse_new_window_args(args);
    let name = opts.name.unwrap_or_default();

    let client = DaemonClient::connect()?;
    let session = resolve_session_name(&client, &opts.session)?;

    client.send(&Message::NewWindowInSession {
        session: Some(session.clone()),
        name: name.clone(),
    })?;
    match recv_reply(&client)? {
        Message::NewWindowResult { window } => {
            if opts.json {
                let val = serde_json::json!({
                    "window_id": window.id,
                    "title": window.title,
                    "workspace": window.workspace,
                });
                println!("{}", serde_json::to_string_pretty(&val)?);
            } else {
                let display = if name.is_empty() { &window.title } else { &name };
                println!("{}  {}", window.id, display);
            }
            Ok(())
        }
        Message::Error { message } => Err(message.into()),
        _ => Err("unexpected reply".into()),
    }
}

/// `tuios run-command <cmd> [args]` — execute a tape command remotely.
fn cmd_run_command(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let opts = termos::cli::parse_run_command_args(args);

    if opts.list {
        // List available tape commands.
        for c in termos::cli::AVAILABLE_RUN_COMMANDS {
            println!("{c}");
        }
        return Ok(());
    }

    let command = opts
        .command
        .ok_or("usage: tuios run-command <command> [args...] (use --list for available commands)")?;

    let client = DaemonClient::connect()?;
    let session = resolve_session_name(&client, &opts.session)?;

    client.send(&Message::RunCommand {
        session: Some(session.clone()),
        command: command.clone(),
        args: opts.args.clone(),
    })?;
    match recv_reply(&client)? {
        Message::RunCommandResult { result } => {
            if opts.json {
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                println!("{}", serde_json::to_string(&result)?);
            }
            Ok(())
        }
        Message::Error { message } => Err(message.into()),
        _ => Err("unexpected reply".into()),
    }
}

/// `tuios set-config <path> <value>` — set a runtime config option.
fn cmd_set_config(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let opts = termos::cli::parse_set_config_args(args)
        .map_err(|e| -> Box<dyn std::error::Error> { e.into() })?;
    let path = opts.path.unwrap();
    let value = opts.value.unwrap();

    let client = DaemonClient::connect()?;
    let session = resolve_session_name(&client, &opts.session)?;

    client.send(&Message::SetConfig {
        session: Some(session),
        path: path.clone(),
        value: value.clone(),
    })?;
    match recv_reply(&client)? {
        Message::Error { message } => Err(message.into()),
        _ => {
            println!("Set {path} = {value}");
            Ok(())
        }
    }
}

/// `tuios get-config <path>` — read a runtime config option.
fn cmd_get_config(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let opts = termos::cli::parse_get_config_args(args)
        .map_err(|e| -> Box<dyn std::error::Error> { e.into() })?;
    let path = opts.path.unwrap();

    let client = DaemonClient::connect()?;
    let session = resolve_session_name(&client, &opts.session)?;

    client.send(&Message::GetConfig {
        session: Some(session),
        path: path.clone(),
    })?;
    match recv_reply(&client)? {
        Message::ConfigValue { value, .. } => {
            println!("{value}");
            Ok(())
        }
        Message::Error { message } => Err(message.into()),
        _ => Err("unexpected reply".into()),
    }
}

/// `tuios explain-agent-screen` — explain screen rule matching.
fn cmd_explain_agent_screen(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let opts = termos::cli::parse_explain_agent_screen_args(args);

    let client = DaemonClient::connect()?;
    let session = resolve_session_name(&client, &opts.session)?;

    client.send(&Message::ExplainAgentScreen {
        session: Some(session),
        window: opts.window,
        harness: opts.harness,
        lines: opts.lines,
    })?;
    match recv_reply(&client)? {
        Message::ExplainResult { explanation } => {
            println!("{}", serde_json::to_string_pretty(&explanation)?);
            Ok(())
        }
        Message::Error { message } => Err(message.into()),
        _ => Err("unexpected reply".into()),
    }
}

/// `tuios set-workspace-name <workspace> [name]` — name a workspace.
fn cmd_set_workspace_name(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let opts = termos::cli::parse_set_workspace_name_args(args)
        .map_err(|e| -> Box<dyn std::error::Error> { e.into() })?;
    let workspace = opts.workspace.unwrap();
    let name = opts.name;

    let client = DaemonClient::connect()?;
    let session = resolve_session_name(&client, &opts.session)?;

    client.send(&Message::SetWorkspaceName {
        session: Some(session),
        workspace,
        name: name.clone(),
    })?;
    match recv_reply(&client)? {
        Message::Error { message } => Err(message.into()),
        _ => {
            if name.is_empty() {
                println!("Cleared the name of workspace {workspace}.");
            } else {
                println!("Workspace {workspace} is now named {name:?}.");
            }
            Ok(())
        }
    }
}

/// `tuios get-window [id-or-name]` — get detailed window info.
fn cmd_get_window(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let opts = termos::cli::parse_get_window_args(args);

    let client = DaemonClient::connect()?;
    let session = resolve_session_name(&client, &opts.session)?;

    client.send(&Message::GetWindow {
        session: Some(session.clone()),
        window: opts.window,
    })?;
    match recv_reply(&client)? {
        Message::WindowDetail { detail } => {
            if opts.json {
                println!("{}", serde_json::to_string_pretty(&detail)?);
            } else {
                // Print as labelled lines.
                if let Some(obj) = detail.as_object() {
                    for (key, val) in obj {
                        println!("{:<14} {}", key, val);
                    }
                } else {
                    println!("{}", serde_json::to_string_pretty(&detail)?);
                }
            }
            Ok(())
        }
        Message::Error { message } => Err(message.into()),
        _ => Err("unexpected reply".into()),
    }
}

/// `tuios list-verbs [verb]` — list control protocol verbs.
fn cmd_list_verbs(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let mut json = false;
    let mut verb_filter: Option<String> = None;
    for a in args {
        if a == "--json" {
            json = true;
        } else if !a.starts_with('-') {
            verb_filter = Some(a.clone());
        }
    }
    let registry = termos::session::VerbRegistry::new();
    let req = termos::session::VerbRequest {
        id: None,
        verb: "list-verbs".to_string(),
        params: Some(verb_filter.as_ref().map(|v| serde_json::json!({"verb": v})).unwrap_or(serde_json::json!({}))),
    };
    let resp = registry.dispatch(&req);
    if json {
        if let Some(result) = resp.result {
            println!("{}", serde_json::to_string_pretty(&result)?);
        } else if let Some(err) = resp.error {
            return Err(err.message.into());
        }
    } else {
        // Print the verb list in human-readable form.
        for v in VERBS {
            println!("{v}");
        }
        if let Some(result) = resp.result {
            if let Some(verbs) = result.get("verbs").and_then(|v| v.as_array()) {
                println!();
                for v in verbs {
                    let name = v.get("verb").and_then(|v| v.as_str()).unwrap_or("?");
                    let desc = v.get("description").and_then(|v| v.as_str()).unwrap_or("");
                    println!("{name}");
                    println!("  {desc}");
                }
            }
        }
    }
    Ok(())
}

/// `tuios keybinds list-custom` — list only customized keybindings.
fn cmd_keybinds_list_custom() -> Result<(), Box<dyn std::error::Error>> {
    let user_cfg = UserConfig::load();
    let default_cfg = UserConfig::default_config();

    #[allow(clippy::type_complexity)]
    let sections: &[(&str, &std::collections::HashMap<String, Vec<String>>, &std::collections::HashMap<String, Vec<String>>)] = &[
        ("window_management", &user_cfg.keybindings.window_management, &default_cfg.keybindings.window_management),
        ("workspaces", &user_cfg.keybindings.workspaces, &default_cfg.keybindings.workspaces),
        ("layout", &user_cfg.keybindings.layout, &default_cfg.keybindings.layout),
        ("mode_control", &user_cfg.keybindings.mode_control, &default_cfg.keybindings.mode_control),
        ("system", &user_cfg.keybindings.system, &default_cfg.keybindings.system),
        ("prefix_mode", &user_cfg.keybindings.prefix_mode, &default_cfg.keybindings.prefix_mode),
        ("window_prefix", &user_cfg.keybindings.window_prefix, &default_cfg.keybindings.window_prefix),
        ("minimize_prefix", &user_cfg.keybindings.minimize_prefix, &default_cfg.keybindings.minimize_prefix),
        ("workspace_prefix", &user_cfg.keybindings.workspace_prefix, &default_cfg.keybindings.workspace_prefix),
    ];

    let mut customizations: Vec<(String, String, String)> = Vec::new();
    for (_section, user, default) in sections {
        for (action, default_keys) in default.iter() {
            if let Some(user_keys) = user.get(action) {
                if user_keys != default_keys {
                    customizations.push((
                        action.replace('_', " "),
                        default_keys.join(", "),
                        user_keys.join(", "),
                    ));
                }
            }
        }
    }

    if customizations.is_empty() {
        println!("No custom keybindings configured. All keybindings are using defaults.");
        println!();
        println!("Run 'tuios keybinds list' to see all keybindings.");
        return Ok(());
    }

    println!();
    println!("Custom Keybindings");
    println!();
    println!("{:<30} {:<20} Custom", "Action", "Default");
    println!("{}", "-".repeat(70));
    for (action, default, custom) in &customizations {
        println!("{:<30} {:<20} {}", action, default, custom);
    }
    println!();
    println!("Found {} customized keybinding(s)", customizations.len());
    Ok(())
}

/// `tuios completion <shell>` — generate shell completion scripts.
fn cmd_completion(shell: &str) -> Result<(), Box<dyn std::error::Error>> {
    let shell = termos::cli::CompletionShell::parse(shell)
        .ok_or_else(|| format!("unsupported shell '{shell}' (try: bash, zsh, fish)"))?;
    let completions = match shell {
        termos::cli::CompletionShell::Bash => generate_bash_completions(),
        termos::cli::CompletionShell::Zsh => generate_zsh_completions(),
        termos::cli::CompletionShell::Fish => generate_fish_completions(),
    };
    print!("{completions}");
    Ok(())
}

fn generate_bash_completions() -> String {
    let mut out = String::new();
    out.push_str("# Bash completion for termos\n");
    out.push_str("_termos() {\n");
    out.push_str("    local cur prev cmds\n");
    out.push_str("    cur=${COMP_WORDS[COMP_CWORD]}\n");
    out.push_str("    cmds=\"");
    out.push_str(&termos::cli::COMPLETION_COMMANDS.join(" "));
    out.push_str("\"\n");
    out.push_str("    if [ $COMP_CWORD -eq 1 ]; then\n");
    out.push_str("        COMPREPLY=( $(compgen -W \"$cmds\" -- \"$cur\") )\n");
    out.push_str("        return 0\n");
    out.push_str("    fi\n");
    out.push_str("}\n");
    out.push_str("complete -F _termos termos\n");
    out
}

fn generate_zsh_completions() -> String {
    let mut out = String::new();
    out.push_str("#compdef termos\n");
    out.push_str("_termos() {\n");
    out.push_str("    local -a commands\n");
    out.push_str("    commands=(\n");
    for c in termos::cli::COMPLETION_COMMANDS {
        out.push_str(&format!("        '{c}'\n"));
    }
    out.push_str("    )\n");
    out.push_str("    _arguments '1: :->command'\n");
    out.push_str("    case $state in\n");
    out.push_str("        command) _describe 'command' commands ;;\n");
    out.push_str("    esac\n");
    out.push_str("}\n");
    out.push_str("compdef _termos termos\n");
    out
}

fn generate_fish_completions() -> String {
    let mut out = String::new();
    out.push_str("# Fish completion for termos\n");
    for c in termos::cli::COMPLETION_COMMANDS {
        out.push_str(&format!("complete -c termos -n '__fish_use_subcommand' -a {c}\n"));
    }
    out
}

/// Preview a theme's ANSI colors (mirrors Go's `previewThemeColors`).
fn preview_theme_colors(name: &str) -> Result<(), Box<dyn std::error::Error>> {
    let themes = termos::config::theme::list_theme_names();
    if !themes.iter().any(|t| t == name) {
        eprintln!("theme '{name}' not found");
        eprintln!("available themes:");
        for t in themes {
            eprintln!("  {t}");
        }
        return Err(format!("theme '{name}' not found").into());
    }
    // Print the 16 ANSI colors as colored blocks.
    let labels = [
        "00 Black", "01 Red", "02 Green", "03 Yellow",
        "04 Blue", "05 Magenta", "06 Cyan", "07 White",
        "08 Bright Black", "09 Bright Red", "10 Bright Green", "11 Bright Yellow",
        "12 Bright Blue", "13 Bright Magenta", "14 Bright Cyan", "15 Bright White",
    ];
    println!("Theme: {name}");
    println!();
    for (i, label) in labels.iter().enumerate() {
        let bg = if i < 8 { i + 40 } else { i + 92 };
        println!("\x1b[{bg}m  {label:<20}  \x1b[0m");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn completion_scripts_include_control_surface_commands() {
        for script in [
            super::generate_bash_completions(),
            super::generate_zsh_completions(),
            super::generate_fish_completions(),
        ] {
            for cmd in ["action", "subscribe", "block-until-exit", "exec"] {
                assert!(
                    script.contains(cmd),
                    "completion script missing '{cmd}': {script}"
                );
            }
        }
    }

    #[test]
    fn exec_rejects_unknown_flags() {
        // `-s` takes a value; a flag-like first arg is an error, not a command.
        let err = super::cmd_exec(&["-x".to_string(), "echo hi".to_string()])
            .err()
            .map(|e| e.to_string())
            .unwrap_or_default();
        assert!(
            err.contains("unknown flag"),
            "expected an unknown-flag error, got: {err}"
        );
    }

    #[test]
    fn exec_requires_a_command() {
        let err = super::cmd_exec(&[]).err().map(|e| e.to_string()).unwrap_or_default();
        assert!(
            err.contains("usage: tuios exec"),
            "expected a usage error, got: {err}"
        );
    }

    #[test]
    fn exec_flags_default_shell_and_cwd() {
        let f = super::parse_exec_flags(&["echo hi".to_string()]).unwrap();
        assert!(f.shell.is_none());
        assert!(f.cwd.is_none());
        assert_eq!(f.cmd, vec!["echo hi"]);
        assert_eq!(f.timeout_ms, 30_000);
        assert!(!f.json && !f.keep);
    }

    #[test]
    fn exec_flags_parse_shell_and_cwd() {
        let args = vec![
            "--shell".to_string(),
            "/bin/bash".to_string(),
            "--cwd".to_string(),
            "/tmp".to_string(),
            "-s".to_string(),
            "build".to_string(),
            "make".to_string(),
        ];
        let f = super::parse_exec_flags(&args).unwrap();
        assert_eq!(f.shell.as_deref(), Some("/bin/bash"));
        assert_eq!(f.cwd.as_deref(), Some("/tmp"));
        assert_eq!(f.session.as_deref(), Some("build"));
        assert_eq!(f.cmd, vec!["make"]);
    }

    #[test]
    fn exec_flags_unknown_flag_rejected() {
        let err = super::parse_exec_flags(&["--bogus".to_string()]).unwrap_err();
        assert!(err.contains("unknown flag '--bogus'"), "got: {err}");
    }

    #[test]
    fn parse_param_value_keeps_plain_strings() {
        assert_eq!(super::parse_param_value("exit 0"), serde_json::json!("exit 0"));
        assert_eq!(super::parse_param_value("test.*"), serde_json::json!("test.*"));
        assert_eq!(super::parse_param_value("w0"), serde_json::json!("w0"));
    }

    #[test]
    fn parse_param_value_types_json_shapes() {
        assert_eq!(super::parse_param_value("5000"), serde_json::json!(5000));
        assert_eq!(super::parse_param_value("0"), serde_json::json!(0));
        assert_eq!(super::parse_param_value("1.5"), serde_json::json!(1.5));
        assert_eq!(super::parse_param_value("true"), serde_json::json!(true));
        assert_eq!(super::parse_param_value("null"), serde_json::json!(null));
        assert_eq!(
            super::parse_param_value("[\"a\", \"b\"]"),
            serde_json::json!(["a", "b"])
        );
        assert_eq!(
            super::parse_param_value("\"hello world\""),
            serde_json::json!("hello world")
        );
        // A malformed JSON-looking value degrades to a plain string.
        assert_eq!(super::parse_param_value("[unclosed"), serde_json::json!("[unclosed"));
    }

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
