//! imessage-ap: a self-hosted archive and viewer for iMessage history.
//!
//! Milestone 1 provides the HTTP skeleton only. See `docs/ROADMAP.md`.

pub mod server;

/// Crate version, reported by `/api/health` and recorded with each import.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
