//! Persistent sessions, the daemon, and the client control protocol —
//! ported from TUIOS `internal/session`. The daemon owns PTYs; clients run
//! their own emulator and renderer and exchange raw bytes with the daemon.

pub mod agent_state;
pub mod client;
pub mod daemon;
pub mod events;
pub mod manager;
pub mod model;
pub mod persistence;
pub mod protocol;
pub mod remote;
pub mod resurrection;
pub mod tree;
pub mod verb;

pub use client::DaemonClient;
pub use daemon::{default_socket_path, ensure_daemon_running, Daemon};
pub use manager::{Manager, ManagerError};
pub use model::{
    validate_session_name, Session, SessionConfig, SessionInfo, SessionState, WindowInfo,
    WindowState,
};
pub use protocol::Message;
pub use remote::RemoteSink;
pub use resurrection::{
    clean_resurrection_dir, list_resurrectable_infos, list_resurrectable_sessions,
    load_resurrection_state, remove_resurrection_state, resurrection_dir, resurrection_path,
    save_session_for_resurrection, ResurrectableInfo, RESURRECTION_INTERVAL, RESURRECTION_VERSION,
};
pub use verb::{
    VerbError, VerbHint, VerbRegistry, VerbRequest, VerbResponse, VERB_PROTOCOL_VERSION,
};
