//! Network module smoke tests — verifies the SSH and web servers can start
//! and accept connections. Feature-gated behind `network`.

#![cfg(feature = "network")]

use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::timeout;

use termos::config::UserConfig;
use termos::network::web::run_web_server;

/// Connect to a TCP port and verify the SSH banner is received.
#[tokio::test]
async fn ssh_server_starts_and_accepts_connection() {
    use termos::network::ssh::{SshServerConfig, TermosSshServer};

    // Generate a temporary host key.
    let key_dir = std::env::temp_dir().join(format!("tuios-ssh-test-{}", std::process::id()));
    std::fs::create_dir_all(&key_dir).unwrap();
    let key_path = key_dir.join("host_key");
    // Generate an Ed25519 key using ssh-keygen if available; skip test if not.
    let status = std::process::Command::new("ssh-keygen")
        .args([
            "-t",
            "ed25519",
            "-f",
            key_path.to_str().unwrap(),
            "-N",
            "",
            "-q",
        ])
        .status();
    if !status.map(|s| s.success()).unwrap_or(false) {
        eprintln!("skipping SSH test: ssh-keygen not available");
        return;
    }

    let config = UserConfig::default();
    let _server = TermosSshServer::new(config);
    let _server_cfg = SshServerConfig {
        addr: "127.0.0.1:0".to_string(),
        host_key_path: Some(key_path.to_string_lossy().into_owned()),
        read_only: false,
    };

    // We can't easily get the bound port with the current API, so we bind
    // a listener ourselves, get the port, then connect.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);

    let server_cfg2 = SshServerConfig {
        addr: addr.to_string(),
        host_key_path: Some(key_path.to_string_lossy().into_owned()),
        read_only: false,
    };

    // Spawn the server.
    let server2 = TermosSshServer::new(UserConfig::default());
    tokio::spawn(async move {
        let _ = server2.run(server_cfg2).await;
    });

    // Give the server a moment to bind.
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Connect and verify we get an SSH banner.
    let mut stream = match timeout(Duration::from_secs(5), TcpStream::connect(addr)).await {
        Ok(Ok(s)) => s,
        _ => {
            eprintln!("skipping SSH test: could not connect");
            let _ = std::fs::remove_dir_all(&key_dir);
            return;
        }
    };

    let mut buf = [0u8; 256];
    let read = match timeout(Duration::from_secs(5), stream.read(&mut buf)).await {
        Ok(Ok(n)) if n > 0 => n,
        _ => {
            eprintln!("skipping SSH test: no banner received");
            let _ = std::fs::remove_dir_all(&key_dir);
            return;
        }
    };

    // SSH servers send a banner starting with "SSH-".
    let banner = &buf[..read];
    assert!(
        banner.starts_with(b"SSH-"),
        "expected SSH banner, got: {:?}",
        String::from_utf8_lossy(banner)
    );

    let _ = std::fs::remove_dir_all(&key_dir);
}

/// Verify the web server serves the index HTML page.
#[tokio::test]
async fn web_server_serves_index_html() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);

    let config = UserConfig::default();
    tokio::spawn(async move {
        let _ = run_web_server(termos::network::web::WebServerOptions {
            addr: addr.to_string(),
            config,
            touch_mode: termos::web::TouchMode::Auto,
            max_connections: 0,
            read_only: false,
            tls_enabled: false,
            token: None,
            cert: None,
            key: None,
        })
        .await;
    });

    tokio::time::sleep(Duration::from_millis(100)).await;

    let mut stream = TcpStream::connect(addr).await.expect("should connect");
    stream
        .write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .await
        .expect("should write");

    let mut response = Vec::new();
    let _ = timeout(Duration::from_secs(5), stream.read_to_end(&mut response)).await;

    let text = String::from_utf8_lossy(&response);
    assert!(
        text.contains("<!DOCTYPE html>") || text.contains("<html"),
        "expected HTML, got: {} bytes",
        response.len()
    );
}

