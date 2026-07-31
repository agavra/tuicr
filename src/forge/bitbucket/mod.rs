//! Bitbucket Cloud integration, driven by the `bkt` CLI.
//!
//! Data Center is deliberately out of scope: it exposes an unrelated REST 1.0
//! API, so `parse_bitbucket_remote_url` accepts `bitbucket.org` only and any
//! self-hosted remote falls through to the other forge parsers.

pub mod bkt;
pub mod models;

pub use bkt::BitbucketBktBackend;
