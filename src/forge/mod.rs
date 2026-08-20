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
use std::process::Command;

use git2::Repository;
use serde::Deserialize;

use crate::forge::azure::az::parse_azure_remote_url;
use crate::forge::bitbucket::bkt::parse_bitbucket_remote_url;
use crate::forge::github::gh::{parse_forgejo_remote_url, parse_github_remote_url};
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

fn named_remote_urls(repo_root: &Path) -> Vec<(String, String)> {
    let Ok(repo) = Repository::discover(repo_root) else {
        return Vec::new();
    };
    let mut remotes = Vec::new();
    if let Ok(remote) = repo.find_remote("origin")
        && let Some(url) = remote.url()
    {
        remotes.push(("origin".to_string(), url.to_string()));
    }
    if let Ok(names) = repo.remotes() {
        for name in names.iter().flatten() {
            if let Ok(remote) = repo.find_remote(name)
                && let Some(url) = remote.url()
            {
                remotes.push((name.to_string(), url.to_string()));
            }
        }
    }
    remotes
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
        .or_else(|| parse_codeberg_remote_url(url))
        .or_else(|| parse_github_remote_url(url))
}

/// Detect the forge repository for the local checkout at `repo_root`.
/// Returns `None` when no remote can be parsed.
pub fn detect_forge_repository(repo_root: &Path) -> Option<ForgeRepository> {
    let urls = remote_urls(repo_root);
    for url in &urls {
        if let Some(repository) = parse_bitbucket_remote_url(url)
            .or_else(|| parse_gitlab_remote_url(url))
            .or_else(|| parse_azure_remote_url(url))
        {
            return Some(repository);
        }
    }
    for (_, url) in named_remote_urls(repo_root) {
        if parse_github_remote_url(&url).is_some_and(|repository| repository.host != "github.com")
            && let Some(repository) = parse_forgejo_remote_url(&url)
            && let Some(login) = tea_login_for_host(&repository.host)
        {
            return Some(repository.with_tea_login(login));
        }
    }
    urls.iter().find_map(|url| parse_github_remote_url(url))
}

/// `root`'s local checkout, but only when one of its remotes — not
/// necessarily `origin` — matches `target_repo`.
pub fn local_checkout_for_repo(root: &Path, target_repo: &ForgeRepository) -> Option<PathBuf> {
    if target_repo.kind == crate::forge::traits::ForgeKind::Forgejo
        && named_remote_urls(root).iter().any(|(_, url)| {
            let Some(repository) = parse_forgejo_remote_url(url) else {
                return false;
            };
            tea_login_for_host(&repository.host).as_deref() == target_repo.tea_login.as_deref()
                && repository.host == target_repo.host
                && repository.owner == target_repo.owner
                && repository.name == target_repo.name
        })
    {
        return Some(root.to_path_buf());
    }
    remote_urls(root)
        .iter()
        .any(|url| parse_any_remote_url(url).as_ref() == Some(target_repo))
        .then(|| root.to_path_buf())
}

pub fn resolve_tea_login(mut repository: ForgeRepository) -> ForgeRepository {
    if repository.kind == crate::forge::traits::ForgeKind::Forgejo && repository.tea_login.is_none()
    {
        repository.tea_login = tea_login_for_host(&repository.host);
    }
    repository
}

fn parse_codeberg_remote_url(url: &str) -> Option<ForgeRepository> {
    let repository = parse_github_remote_url(url)?;
    repository
        .host
        .eq_ignore_ascii_case("codeberg.org")
        .then(|| ForgeRepository::forgejo(repository.host, repository.owner, repository.name))
}

#[derive(Deserialize)]
struct TeaLogin {
    name: String,
    url: String,
}

fn tea_login_for_host(host: &str) -> Option<String> {
    let output = Command::new("tea")
        .args(["logins", "list", "--output", "json"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let logins: Vec<TeaLogin> = serde_json::from_slice(&output.stdout).ok()?;
    logins.into_iter().find_map(|login| {
        tea_host(&login.url)
            .eq_ignore_ascii_case(host)
            .then_some(login.name)
    })
}

fn tea_host(url: &str) -> &str {
    url.strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .unwrap_or(url)
        .split('/')
        .next()
        .unwrap_or(url)
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
    fn detects_github_repository_from_origin() {
        let dir = init_repo_with_origin("https://github.com/agavra/tuicr");
        assert_eq!(
            detect_forge_repository(dir.path()),
            Some(ForgeRepository::github("github.com", "agavra", "tuicr"))
        );
    }

    #[test]
    fn parses_codeberg_url_as_forgejo() {
        assert_eq!(
            parse_any_remote_url("https://codeberg.org/team/service.git"),
            Some(ForgeRepository::forgejo("codeberg.org", "team", "service"))
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
