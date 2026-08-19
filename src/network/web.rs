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
use axum::response::{Html, IntoResponse, Response};
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

/// A minimal token-gate page shown when the server requires a token and the
/// request does not carry one. The form GETs back to `/` with `?token=...`,
/// which the index handler validates before serving the terminal page.
const LOGIN_HTML: &str = "<!DOCTYPE html>\
<html><head><meta charset=\"utf-8\"><title>TermOS — access token</title></head>\
<body style=\"font-family:sans-serif;background:#111;color:#eee;display:flex;align-items:center;\
justify-content:center;height:100vh;margin:0\">\
<form method=\"get\" action=\"/\">\
<h2 style=\"margin-top:0\">TermOS</h2>\
<p style=\"color:#aaa\">This server requires an access token.</p>\
<input type=\"password\" name=\"token\" placeholder=\"access token\" autofocus\
style=\"padding:8px;width:16em;font-size:14px\">\
<button type=\"submit\" style=\"padding:8px 14px;margin-left:6px\">Connect</button>\
</form></body></html>";

/// Web server state shared across connections.
#[derive(Clone)]
/// Releases a [`crate::web::ConnectionLimiter`] slot on drop.
struct ConnectionGuard(Arc<crate::web::ConnectionLimiter>);

impl Drop for ConnectionGuard {
    fn drop(&mut self) {
        self.0.release();
    }
}

#[derive(Clone)]
pub struct WebServerState {
    pub config: Arc<UserConfig>,
    /// Enforces the max-connections limit across all live sockets.
    pub limiter: Arc<crate::web::ConnectionLimiter>,
    /// The operator's touch-mode preference (auto/on/off).
    pub touch_mode: crate::web::TouchMode,
    /// Read-only mode: guest input is dropped.
    pub read_only: bool,
    /// Access token, when auth is on. Clients must present it as `?token=`.
    pub token: Option<String>,
    /// Whether clients must authenticate (computed from the bind host and
    /// the configured token; loopback binds without a token stay open).
    pub auth_required: bool,
}

/// Run the web server on the given address.
///
/// `touch_mode` decides how a client's touch screen is detected, `limiter`
/// caps concurrent connections (0 = unlimited), and `read_only` stops guest
/// input from reaching the shells.
/// Web server options.
#[derive(Debug, Clone, Default)]
pub struct WebServerOptions {
    /// Bind address, e.g. "127.0.0.1:8080".
    pub addr: String,
    /// Base user config cloned for each guest session.
    pub config: UserConfig,
    /// Touch detection preference for the mobile key bar.
    pub touch_mode: crate::web::TouchMode,
    /// Max concurrent WebSocket connections (0 = unlimited).
    pub max_connections: u64,
    /// Read-only observer mode: guest input is dropped.
    pub read_only: bool,
    /// Whether to serve over TLS (auto-TLS or explicit cert/key).
    pub tls_enabled: bool,
    /// Access token; required from non-loopback binds and always when set.
    pub token: Option<String>,
    /// Explicit TLS certificate PEM path.
    pub cert: Option<String>,
    /// Explicit TLS private key PEM path.
    pub key: Option<String>,
}

/// Run the web server on the given address.
///
/// `touch_mode` decides how a client's touch screen is detected, `limiter`
/// caps concurrent connections (0 = unlimited), and `read_only` stops guest
/// input from reaching the shells. `token` enables auth (and is mandatory
/// for non-loopback binds); `cert`/`key` are explicit PEM files, while
/// `auto_tls` generates a self-signed certificate for the bind host.
pub async fn run_web_server(
    opts: WebServerOptions,
) -> Result<(), Box<dyn std::error::Error>> {
    let WebServerOptions {
        addr,
        config,
        touch_mode,
        max_connections,
        read_only,
        tls_enabled,
        token,
        cert,
        key,
    } = opts;
    // Refuse non-TLS on non-loopback addresses to prevent credential exposure.
    let host = addr.split(':').next().unwrap_or("127.0.0.1");
    crate::web::check_transport_security(host, tls_enabled)?;

    let addr: SocketAddr = addr.parse()?;
    let auth_required = crate::web::requires_token(host, token.as_deref());
    let state = WebServerState {
        config: Arc::new(config),
        limiter: Arc::new(crate::web::ConnectionLimiter::new(max_connections)),
        touch_mode,
        read_only,
        token,
        auth_required,
    };

    let app = Router::new()
        .route("/", get(index))
        .route("/ws", get(ws_handler))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    log::info!("web server listening on {addr}{}", if tls_enabled { " (TLS)" } else { "" });
    if tls_enabled {
        #[cfg(feature = "tls")]
        {
            return serve_tls(listener, app, host, cert, key).await;
        }
        #[cfg(not(feature = "tls"))]
        {
            return Err("TLS requested but this build lacks the `tls` feature".into());
        }
    }
    axum::serve(listener, app).await?;
    Ok(())
}

