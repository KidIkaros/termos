//! Agent harness detection and manifests — ported from TUIOS `internal/harness`.
//!
//! The registry answers one question: which harness, if any, is this process.
//! It deliberately does not answer "what is that harness doing"; the sources
//! that can answer that honestly are the harness reporting for itself and the
//! escape sequences it emits.

pub mod classify;
pub mod manifest;
pub mod registry;

pub use manifest::{DetectSpec, Manifest, ScreenRule, ScreenSpec, SCHEMA_VERSION};
pub use registry::{user_dir, LoadError, Registry};
