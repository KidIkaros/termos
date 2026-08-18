//! Web server network mode — serves TermOS over HTTP + WebSocket.
//!
//! The Go reference uses a separate `tuios-web` binary with xterm.js for
//! security isolation. The Rust port uses `axum` for HTTP and
//! `tokio-tungstenite` for WebSocket, serving a static HTML page with
//! xterm.js that connects back over WebSocket. Terminal I/O is carried as
//! JSON frames: `{ "type": "input", "data": "..." }` and
//! `{ "type": "output", "data": "..." }`.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::response::Html;
use axum::routing::get;
use axum::Router;
use futures_util::{SinkExt, StreamExt};
use ratatui::backend::{Backend, CrosstermBackend};
use ratatui::Terminal;
use tokio::sync::Mutex;

use crate::app::render::render;
use crate::app::Os;
use crate::config::UserConfig;

/// The static HTML page with xterm.js that connects to the WebSocket.
const INDEX_HTML: &str = include_str!("web/index.html");

/// Web server state shared across connections.
#[derive(Clone)]
pub struct WebServerState {
    pub config: Arc<UserConfig>,
}

/// Run the web server on the given address.
pub async fn run_web_server(
    addr: &str,
    config: UserConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    let addr: SocketAddr = addr.parse()?;
    let state = WebServerState {
        config: Arc::new(config),
    };

    let app = Router::new()
        .route("/", get(index))
        .route("/ws", get(ws_handler))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    log::info!("web server listening on {addr}");
    axum::serve(listener, app).await?;
    Ok(())
}

async fn index() -> Html<&'static str> {
    Html(INDEX_HTML)
}

async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<WebServerState>,
) -> axum::response::Response {
    ws.on_upgrade(move |socket| handle_ws(socket, state))
}

