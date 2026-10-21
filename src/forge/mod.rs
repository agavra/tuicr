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
pub mod gerrit;
pub mod gitea;
pub mod github;
pub mod gitlab;
pub mod heptapod;
pub mod local_git;
pub mod local_hg;
pub mod pr_open;
pub mod remote_comments;
pub mod selector;
pub mod submit;
pub mod traits;

use std::path::{Path, PathBuf};

use git2::Repository;

use crate::error::{Result, TuicrError};
use crate::forge::azure::az::parse_azure_remote_url;
use crate::forge::bitbucket::bkt::parse_bitbucket_remote_url;
use crate::forge::gerrit::api::parse_gerrit_remote_url;
use crate::forge::gitea::tea::parse_gitea_remote_url;
use crate::forge::github::gh::parse_github_remote_url;
use crate::forge::gitlab::glab::{parse_gitlab_remote_url, parse_gitlab_remote_url_by_hostname};
use crate::forge::heptapod::parse::{
    parse_heptapod_remote_url, parse_heptapod_remote_url_by_hostname,
};
use crate::forge::traits::ForgeRepository;
use crate::vcs::VcsBackend;

/// Run two independent forge requests at the same time and return both
/// results in argument order.
///
/// `first` runs on the calling thread and `second` on a scoped thread, so a
/// call costs the slower of the two round trips instead of their sum. Both
/// always run to completion; callers decide which error wins, which keeps
/// error reporting as deterministic as the sequential code it replaces. A
/// panic in `second` is re-raised on the calling thread once `first` returns.
pub(crate) fn join<A, B, RA, RB>(first: A, second: B) -> (RA, RB)
where
    A: FnOnce() -> RA,
    B: FnOnce() -> RB + Send,
    RB: Send,
{
    std::thread::scope(|scope| {
        let second = scope.spawn(second);
        let first = first();
        let second = second
            .join()
            .unwrap_or_else(|panic| std::panic::resume_unwind(panic));
        (first, second)
    })
}

/// Return `Some(sha)` for `git merge-base <base> <head>` when both commits
/// are present in the checkout at `repo_root`.
///
/// Forge PR diffs are three-dot, so this — not the base branch tip a forge
/// reports — is the revision the patch's old side lives at. Returns `None`
/// when either object is missing or git fails, so callers can fall back to
/// the forge (or leave `base_sha` unrefined). See the `base_sha` gotcha in AGENTS.md.
pub(crate) fn local_merge_base(repo_root: &Path, base_sha: &str, head_sha: &str) -> Option<String> {
    use std::ffi::OsStr;

    for sha in [base_sha, head_sha] {
        crate::process::run_command_output(
            "git",
            Some(repo_root),
            ["cat-file", "-e", sha].iter().map(|s| OsStr::new(*s)),
        )
        .ok()?;
    }
    let output = crate::process::run_command_output(
        "git",
        Some(repo_root),
        ["merge-base", base_sha, head_sha]
            .iter()
            .map(|s| OsStr::new(*s)),
    )
    .ok()?;
    let sha = output.trim();
    (!sha.is_empty()).then(|| sha.to_string())
}

/// Try to detect a GitHub forge repository for the local checkout at `repo_root`.
///
/// Looks at the `origin` remote first, then falls back to any remote whose URL
/// parses as a GitHub host. Returns `None` when no GitHub remote is configured.
pub fn detect_github_repository(repo_root: &Path) -> Option<ForgeRepository> {
    remote_urls(repo_root)
        .iter()
        .find_map(|url| parse_github_remote_url(url))
}

/// Try to detect a GitLab forge repository for the local checkout at `repo_root`.
///
/// Looks at the `origin` remote first, then falls back to any remote whose URL
/// parses as a GitLab host. Returns `None` when no GitLab remote is configured.
pub fn detect_gitlab_repository(repo_root: &Path) -> Option<ForgeRepository> {
    remote_urls(repo_root)
        .iter()
        .find_map(|url| parse_gitlab_remote_url(url))
}

