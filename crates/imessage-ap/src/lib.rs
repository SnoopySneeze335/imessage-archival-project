//! imessage-ap: a self-hosted archive and viewer for iMessage history.
//!
//! See `docs/ROADMAP.md` for what is built so far.

pub mod address;
pub mod archive;
pub mod backup;
pub mod ingest;
pub mod server;
pub mod stats;

/// Crate version, reported by `/api/health` and recorded with each import.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
