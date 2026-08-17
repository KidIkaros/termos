//! Web server network mode — serves TUIOS over HTTP + WebSocket.
//!
//! The Go reference uses a separate `tuios-web` binary with xterm.js for
//! security isolation. The Rust port uses `axum` for HTTP and
//! `tokio-tungstenite` for WebSocket, serving a static HTML page with
//! xterm.js that connects back over WebSocket. Terminal I/O is carried as
//! JSON frames: `{ "type": "input", "data": "..." }` and
//! `{ "type": "output", "data": "..." }`.

use std::net::SocketAddr;
use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::response::Html;
use axum::routing::get;
use axum::Router;
use tokio::sync::Mutex;

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

/// Handle a WebSocket connection: spawn a TUIOS session and bridge I/O.
async fn handle_ws(socket: WebSocket, state: WebServerState) {
    let mut os = Os::new((*state.config).clone());
    os.width = 80;
    os.height = 24;

    let _os = Arc::new(Mutex::new(os));

    // TODO: wire the Os's render output to the WebSocket and WebSocket
    // input to the Os's input handler. This requires a render loop that
    // draws to a buffer and sends the diff as ANSI escape sequences. For
    // now, this is a skeleton that echoes input.

    let mut socket = socket;
    while let Some(msg) = socket.recv().await {
        match msg {
            Ok(Message::Text(text)) => {
                let response = serde_json::json!({
                    "type": "output",
                    "data": text,
                });
                let _ = socket.send(Message::Text(response.to_string())).await;
            }
            Ok(Message::Binary(_)) => {}
            Ok(Message::Close(_)) | Err(_) => break,
            _ => {}
        }
    }
}
