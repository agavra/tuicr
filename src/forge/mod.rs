//! Remote forge integration.
//!
//! This module is intentionally transport-focused for the first integration
//! slice. UI and review submission code should depend on the trait shape here
//! instead of shelling out to forge-specific tools directly.
#![allow(dead_code)]

pub mod canonical;
pub mod context;
pub mod github;
pub mod gitlab;
pub mod pr_open;
pub mod remote_comments;
pub mod selector;
pub mod submit;
pub mod traits;

use std::path::Path;

use git2::Repository;

use crate::forge::github::gh::parse_github_remote_url;
use crate::forge::gitlab::glab::parse_gitlab_remote_url;
use crate::forge::traits::ForgeRepository;

/// Try to detect a GitHub forge repository for the local checkout at `repo_root`.
///
/// Looks at the `origin` remote first, then falls back to any remote whose URL
/// parses as a GitHub host. Returns `None` when no GitHub remote is configured.
pub fn detect_github_repository(repo_root: &Path) -> Option<ForgeRepository> {
    let repo = Repository::discover(repo_root).ok()?;
    if let Ok(remote) = repo.find_remote("origin")
        && let Some(url) = remote.url()
        && let Some(parsed) = parse_github_remote_url(url)
    {
        return Some(parsed);
    }
    let remotes = repo.remotes().ok()?;
    for name in remotes.iter().flatten() {
        if let Ok(remote) = repo.find_remote(name)
            && let Some(url) = remote.url()
            && let Some(parsed) = parse_github_remote_url(url)
        {
            return Some(parsed);
        }
    }
    None
}

/// Try to detect a GitLab forge repository for the local checkout at `repo_root`.
///
/// Looks at the `origin` remote first, then falls back to any remote whose URL
/// parses as a GitLab host. Returns `None` when no GitLab remote is configured.
pub fn detect_gitlab_repository(repo_root: &Path) -> Option<ForgeRepository> {
    let repo = Repository::discover(repo_root).ok()?;
    if let Ok(remote) = repo.find_remote("origin")
        && let Some(url) = remote.url()
        && let Some(parsed) = parse_gitlab_remote_url(url)
    {
        return Some(parsed);
    }
    let remotes = repo.remotes().ok()?;
    for name in remotes.iter().flatten() {
        if let Ok(remote) = repo.find_remote(name)
            && let Some(url) = remote.url()
            && let Some(parsed) = parse_gitlab_remote_url(url)
        {
            return Some(parsed);
        }
    }
    None
}

/// Detect the forge repository for the local checkout at `repo_root`.
///
/// Tries GitHub (host must contain "github") first, then GitLab (host must
/// contain "gitlab"). Returns `None` when no recognized remote is found.
pub fn detect_forge_repository(repo_root: &Path) -> Option<ForgeRepository> {
    let repo = Repository::discover(repo_root).ok()?;
    let mut all_urls: Vec<String> = Vec::new();

    if let Ok(remote) = repo.find_remote("origin")
        && let Some(url) = remote.url()
    {
        all_urls.push(url.to_string());
    }
    if let Ok(remotes) = repo.remotes() {
        for name in remotes.iter().flatten() {
            if let Ok(remote) = repo.find_remote(name)
                && let Some(url) = remote.url()
            {
                all_urls.push(url.to_string());
            }
        }
    }

    // Try GitHub first (filter to hosts containing "github").
    for url in &all_urls {
        if let Some(parsed) = parse_github_remote_url(url)
            && parsed.host.contains("github")
        {
            return Some(parsed);
        }
    }
    // Then try GitLab (parse_gitlab_remote_url already filters by "gitlab" host).
    for url in &all_urls {
        if let Some(parsed) = parse_gitlab_remote_url(url) {
            return Some(parsed);
        }
    }
    None
}

