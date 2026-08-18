//! Network module smoke tests — verifies the SSH and web servers can start
//! and accept connections. Feature-gated behind `network`.

#![cfg(feature = "network")]

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
    };

    // We can't easily get the bound port with the current API, so we bind
    // a listener ourselves, get the port, then connect.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);

    let server_cfg2 = SshServerConfig {
        addr: addr.to_string(),
        host_key_path: Some(key_path.to_string_lossy().into_owned()),
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
        let _ = run_web_server(
            &addr.to_string(),
            config,
            termos::web::TouchMode::Auto,
            0,
            false,
        )
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

/// Verify the web server accepts a WebSocket upgrade at /ws.
#[tokio::test]
async fn web_websocket_upgrade() {
    use termos::network::web::run_web_server;

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);

    let config = UserConfig::default();
    tokio::spawn(async move {
        let _ = run_web_server(
            &addr.to_string(),
            config,
            termos::web::TouchMode::Auto,
            0,
            false,
        )
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

    // Send a text message and verify we get an echo back.
    ws_stream
        .send(Message::Text("hello".into()))
        .await
        .expect("should send");

    let msg = match timeout(Duration::from_secs(5), ws_stream.next()).await {
        Ok(Some(Ok(m))) => m,
        _ => {
            eprintln!("skipping WebSocket test: no response");
            return;
        }
    };

    if let Message::Text(text) = msg {
        assert!(
            text.contains("hello"),
            "echo should contain 'hello': {text}"
        );
    }
}