/// Serve the state-injected axum app over TLS, either from explicit cert/key
/// PEM files or from a self-signed certificate generated for the bind host
/// (auto-TLS).
///
/// axum's `Router` is a tower `Service`, but hyper's connection builder wants
/// a hyper `Service`, so each TLS stream is bridged with `service_fn` that
/// forwards requests through `Router::into_service` via `oneshot`.
#[cfg(feature = "tls")]
async fn serve_tls(
    listener: tokio::net::TcpListener,
    app: Router<()>,
    host: &str,
    cert: Option<String>,
    key: Option<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    use hyper::body::Incoming;
    use hyper::service::service_fn;
    use hyper_util::rt::{TokioExecutor, TokioIo};
    use hyper_util::server::conn::auto::Builder as HttpBuilder;
    use std::convert::Infallible;
    use tower::ServiceExt;

    let tls_config = match (cert, key) {
        (Some(cert), Some(key)) => crate::network::tls::load_tls_config(&cert, &key)?,
        _ => {
            let dir = dirs::data_dir()
                .unwrap_or_else(std::env::temp_dir)
                .join("termos")
                .join("tls");
            crate::network::tls::auto_tls_config(&dir, &[host.to_string()], 365)?
        }
    };
    let acceptor = tokio_rustls::TlsAcceptor::from(tls_config);
    let builder = HttpBuilder::new(TokioExecutor::new());

    // The router as a tower service with a fixed body type (hyper's Incoming).
    let tower_service = app.into_service::<Incoming>();

    loop {
        let (stream, peer) = match listener.accept().await {
            Ok(s) => s,
            Err(e) => {
                log::warn!("web: accept error: {e}");
                continue;
            }
        };
        let acceptor = acceptor.clone();
        let tower_service = tower_service.clone();
        let builder = builder.clone();
        tokio::spawn(async move {
            match acceptor.accept(stream).await {
                Ok(tls_stream) => {
                    let io = TokioIo::new(tls_stream);
                    let tower_service = tower_service.clone();
                    let hyper_service = service_fn(move |request: axum::http::Request<Incoming>| {
                        let tower_service = tower_service.clone();
                        async move {
                            let response = tower_service.oneshot(request).await.unwrap();
                            Ok::<_, Infallible>(response)
                        }
                    });
                    if let Err(e) =
                        builder.serve_connection_with_upgrades(io, hyper_service).await
                    {
                        log::debug!("web: connection from {peer} closed: {e}");
                    }
                }
                Err(e) => log::warn!("web: TLS handshake with {peer} failed: {e}"),
            }
        });
    }
}

/// Pull the `?token=` value out of a request's query string.
fn query_token(uri: &axum::http::Uri) -> Option<String> {
    uri.query().and_then(|q| {
        q.split('&')
            .find_map(|pair| pair.strip_prefix("token="))
            .map(|v| v.to_string())
    })
}

/// Whether a request passes the auth gate: open servers always pass, and
/// gated servers require a valid `?token=`.
pub(crate) fn auth_passes(state: &WebServerState, uri: &axum::http::Uri) -> bool {
    !state.auth_required
        || crate::web::token_is_valid(state.token.as_deref(), query_token(uri).as_deref())
}

