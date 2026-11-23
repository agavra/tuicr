//! Gitea backend, driven by the `tea` CLI.
//!
//! Scoped to Gitea itself and to Forgejo (issue #518): Forgejo is a Gitea fork
//! that serves the same REST v1 API, so both are served by `tea`. Repositories
//! on Forgejo hosts get `ForgeKind::Forgejo`; everything else is
//! `ForgeKind::Gitea`. Codeberg is *not* claimed: it is an independent Gitea
//! deployment and stays reachable through a `tea` login as Gitea.

pub mod models;
pub mod tea;

pub use tea::GiteaTeaBackend;
