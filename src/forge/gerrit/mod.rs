//! Gerrit Code Review integration, driven by the Gerrit REST API.
//!
//! Gerrit ships no companion CLI (the other backends wrap `gh`, `glab`, `bkt`,
//! and `az`), so `api.rs` speaks HTTP directly. Gerrit is always self-hosted,
//! which also means there is no reserved hostname to detect: see
//! [`api::parse_gerrit_remote_url`] for the signals used instead.

pub mod api;
pub mod models;

pub use api::GerritBackend;
