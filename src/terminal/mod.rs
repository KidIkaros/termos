//! PTY management and the terminal window — ported from TUIOS
//! `internal/terminal`.

pub mod pty;
pub mod window;

pub use pty::{spawn_pty, PtyError, PtyHandle, PtyReader, PtySink, PtyWriter};
pub use window::Window;