/// `repo_root`'s remote URLs, `origin` first, then every other remote.
///
/// Reads Git remotes first (libgit2 discovery, then the `git` CLI for
/// repository formats libgit2 cannot open). When neither finds a Git
/// repository, a Mercurial checkout still carries remotes — `hg paths` —
/// and a forge can live behind them (Heptapod), so those are read before
/// giving up; `default` is Mercurial's `origin` and sorts first.
fn remote_urls(repo_root: &Path) -> Vec<String> {
    let Ok(repo) = Repository::discover(repo_root) else {
        // Local VCS discovery already falls back to Git for repository formats
        // libgit2 cannot open (notably reftable and SHA-256). Remote metadata
        // needs the same fallback; no object access or network is necessary.
        let Ok(names) = crate::process::run_command_output("git", Some(repo_root), ["remote"])
        else {
            // Not a Git repository: the checkout may still be Mercurial, and
            // `hg paths` is where its remotes live.
            return hg_remote_urls(repo_root);
        };
        let mut names: Vec<_> = names.lines().collect();
        names.sort_by_key(|name| *name != "origin");
        return names
            .into_iter()
            .filter_map(|name| {
                crate::process::run_command_output(
                    "git",
                    Some(repo_root),
                    ["remote", "get-url", "--", name],
                )
                .ok()
                .map(|url| url.trim_end_matches(['\r', '\n']).to_string())
            })
            .collect();
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

/// `repo_root`'s hg remote URLs, `default` first, then every other named
/// path. Returns an empty vec when `repo_root` is not an hg checkout or
/// `hg` is missing, so a non-hg directory costs one failed spawn.
///
/// Mercurial records clone sources as named paths in `.hg/hgrc`; `default`
/// is what `hg clone` writes and the closest thing to Git's `origin`.
fn hg_remote_urls(repo_root: &Path) -> Vec<String> {
    let Ok(output) = crate::process::run_command_output("hg", Some(repo_root), ["paths"]) else {
        return Vec::new();
    };
    let mut entries: Vec<(&str, &str)> = output
        .lines()
        .filter_map(|line| {
            // `hg paths` prints `name = url`. Split on the first `=` and
            // trim both halves so any spacing survives; path names cannot
            // contain `=`, and the URL may (query strings).
            let (name, url) = line.split_once('=')?;
            let (name, url) = (name.trim(), url.trim());
            (!name.is_empty() && !url.is_empty()).then_some((name, url))
        })
        .collect();
    entries.sort_by_key(|(name, _)| *name != "default");
    entries
        .into_iter()
        .map(|(_, url)| url.to_string())
        .collect()
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
/// them first won't claim GitHub Enterprise remotes. Heptapod follows: it
/// speaks GitLab's API and URL grammar, so it must come before GitLab or
/// every Heptapod remote would be tagged as GitLab and lose the Mercurial
/// local-checkout routing. Azure next — its parser filters to
/// `dev.azure.com` / `*.visualstudio.com` hosts. Gitea then, which recognizes
/// its own public hosts by name and self-hosted ones through the logins
/// configured in `tea`. Gerrit follows: it is always self-hosted, so it gates
/// on the canonical SSH port `29418`, a configured `GERRIT_URL`, or a hostname
/// containing "gerrit". GitHub must stay last because its parser accepts any
/// host (covers github.com and GHE hosts whose hostname does not literally
/// contain "github") — it would otherwise swallow every Bitbucket, self-hosted
/// GitLab, Heptapod, Gitea, Azure, and Gerrit remote.
pub fn parse_any_remote_url(url: &str) -> Option<ForgeRepository> {
    parse_bitbucket_remote_url(url)
        .or_else(|| parse_heptapod_remote_url(url))
        .or_else(|| parse_gitlab_remote_url(url))
        .or_else(|| parse_azure_remote_url(url))
        .or_else(|| parse_gitea_remote_url(url))
        .or_else(|| parse_gerrit_remote_url(url))
        .or_else(|| parse_github_remote_url(url))
}

/// Parse `url` using only the forge parsers that decide from the URL alone —
/// no subprocess, no `~/.ssh/config` read — and without the GitHub catch-all.
///
/// For hot paths such as [`crate::slug::resolve_owner_repo`], which runs on
/// every session save, and only where an unrecognized remote has a usable
/// fallback. Prefer [`parse_any_remote_url`] everywhere else: it recognizes
/// more remotes, at the cost of a `glab config get host` spawn and up to three
/// `~/.ssh/config` reads per call.
///
/// GitHub is excluded on purpose. Its parser accepts any host and reads the
/// *first* two path segments, where `slug.rs`'s generic fallback reads the last
/// two; letting it claim unknown hosts would turn
/// `code.example.com/git/owner/repo` into `git/owner`. Bitbucket is left out
/// because a workspace is always one segment, so the fallback already agrees
/// with it. GitLab enters as its hostname-only variant, which forgoes the
/// `glab` config lookup that would otherwise recognize self-hosted instances on
/// custom domains. Heptapod's hostname-only variant is included for the same
/// reason: an `foss.heptapod.net` remote should be recognized without spawning
/// a process. Gerrit is included: its project path can be any depth, so the
/// last-two-segments rule would mis-split `gerrit.example.com/platform/frameworks/base`,
/// and its parser only reads the URL plus one env var.
pub fn parse_any_remote_url_by_hostname(url: &str) -> Option<ForgeRepository> {
    parse_heptapod_remote_url_by_hostname(url)
        .or_else(|| parse_gitlab_remote_url_by_hostname(url))
        .or_else(|| parse_azure_remote_url(url))
        .or_else(|| parse_gerrit_remote_url(url))
}

/// Detect the forge repository for the local checkout at `repo_root`.
/// Returns `None` when no remote can be parsed.
pub fn detect_forge_repository(repo_root: &Path) -> Option<ForgeRepository> {
    remote_urls(repo_root)
        .iter()
        .find_map(|url| parse_any_remote_url(url))
}

/// Resolve a VCS remote to a supported forge repository.
pub fn resolve_remote_repository(vcs: &dyn VcsBackend, name: &str) -> Result<ForgeRepository> {
    let url = vcs
        .remote_url(name)
        .map_err(|err| TuicrError::Forge(format!("Failed to resolve remote '{name}': {err}")))?;
    parse_any_remote_url(url.trim()).ok_or_else(|| {
        TuicrError::Forge(format!(
            "Remote '{name}' does not have a recognized forge fetch URL"
        ))
    })
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

    #[test]
    fn join_returns_both_results_in_argument_order() {
        let (first, second) = join(|| "first", || 2);
        assert_eq!((first, second), ("first", 2));
    }

    #[test]
    #[should_panic(expected = "second failed")]
    fn join_re_raises_a_panic_from_the_spawned_side() {
        join(|| (), || -> () { panic!("second failed") });
    }

    fn init_repo_with_origin(url: &str) -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        let repo = Repository::init(dir.path()).expect("init repo");
        repo.remote("origin", url).expect("add origin");
        dir
    }

    fn hg_available() -> bool {
        std::process::Command::new("hg")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    /// Create an hg checkout whose `default` path points at `url` — what
    /// `hg clone <url>` leaves behind.
    fn init_hg_repo_with_default_path(url: &str) -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        crate::process::run_command_output("hg", Some(dir.path()), ["init"]).expect("hg init");
        std::fs::write(
            dir.path().join(".hg/hgrc"),
            format!("[paths]\ndefault = {url}\n"),
        )
        .expect("write hgrc");
        dir
    }

    #[test]
    fn detects_heptapod_repository_from_an_hg_checkout() {
        // A pure Mercurial checkout has no Git remotes to read; forge
        // detection must fall back to `hg paths`, where `hg clone` records
        // the source URL.
        if !hg_available() {
            return;
        }
        let dir = init_hg_repo_with_default_path("https://foss.heptapod.net/owner/repo");
        assert_eq!(
            detect_forge_repository(dir.path()),
            Some(ForgeRepository::heptapod(
                "foss.heptapod.net",
                "owner",
                "repo"
            ))
        );
    }

    #[test]
    fn hg_default_path_sorts_first_among_hg_remotes() {
        if !hg_available() {
            return;
        }
        let dir = tempfile::tempdir().expect("tempdir");
        crate::process::run_command_output("hg", Some(dir.path()), ["init"]).expect("hg init");
        // `upstream` first in the hgrc so the sort, not the file order, is
        // what puts `default` ahead.
        std::fs::write(
            dir.path().join(".hg/hgrc"),
            "[paths]\nupstream = https://foss.heptapod.net/other/repo\ndefault = https://foss.heptapod.net/owner/repo\n",
        )
        .expect("write hgrc");
        assert_eq!(
            remote_urls(dir.path()),
            vec![
                "https://foss.heptapod.net/owner/repo".to_string(),
                "https://foss.heptapod.net/other/repo".to_string(),
            ]
        );
    }

    #[test]
    fn git_remotes_win_in_a_colocated_checkout() {
        // A colocated repo has both `.git` and `.hg`; the Git remotes are
        // the source of truth and the hg paths must not be consulted.
        if !hg_available() {
            return;
        }
        let dir = init_repo_with_origin("https://github.com/agavra/tuicr");
        crate::process::run_command_output("hg", Some(dir.path()), ["init"]).expect("hg init");
        std::fs::write(
            dir.path().join(".hg/hgrc"),
            "[paths]\ndefault = https://foss.heptapod.net/other/repo\n",
        )
        .expect("write hgrc");
        assert_eq!(
            detect_forge_repository(dir.path()),
            Some(ForgeRepository::github("github.com", "agavra", "tuicr"))
        );
    }

    #[test]
    fn hg_fallback_yields_nothing_outside_any_repo() {
        if !hg_available() {
            return;
        }
        let dir = tempfile::tempdir().expect("tempdir");
        assert!(remote_urls(dir.path()).is_empty());
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
    fn hostname_only_parser_claims_azure_but_leaves_other_hosts_alone() {
        assert_eq!(
            parse_any_remote_url_by_hostname("https://dev.azure.com/myorg/myproject/_git/myrepo"),
            Some(ForgeRepository::azure(
                "dev.azure.com",
                "myorg/myproject",
                "myrepo"
            ))
        );
        assert_eq!(
            parse_any_remote_url_by_hostname("git@gitlab.com:org/team/svc.git"),
            Some(ForgeRepository::gitlab("gitlab.com", "org/team", "svc"))
        );
        // The GitHub catch-all is excluded, so anything it would have claimed
        // falls through to the caller's own rule. Were it in the chain, its
        // first-two-segments reading would turn the last URL into `git/owner`.
        assert_eq!(
            parse_any_remote_url_by_hostname("https://github.com/agavra/tuicr"),
            None
        );
        assert_eq!(
            parse_any_remote_url_by_hostname("https://code.example.com/git/owner/repo"),
            None
        );
    }

    #[test]
    fn detects_gerrit_repository_from_a_canonical_ssh_port_remote() {
        // A neutral hostname on port 29418 is unambiguously Gerrit; without
        // this arm the GitHub catch-all would claim it as `platform/base`.
        let dir =
            init_repo_with_origin("ssh://jdoe@review.internal:29418/platform/frameworks/base");
        assert_eq!(
            detect_forge_repository(dir.path()),
            Some(ForgeRepository::gerrit(
                "review.internal",
                "platform/frameworks/base"
            ))
        );
    }

    #[test]
    fn leaves_github_enterprise_remotes_to_the_github_catch_all() {
        // Gerrit sits ahead of GitHub in the chain, so it must not claim a
        // self-hosted host that shows none of its signals.
        let dir = init_repo_with_origin("https://code.example.com/owner/repo.git");
        assert_eq!(
            detect_forge_repository(dir.path()),
            Some(ForgeRepository::github("code.example.com", "owner", "repo"))
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
