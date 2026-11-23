//! Heptapod forge integration.
//!
//! [Heptapod](https://about.heptapod.host/) is a GitLab fork that adds
//! Mercurial support: same REST API and merge-request model as GitLab, but the
//! underlying repository is Mercurial rather than Git. That has two
//! consequences for tuicr:
//!
//! - **Transport.** `glab` speaks the GitLab REST API and works against an
//!   authenticated Heptapod instance unchanged. The backend delegates to
//!   [`crate::forge::gitlab::GitLabGlabBackend`] for every API call.
//! - **Local checkout.** When the PR is opened from a local checkout of the
//!   MR branch, that checkout is Mercurial — so optimizations that the GitLab
//!   backend routes through `git show <sha>:<path>` and `git diff <a>..<b>`
//!   have to use `hg cat -r <sha>` and `hg diff -r <a> -r <b>` instead.
//!   [`HeptapodBackend`] detects the local VCS once at PR-open time and routes
//!   accordingly.
//!
//! Heptapod has no single canonical public host — `foss.heptapod.net` hosts
//! the community edition, commercial instances live on their own domains
//! (e.g. `orus.io`). Detection therefore mirrors the self-hosted Gerrit
//! pattern: a hostname containing "heptapod", or an explicit `HEPTAPOD_URL`
//! override. See [`parse::parse_heptapod_remote_url`].

pub mod glab;
pub mod parse;

pub use glab::HeptapodBackend;
