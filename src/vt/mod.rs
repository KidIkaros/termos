//! The VT emulator — a virtual terminal used by every pane.
//!
//! Ported from TUIOS `internal/vt`. The emulator owns a parser, two screen
//! buffers (main + alt), scrollback, and the SGR/CSI/OSC/ESC handlers that
//! turn the PTY byte stream into screen state the renderer can paint.

pub mod cell;
pub mod charset;
pub mod emulator;
pub mod parser;
pub mod screen;
pub mod scrollback;

pub use cell::{Cell, Color, Decoration, Link, Style};
pub use emulator::{Emulator, MODE_ALT_SCREEN, MODE_AUTO_WRAP};
pub use parser::{CsiSequence, DcsSequence, Handler, OscSequence, Param, Parser, State, StringSequence};
pub use screen::{Position, ScreenBuffer, ScrollRegion};
pub use scrollback::Scrollback;
