//! Azure DevOps forge integration.
//!
//! Talks to the Azure DevOps REST API through the Azure CLI's `az rest`
//! (reusing `az login` auth), mirroring the "shell out to an installed CLI via
//! a mockable runner" pattern used by the GitHub (`gh`) and GitLab (`glab`)
//! backends. Diffs are sourced from a local clone of the repo, because Azure
//! DevOps exposes no single unified-diff REST endpoint.

pub mod az;
pub mod models;

pub use az::AzureDevOpsBackend;
