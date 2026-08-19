//! Persistent sessions, the daemon, and the client control protocol —
//! ported from TUIOS `internal/session`. The daemon owns PTYs; clients run
//! their own emulator and renderer and exchange raw bytes with the daemon.

pub mod agent_detect;
pub mod agent_hold;
pub mod agent_osc;
pub mod agent_screen;
pub mod agent_source;
pub mod agent_state;
pub mod autostart;
pub mod client;
pub mod daemon;
pub mod debug;
pub mod eventhub;
pub mod events;
pub mod manager;
pub mod model;
pub mod osc_scan;
pub mod persistence;
pub mod protocol;
pub mod remote;
pub mod resurrection;
pub mod session_state;
pub mod startlock;
pub mod state_merge;
pub mod tree;
pub mod verb;

pub use client::DaemonClient;
pub use daemon::{default_socket_path, ensure_daemon_running, Daemon};
pub use manager::{Manager, ManagerError};
pub use model::{
    validate_session_name, Session, SessionConfig, SessionInfo, SessionState, WindowInfo,
    WindowState,
};
pub use protocol::{negotiate_codec, Codec, DaemonReport, Message, SessionReport};
pub use remote::RemoteSink;
pub use resurrection::{
    clean_resurrection_dir, list_resurrectable_infos, list_resurrectable_sessions,
    load_resurrection_state, remove_resurrection_state, resurrection_dir, resurrection_path,
    save_session_for_resurrection, ResurrectableInfo, RESURRECTION_INTERVAL, RESURRECTION_VERSION,
};
pub use verb::{
    VerbError, VerbHint, VerbRegistry, VerbRequest, VerbResponse, VERB_PROTOCOL_VERSION,
};
pub mod verb_client;
pub use verb_client::{VerbClient, VerbClientError};