/// Handle a WebSocket connection: spawn a TermOS session and bridge I/O.
///
/// The render loop draws Os state to a `CrosstermBackend<Vec<u8>>` and sends
/// the ANSI escape sequences as JSON frames. Input from the WebSocket is
/// parsed into crossterm `KeyEvent`s and forwarded to the Os.
async fn handle_ws(socket: WebSocket, state: WebServerState) {
    let mut os = Os::new((*state.config).clone());
    os.init_graphics();

    // Determine terminal size from env or default to 80x24.
    if let Ok((cols, rows)) = crossterm::terminal::size() {
        os.width = cols.max(1) as i32;
        os.height = rows.max(1) as i32;
    } else {
        os.width = 80;
        os.height = 24;
    }

    // Spawn a shell.
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
    let wake = Box::new(|| {}) as Box<dyn Fn() + Send + 'static>;
    if let Err(e) = os.spawn_window(&shell, wake) {
        log::warn!("web: failed to spawn shell: {e}");
    }

    let os = Arc::new(Mutex::new(os));
    let (mut ws_sink, mut ws_source) = socket.split();

    // Render loop: draws Os state → ANSI → WebSocket JSON frames.
    let os_render = os.clone();
    let render_handle = tokio::spawn(async move {
        let frame_budget = Duration::from_millis(16); // ~60 FPS
        let mut last_render = Instant::now();
        let mut buf: Vec<u8> = Vec::new();

        loop {
            // Rate-limit rendering.
            if last_render.elapsed() < frame_budget {
                tokio::time::sleep(Duration::from_millis(2)).await;
                continue;
            }

            // Render the Os state.
            buf.clear();
            {
                let mut os = os_render.lock().await;

                // Tick the Os state machine.
                os.tick_agent_alerts();
                os.tick_script();
                os.sync_window_sizes();
                os.flush_graphics();

                // Create a terminal backend writing to our buffer.
                let backend = CrosstermBackend::new(&mut buf);
                if let Ok(mut terminal) = Terminal::new(backend) {
                    let _ = terminal.draw(|frame| {
                        render(&os, frame.buffer_mut());
                    });
                    let _ = terminal.backend_mut().flush();
                }

                // Send host sequences (OSC 9, BEL, etc.) if any.
                let host_seq = os.take_host_sequence();
                if !host_seq.is_empty() {
                    buf.extend_from_slice(&host_seq);
                }
            }

            // Send the rendered ANSI output as a JSON frame.
            if !buf.is_empty() {
                let ansi = String::from_utf8_lossy(&buf).to_string();
                let msg = serde_json::json!({
                    "type": "output",
                    "data": ansi,
                });
                if ws_sink.send(Message::Text(msg.to_string())).await.is_err() {
                    break; // Client disconnected
                }
            }

            last_render = Instant::now();
        }
    });

    // Input loop: receives JSON frames from WebSocket, parses into key events,
    // and forwards to Os.
    let os_input = os.clone();
    let input_handle = tokio::spawn(async move {
        while let Some(msg) = ws_source.next().await {
            match msg {
                Ok(Message::Text(text)) => {
                    if let Ok(frame) = serde_json::from_str::<serde_json::Value>(&text) {
                        if let Some(msg_type) = frame.get("type").and_then(|v| v.as_str()) {
                            match msg_type {
                                "input" => {
                                    if let Some(data) = frame.get("data").and_then(|v| v.as_str()) {
                                        let mut os = os_input.lock().await;
                                        // Parse the input data as crossterm key events.
                                        // The xterm.js frontend sends key data as raw
                                        // characters or escape sequences.
                                        let events = parse_web_input(data);
                                        for key_event in &events {
                                            let effects =
                                                os.update(crate::app::msg::Msg::Key(*key_event));
                                            if effects.iter().any(|e| {
                                                matches!(e, crate::app::effect::Effect::Quit)
                                            }) {
                                                return;
                                            }
                                        }
                                    }
                                }
                                "resize" => {
                                    if let (Some(cols), Some(rows)) = (
                                        frame.get("cols").and_then(|v| v.as_u64()),
                                        frame.get("rows").and_then(|v| v.as_u64()),
                                    ) {
                                        let mut os = os_input.lock().await;
                                        os.update(crate::app::msg::Msg::Resize {
                                            cols: cols.min(u16::MAX as u64) as u16,
                                            rows: rows.min(u16::MAX as u64) as u16,
                                        });
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                }
                Ok(Message::Binary(_)) => {}
                Ok(Message::Close(_)) | Err(_) => break,
                _ => {}
            }
        }
    });

    // Wait for either the render or input loop to finish.
    tokio::select! {
        _ = render_handle => {},
        _ = input_handle => {},
    }
}

/// Parse WebSocket input data into crossterm `KeyEvent`s.
///
/// The xterm.js frontend sends input as:
/// - Plain characters for printable keys
/// - Escape sequences for special keys (arrows, function keys, etc.)
/// - Modified keys with escape sequence prefixes
///
/// This parser handles the most common xterm.js input sequences.
fn parse_web_input(data: &str) -> Vec<crossterm::event::KeyEvent> {
    let bytes = data.as_bytes();
    let mut events = Vec::new();
    let mut i = 0;

    while i < bytes.len() {
        let b = bytes[i];

        match b {
            // ESC sequences from xterm.js
            0x1b => {
                if i + 1 < bytes.len() {
                    match bytes[i + 1] {
                        // CSI sequences: ESC [
                        b'[' => {
                            if i + 2 < bytes.len() {
                                match bytes[i + 2] {
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
                                        events.push(crossterm::event::KeyEvent {
                                            code: crossterm::event::KeyCode::BackTab,
                                            modifiers: crossterm::event::KeyModifiers::SHIFT,
                                            kind: crossterm::event::KeyEventKind::Press,
                                            state: crossterm::event::KeyEventState::NONE,
                                        });
                                        i += 3;
                                    }
                                    b'1'..=b'9' => {
                                        let mut num = (bytes[i + 2] - b'0') as u32;
                                        let mut j = i + 3;
                                        while j < bytes.len() && bytes[j].is_ascii_digit() {
                                            num = num * 10 + (bytes[j] - b'0') as u32;
                                            j += 1;
                                        }
                                        if j < bytes.len() && bytes[j] == b'~' {
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
                                            i += 2;
                                        }
                                    }
                                    _ => {
                                        i += 2;
                                    }
                                }
                            } else {
                                i += 1;
                            }
                        }
                        // Alt+letter
                        _ if bytes[i + 1].is_ascii_alphabetic() => {
                            events.push(crossterm::event::KeyEvent {
                                code: crossterm::event::KeyCode::Char(bytes[i + 1] as char),
                                modifiers: crossterm::event::KeyModifiers::ALT,
                                kind: crossterm::event::KeyEventKind::Press,
                                state: crossterm::event::KeyEventState::NONE,
                            });
                            i += 2;
                        }
                        _ => {
                            i += 1;
                        }
                    }
                } else {
                    i += 1;
                }
            }
            // Backspace (0x08 and 0x7f)
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
            // Enter
            b'\n' | b'\r' => {
                events.push(crossterm::event::KeyEvent {
                    code: crossterm::event::KeyCode::Enter,
                    modifiers: crossterm::event::KeyModifiers::NONE,
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
            // Ctrl+letter (catch remaining 0x01..=0x1a not handled above)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_web_ctrl_b() {
        let events = parse_web_input("\x02");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].code, crossterm::event::KeyCode::Char('b'));
        assert!(events[0]
            .modifiers
            .contains(crossterm::event::KeyModifiers::CONTROL));
    }

    #[test]
    fn parse_web_arrow_keys() {
        let events = parse_web_input("\x1b[A\x1b[B\x1b[C\x1b[D");
        assert_eq!(events.len(), 4);
        assert_eq!(events[0].code, crossterm::event::KeyCode::Up);
        assert_eq!(events[1].code, crossterm::event::KeyCode::Down);
        assert_eq!(events[2].code, crossterm::event::KeyCode::Right);
        assert_eq!(events[3].code, crossterm::event::KeyCode::Left);
    }

    #[test]
    fn parse_web_printable() {
        let events = parse_web_input("hello");
        assert_eq!(events.len(), 5);
        assert_eq!(events[0].code, crossterm::event::KeyCode::Char('h'));
    }

    #[test]
    fn parse_web_enter() {
        let events = parse_web_input("\n");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].code, crossterm::event::KeyCode::Enter);
    }

    #[test]
    fn parse_web_backspace() {
        let events = parse_web_input("\x7f");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].code, crossterm::event::KeyCode::Backspace);
    }

    #[test]
    fn parse_web_function_key() {
        // F5 = ESC [ 1 5 ~
        let events = parse_web_input("\x1b[15~");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].code, crossterm::event::KeyCode::F(5));
    }

    #[test]
    fn parse_web_home_end() {
        let events = parse_web_input("\x1b[H\x1b[F");
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].code, crossterm::event::KeyCode::Home);
        assert_eq!(events[1].code, crossterm::event::KeyCode::End);
    }
}