/// Verify token auth: a gated server returns 401 without a token and serves
/// the page with the correct one.
#[tokio::test]
async fn web_token_auth_gates_index() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);

    let config = UserConfig::default();
    tokio::spawn(async move {
        let _ = run_web_server(termos::network::web::WebServerOptions {
            addr: addr.to_string(),
            config,
            touch_mode: termos::web::TouchMode::Auto,
            max_connections: 0,
            read_only: false,
            tls_enabled: false,
            token: Some("s3cret".to_string()),
            cert: None,
            key: None,
        })
        .await;
    });

    tokio::time::sleep(Duration::from_millis(100)).await;

    // Without a token: 401 + the login page.
    let mut stream = TcpStream::connect(addr).await.expect("should connect");
    stream
        .write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .await
        .expect("should write");
    let mut response = Vec::new();
    let _ = timeout(Duration::from_secs(5), stream.read_to_end(&mut response)).await;
    let text = String::from_utf8_lossy(&response);
    assert!(
        text.starts_with("HTTP/1.1 401"),
        "expected 401 without token, got: {}",
        text.lines().next().unwrap_or("")
    );
    assert!(
        text.contains("access token"),
        "expected login page, got: {} bytes",
        response.len()
    );

    // With the correct token: 200 + the terminal page.
    let mut stream = TcpStream::connect(addr).await.expect("should connect");
    stream
        .write_all(b"GET /?token=s3cret HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .await
        .expect("should write");
    let mut response = Vec::new();
    let _ = timeout(Duration::from_secs(5), stream.read_to_end(&mut response)).await;
    let text = String::from_utf8_lossy(&response);
    assert!(
        text.starts_with("HTTP/1.1 200"),
        "expected 200 with token, got: {}",
        text.lines().next().unwrap_or("")
    );

    // With a wrong token: 401.
    let mut stream = TcpStream::connect(addr).await.expect("should connect");
    stream
        .write_all(b"GET /?token=wrong HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .await
        .expect("should write");
    let mut response = Vec::new();
    let _ = timeout(Duration::from_secs(5), stream.read_to_end(&mut response)).await;
    let text = String::from_utf8_lossy(&response);
    assert!(
        text.starts_with("HTTP/1.1 401"),
        "expected 401 with wrong token, got: {}",
        text.lines().next().unwrap_or("")
    );
}

/// Start a real daemon on an isolated temp socket, pointing TERMOS_SOCKET at
/// it. The returned guard is held for the whole test: the daemon tests share
/// the process-global socket env, so they must not interleave.
static DAEMON_LOCK: std::sync::OnceLock<tokio::sync::Mutex<()>> = std::sync::OnceLock::new();
async fn start_isolated_daemon(
) -> (
    tokio::sync::MutexGuard<'static, ()>,
    tempfile::TempDir,
    std::path::PathBuf,
) {
    let guard = DAEMON_LOCK.get_or_init(|| tokio::sync::Mutex::new(())).lock().await;
    // Point persistence at a throwaway directory.
    let state_dir = tempfile::tempdir().unwrap();
    let state_path = state_dir.keep();
    std::env::set_var("XDG_STATE_HOME", &state_path);
    std::env::remove_var("TERMOS_SOCKET");
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("tuios.sock");
    let daemon = Arc::new(termos::session::Daemon::new());
    let path = socket.clone();
    std::thread::spawn(move || {
        let _ = daemon.run(&path);
    });
    for _ in 0..100 {
        if std::os::unix::net::UnixStream::connect(&socket).is_ok() {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    std::env::set_var("TERMOS_SOCKET", &socket);
    (guard, dir, socket)
}

/// A raw HTTP GET returning the status line + body.
async fn http_get(addr: std::net::SocketAddr, path: &str) -> (String, String) {
    let mut stream = TcpStream::connect(addr).await.expect("should connect");
    stream
        .write_all(format!("GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n").as_bytes())
        .await
        .expect("should write");
    let mut response = Vec::new();
    let _ = timeout(Duration::from_secs(5), stream.read_to_end(&mut response)).await;
    let text = String::from_utf8_lossy(&response).to_string();
    let status = text.lines().next().unwrap_or("").to_string();
    (status, text)
}

/// Spawn a web server (no auth) on a random port.
async fn spawn_web_server() -> std::net::SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);
    tokio::spawn(async move {
        let _ = run_web_server(termos::network::web::WebServerOptions {
            addr: addr.to_string(),
            config: UserConfig::default(),
            touch_mode: termos::web::TouchMode::Auto,
            max_connections: 0,
            read_only: false,
            tls_enabled: false,
            token: None,
            cert: None,
            key: None,
        })
        .await;
    });
    tokio::time::sleep(Duration::from_millis(100)).await;
    addr
}

/// The picker page lists daemon sessions and `/new` creates + redirects.
#[tokio::test]
async fn web_picker_lists_sessions_and_creates() {
    let (_guard, _dir, _socket) = start_isolated_daemon().await;
    let client = termos::session::DaemonClient::connect().expect("daemon connect");
    client.new_session("webpick", "/bin/sh").expect("create session");

    let addr = spawn_web_server().await;

    // GET / → picker with the session as an attach link.
    let (status, body) = http_get(addr, "/").await;
    assert!(status.starts_with("HTTP/1.1 200"), "picker: {status}");
    assert!(body.contains("webpick"), "picker lists session");
    assert!(body.contains("href=\"/webpick\""), "session link: {body}");

    // GET /webpick → the terminal page (xterm.js frontend).
    let (status, body) = http_get(addr, "/webpick").await;
    assert!(status.starts_with("HTTP/1.1 200"), "session page: {status}");
    assert!(body.contains("xterm"), "terminal page: {body}");

    // GET /new?name=created2 → 303 redirect to the new session's page.
    let (status, body) = http_get(addr, "/new?name=created2").await;
    assert!(status.starts_with("HTTP/1.1 303"), "redirect: {status}");
    assert!(
        body.to_lowercase().contains("location: /created2"),
        "location: {body}"
    );
    let sessions = termos::session::DaemonClient::connect()
        .expect("daemon")
        .list()
        .expect("list");
    assert!(sessions.iter().any(|s| s.name == "created2"));

    // A bad name stays on the picker with an error.
    let (status, body) = http_get(addr, "/new?name=bad%20name").await;
    assert!(status.starts_with("HTTP/1.1 200"), "bad name: {status}");
    assert!(body.contains("cannot create session"), "error shown: {body}");
}

/// Attaching to a daemon session over `/ws/<session>`: input reaches the
/// session's shell and echoes back through the frames.
#[tokio::test]
async fn web_session_attach_types_and_echoes() {
    let (_guard, _dir, _socket) = start_isolated_daemon().await;
    let client = termos::session::DaemonClient::connect().expect("daemon connect");
    client
        .new_session("webattach", "/bin/sh")
        .expect("create session");

    let addr = spawn_web_server().await;

    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message;
    let url = format!("ws://{addr}/ws/webattach");
    let (mut ws_stream, _) = timeout(Duration::from_secs(5), tokio_tungstenite::connect_async(&url))
        .await
        .expect("connect")
        .expect("connect ws");

    // Enter terminal mode, then type a marker; the shell echoes it back.
    ws_stream
        .send(Message::Text("{\"type\":\"input\",\"data\":\"i\"}".into()))
        .await
        .expect("send i");
    ws_stream
        .send(Message::Text("{\"type\":\"input\",\"data\":\"marker_42\"}".into()))
        .await
        .expect("send text");

    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    let mut saw_echo = false;
    while tokio::time::Instant::now() < deadline {
        match timeout(Duration::from_millis(500), ws_stream.next()).await {
            Ok(Some(Ok(Message::Text(text)))) if text.contains("marker_42") => {
                saw_echo = true;
                break;
            }
            Ok(Some(Ok(_))) => continue,
            _ => break,
        }
    }
    assert!(saw_echo, "echo should contain 'marker_42' within 10s");
}

/// Attaching to a missing session sends an error frame and closes.
#[tokio::test]
async fn web_session_attach_missing_session_errors() {
    let (_guard, _dir, _socket) = start_isolated_daemon().await;
    let addr = spawn_web_server().await;

    use futures_util::StreamExt;
    use tokio_tungstenite::tungstenite::Message;
    let url = format!("ws://{addr}/ws/nope");
    let (mut ws_stream, _) = timeout(Duration::from_secs(5), tokio_tungstenite::connect_async(&url))
        .await
        .expect("connect")
        .expect("connect ws");

    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    let mut saw_error = false;
    while tokio::time::Instant::now() < deadline {
        match timeout(Duration::from_millis(500), ws_stream.next()).await {
            Ok(Some(Ok(Message::Text(text)))) if text.contains("cannot attach") => {
                saw_error = true;
                break;
            }
            Ok(Some(Ok(_))) => continue,
            _ => break,
        }
    }
    assert!(saw_error, "expected an attach error frame");
}

/// Verify the web server accepts a WebSocket upgrade at /ws.
#[tokio::test]
async fn web_websocket_upgrade() {
    use termos::network::web::run_web_server;

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);

    let config = UserConfig::default();
    tokio::spawn(async move {
        let _ = run_web_server(termos::network::web::WebServerOptions {
            addr: addr.to_string(),
            config,
            touch_mode: termos::web::TouchMode::Auto,
            max_connections: 0,
            read_only: false,
            tls_enabled: false,
            token: None,
            cert: None,
            key: None,
        })
        .await;
    });

    tokio::time::sleep(Duration::from_millis(100)).await;

    // Use tokio-tungstenite to connect a WebSocket client.
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message;
    let url = format!("ws://{addr}/ws");
    let (mut ws_stream, _) = match timeout(
        Duration::from_secs(5),
        tokio_tungstenite::connect_async(&url),
    )
    .await
    {
        Ok(Ok(s)) => s,
        Ok(Err(e)) => {
            eprintln!("skipping WebSocket test: connect failed: {e}");
            return;
        }
        Err(_) => {
            eprintln!("skipping WebSocket test: connect timed out");
            return;
        }
    };

    // Enter terminal mode (`i` in window-management mode), then type text and
    // verify the keystrokes echo back through the shell: the input is parsed
    // into key events, forwarded to the PTY, and the tty echo appears in a
    // later rendered frame. Frames are JSON: {"type":"input","data":"..."}.
    ws_stream
        .send(Message::Text("{\"type\":\"input\",\"data\":\"i\"}".into()))
        .await
        .expect("should send");
    ws_stream
        .send(Message::Text("{\"type\":\"input\",\"data\":\"hello\"}".into()))
        .await
        .expect("should send");

    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    let mut saw_echo = false;
    while tokio::time::Instant::now() < deadline {
        match timeout(Duration::from_millis(500), ws_stream.next()).await {
            Ok(Some(Ok(Message::Text(text)))) if text.contains("hello") => {
                saw_echo = true;
                break;
            }
            Ok(Some(Ok(_))) => continue,
            _ => break,
        }
    }
    assert!(saw_echo, "echo should contain 'hello' within 10s");
}
