//! Persistent sessions, the daemon, and the client control protocol —
//! ported from TUIOS `internal/session`. The daemon owns PTYs; clients run
//! their own emulator and renderer and exchange raw bytes with the daemon.

pub mod client;
pub mod daemon;
pub mod manager;
pub mod model;
pub mod persistence;
pub mod protocol;
pub mod remote;
pub mod tree;

pub use client::DaemonClient;
pub use remote::RemoteSink;
pub use daemon::{default_socket_path, ensure_daemon_running, Daemon};
pub use manager::{Manager, ManagerError};
pub use model::{
    validate_session_name, Session, SessionConfig, SessionInfo, SessionState, WindowInfo,
    WindowState,
};
pub use protocol::Message;
