//! Gitea backend, driven by the `tea` CLI.
//!
//! Scoped to Gitea itself. Forks such as Forgejo and Codeberg currently serve
//! a compatible REST v1 API and will be routed here when the user has a `tea`
//! login for them, but tuicr does not claim to support them: they are
//! independent projects free to diverge, and nothing here is tested against
//! them.

pub mod models;
pub mod tea;

pub use tea::GiteaTeaBackend;
