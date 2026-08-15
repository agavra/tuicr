//! Remote forge integration.
//!
//! This module is intentionally transport-focused for the first integration
//! slice. UI and review submission code should depend on the trait shape here
//! instead of shelling out to forge-specific tools directly.
#![allow(dead_code)]

pub mod azure;
pub mod bitbucket;
pub mod canonical;
pub mod context;
pub mod github;
pub mod gitlab;
pub mod pr_open;
pub mod remote_comments;
pub mod selector;
pub mod submit;
pub mod traits;

use std::path::{Path, PathBuf};

use git2::Repository;

use crate::forge::azure::az::parse_azure_remote_url;
use crate::forge::bitbucket::bkt::parse_bitbucket_remote_url;
use crate::forge::github::gh::parse_github_remote_url;
use crate::forge::gitlab::glab::parse_gitlab_remote_url;
use crate::forge::traits::ForgeRepository;

/// Percent-encode UTF-8 bytes for an API URL path. GitHub accepts a path
/// hierarchy, while GitLab expects the repository file path as one segment.
pub(crate) fn encode_api_path(path: &str, preserve_slashes: bool) -> String {
    use std::fmt::Write;

    let mut encoded = String::with_capacity(path.len());
    for byte in path.bytes() {
        if byte.is_ascii_alphanumeric()
            || matches!(byte, b'-' | b'.' | b'_' | b'~')
            || (preserve_slashes && byte == b'/')
        {
            encoded.push(char::from(byte));
        } else {
            write!(&mut encoded, "%{byte:02X}").expect("writing to a String cannot fail");
        }
    }
    encoded
}

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

/// `repo_root`'s remote URLs, `origin` first, then every other remote.
fn remote_urls(repo_root: &Path) -> Vec<String> {
    let Ok(repo) = Repository::discover(repo_root) else {
        return Vec::new();
    };
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
    all_urls
}

/// Try to detect an Azure DevOps forge repository for the local checkout at
/// `repo_root`. Looks at `origin` first, then any remote whose URL parses as an
/// Azure DevOps host. Returns `None` when no Azure remote is configured.
pub fn detect_azure_repository(repo_root: &Path) -> Option<ForgeRepository> {
    remote_urls(repo_root)
        .iter()
        .find_map(|url| parse_azure_remote_url(url))
}

/// Parse `url` as a forge remote repository.
///
/// Order matters. Bitbucket and GitLab both gate on the hostname, so trying
/// them first won't claim GitHub Enterprise remotes. Azure next — its parser
/// filters to `dev.azure.com` / `*.visualstudio.com` hosts. GitHub must stay
/// last because its parser accepts *any* host (covers github.com and GHE hosts
/// whose hostname does not literally contain "github") — it would otherwise
/// swallow every Bitbucket, self-hosted GitLab, and Azure remote.
pub fn parse_any_remote_url(url: &str) -> Option<ForgeRepository> {
    parse_bitbucket_remote_url(url)
        .or_else(|| parse_gitlab_remote_url(url))
        .or_else(|| parse_azure_remote_url(url))
        .or_else(|| parse_github_remote_url(url))
}

/// Detect the forge repository for the local checkout at `repo_root`.
/// Returns `None` when no remote can be parsed.
pub fn detect_forge_repository(repo_root: &Path) -> Option<ForgeRepository> {
    remote_urls(repo_root)
        .iter()
        .find_map(|url| parse_any_remote_url(url))
}

/// `root`'s local checkout, but only when one of its remotes — not
/// necessarily `origin` — matches `target_repo`.
pub fn local_checkout_for_repo(root: &Path, target_repo: &ForgeRepository) -> Option<PathBuf> {
    remote_urls(root)
        .iter()
        .any(|url| parse_any_remote_url(url).as_ref() == Some(target_repo))
        .then(|| root.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn init_repo_with_origin(url: &str) -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        let repo = Repository::init(dir.path()).expect("init repo");
        repo.remote("origin", url).expect("add origin");
        dir
    }

    #[test]
    fn api_path_encoding_handles_reserved_and_utf8_bytes() {
        assert_eq!(
            encode_api_path("dir/a #?%é.rs", true),
            "dir/a%20%23%3F%25%C3%A9.rs"
        );
        assert_eq!(
            encode_api_path("dir/a #?%é.rs", false),
            "dir%2Fa%20%23%3F%25%C3%A9.rs"
        );
    }

    #[test]
    fn detects_github_repository_from_origin() {
        let dir = init_repo_with_origin("https://github.com/agavra/tuicr");
        assert_eq!(
            detect_forge_repository(dir.path()),
            Some(ForgeRepository::github("github.com", "agavra", "tuicr"))
        );
    }

    #[test]
    fn local_checkout_matches_when_origin_equals_target() {
        let dir = init_repo_with_origin("https://github.com/agavra/tuicr");
        let target = ForgeRepository::github("github.com", "agavra", "tuicr");
        assert_eq!(
            local_checkout_for_repo(dir.path(), &target),
            Some(dir.path().to_path_buf())
        );
    }

    #[test]
    fn local_checkout_rejects_mismatched_repo() {
        let dir = init_repo_with_origin("https://github.com/contributor/tuicr");
        let target = ForgeRepository::github("github.com", "agavra", "tuicr");
        assert_eq!(local_checkout_for_repo(dir.path(), &target), None);
    }

    #[test]
    fn local_checkout_returns_none_outside_a_repo() {
        let dir = tempfile::tempdir().expect("tempdir");
        let target = ForgeRepository::github("github.com", "agavra", "tuicr");
        assert_eq!(local_checkout_for_repo(dir.path(), &target), None);
    }

    #[test]
    fn local_checkout_matches_upstream_remote_in_fork_workflow() {
        let dir = init_repo_with_origin("https://github.com/contributor/tuicr");
        let repo = Repository::open(dir.path()).expect("open repo");
        repo.remote("upstream", "https://github.com/agavra/tuicr")
            .expect("add upstream");

        let target = ForgeRepository::github("github.com", "agavra", "tuicr");
        assert_eq!(
            local_checkout_for_repo(dir.path(), &target),
            Some(dir.path().to_path_buf())
        );
    }
}