async fn index(State(state): State<WebServerState>, uri: axum::http::Uri) -> Response {
    if !auth_passes(&state, &uri) {
        return (axum::http::StatusCode::UNAUTHORIZED, Html(LOGIN_HTML)).into_response();
    }
    Html(INDEX_HTML).into_response()
}

async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<WebServerState>,
    headers: axum::http::HeaderMap,
    uri: axum::http::Uri,
) -> Response {
    if !auth_passes(&state, &uri) {
        return axum::http::StatusCode::UNAUTHORIZED.into_response();
    }
    let touch = crate::web::resolve_touch(
        state.touch_mode,
        headers
            .get("sec-ch-ua-mobile")
            .and_then(|v| v.to_str().ok()),
        headers
            .get(axum::http::header::USER_AGENT)
            .and_then(|v| v.to_str().ok()),
    );
    ws.on_upgrade(move |socket| handle_ws(socket, state, touch))
}

/// Handle a WebSocket connection: spawn a TermOS session and bridge I/O.
///
/// The render loop draws Os state to a `CrosstermBackend<Vec<u8>>` and sends
/// the ANSI escape sequences as JSON frames. Input from the WebSocket is
/// parsed into crossterm `KeyEvent`s and forwarded to the Os.
async fn handle_ws(socket: WebSocket, state: WebServerState, touch: bool) {
    // Enforce the connection limit: a rejected socket is simply dropped.
    if !state.limiter.acquire() {
        log::warn!("web: connection limit reached, rejecting");
        return;
    }
    let _release = ConnectionGuard(state.limiter.clone());

    let mut os = Os::new((*state.config).clone());
    os.init_graphics();
    os.touch_client = touch;
    os.read_only = state.read_only;

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

    fn state_with(token: Option<&str>, auth_required: bool) -> WebServerState {
        WebServerState {
            config: Arc::new(UserConfig::default()),
            limiter: Arc::new(crate::web::ConnectionLimiter::new(0)),
            touch_mode: crate::web::TouchMode::Auto,
            read_only: false,
            token: token.map(|s| s.to_string()),
            auth_required,
        }
    }

    #[test]
    fn query_token_extracts_from_query() {
        let uri: axum::http::Uri = "/?token=abc123&x=1".parse().unwrap();
        assert_eq!(query_token(&uri).as_deref(), Some("abc123"));
    }

    #[test]
    fn query_token_missing() {
        let uri: axum::http::Uri = "/".parse().unwrap();
        assert_eq!(query_token(&uri), None);
    }

    #[test]
    fn auth_passes_open_server() {
        let s = state_with(None, false);
        assert!(auth_passes(&s, &"/".parse().unwrap()));
    }

    #[test]
    fn auth_passes_token_required_rejects_missing() {
        let s = state_with(Some("secret"), true);
        assert!(!auth_passes(&s, &"/".parse().unwrap()));
    }

    #[test]
    fn auth_passes_token_required_accepts_valid() {
        let s = state_with(Some("secret"), true);
        assert!(auth_passes(&s, &"/?token=secret".parse().unwrap()));
    }

    #[test]
    fn auth_passes_token_required_rejects_wrong() {
        let s = state_with(Some("secret"), true);
        assert!(!auth_passes(&s, &"/?token=wrong".parse().unwrap()));
    }

    #[tokio::test]
    async fn index_open_serves_page() {
        let s = state_with(None, false);
        let resp = index(State(s), "/".parse().unwrap()).await;
        assert_eq!(resp.status(), axum::http::StatusCode::OK);
    }

    #[tokio::test]
    async fn index_gated_rejects_without_token() {
        let s = state_with(Some("secret"), true);
        let resp = index(State(s), "/".parse().unwrap()).await;
        assert_eq!(resp.status(), axum::http::StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn index_gated_accepts_valid_token() {
        let s = state_with(Some("secret"), true);
        let resp = index(State(s), "/?token=secret".parse().unwrap()).await;
        assert_eq!(resp.status(), axum::http::StatusCode::OK);
    }

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
