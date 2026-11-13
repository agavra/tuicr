//! Remote forge integration.
//!
//! This module is intentionally transport-focused for the first integration
//! slice. UI and review submission code should depend on the trait shape here
//! instead of shelling out to forge-specific tools directly.

pub mod azure;
pub mod bitbucket;
pub mod canonical;
pub mod context;
pub mod gerrit;
pub mod gitea;
pub mod github;
pub mod gitlab;
pub mod local_git;
pub mod pr_open;
pub mod remote_comments;
pub mod remote_url;
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
/// A repository-relative path as the forge spells it: `/`-separated. Only
/// Windows paths carry `\\` as a separator; on Unix it is a legal filename byte
/// and must reach the forge untouched.
pub(crate) fn forge_path(path: &Path) -> String {
    let path = path.to_string_lossy();
    if cfg!(windows) {
        path.replace('\\', "/")
    } else {
        path.into_owned()
    }
}

/// Percent-encode a path, keeping `/` intact so a repository-relative path
/// stays a path.
pub(crate) fn percent_encode_path(value: &str) -> String {
    value
        .split('/')
        .map(percent_encode_component)
        .collect::<Vec<_>>()
        .join("/")
}

/// Percent-encode everything but RFC 3986 unreserved bytes (`/` and `%` too).
pub(crate) fn percent_encode_component(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                encoded.push(byte as char);
            }
            _ => encoded.push_str(&format!("%{byte:02X}")),
        }
    }
    encoded
}

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
/// the forge (or leave `base_sha` unrefined). See gotcha F6 in docs/FORGES.md.
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

/// Return `Some(diff)` for `git diff <start>..<end>` when both SHAs exist in
/// the checkout at `repo_root`; `None` lets callers fall back to the forge.
pub(crate) fn local_range_diff(
    repo_root: &Path,
    start_sha: &str,
    end_sha: &str,
) -> Option<Vec<crate::model::FilePatch>> {
    use std::ffi::OsStr;

    for sha in [start_sha, end_sha] {
        crate::process::run_command_output(
            "git",
            Some(repo_root),
            ["cat-file", "-e", sha].iter().map(|s| OsStr::new(*s)),
        )
        .ok()?;
    }
    let range = format!("{start_sha}..{end_sha}");
    crate::vcs::git::raw::run_git_diff(repo_root, &[range.as_str()]).ok()
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
fn remote_urls(repo_root: &Path) -> Vec<String> {
    remotes(repo_root).into_iter().map(|(_, url)| url).collect()
}

/// `repo_root`'s `(name, url)` remotes, `origin` first, each remote once.
pub(crate) fn remotes(repo_root: &Path) -> Vec<(String, String)> {
    let Ok(repo) = Repository::discover(repo_root) else {
        // Local VCS discovery already falls back to Git for repository formats
        // libgit2 cannot open (notably reftable and SHA-256). Remote metadata
        // needs the same fallback; no object access or network is necessary.
        let Ok(names) = crate::process::run_command_output("git", Some(repo_root), ["remote"])
        else {
            return Vec::new();
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
                .map(|url| {
                    (
                        name.to_string(),
                        url.trim_end_matches(['\r', '\n']).to_string(),
                    )
                })
            })
            .collect();
    };
    let mut all: Vec<(String, String)> = Vec::new();
    let mut push = |name: &str| {
        if let Ok(remote) = repo.find_remote(name)
            && let Some(url) = remote.url()
        {
            all.push((name.to_string(), url.to_string()));
        }
    };
    push("origin");
    if let Ok(remotes) = repo.remotes() {
        for name in remotes.iter().flatten().filter(|name| *name != "origin") {
            push(name);
        }
    }
    all
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
/// accepts cloud hosts and the self-hosted hostname explicitly configured via
/// `AZURE_DEVOPS_SERVER_URL`. Gitea then, which
/// recognizes its own public hosts by name and self-hosted ones through the
/// logins configured in `tea`. Gerrit follows: it is always self-hosted, so it
/// gates on the canonical SSH port `29418`, a configured `GERRIT_URL`, or a
/// hostname containing "gerrit". GitHub must stay last because its parser
/// accepts *any* host (covers github.com and GHE hosts whose hostname does not
/// literally contain "github") — it would otherwise swallow every Bitbucket,
/// self-hosted GitLab, Gitea, Azure, and Gerrit remote.
pub fn parse_any_remote_url(url: &str) -> Option<ForgeRepository> {
    parse_bitbucket_remote_url(url)
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
/// custom domains. Gerrit is included: its project path can be any depth, so the
/// last-two-segments rule would mis-split `gerrit.example.com/platform/frameworks/base`,
/// and its parser only reads the URL plus one env var.
pub fn parse_any_remote_url_by_hostname(url: &str) -> Option<ForgeRepository> {
    parse_gitlab_remote_url_by_hostname(url)
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

/// Serializes tests that point `GIT_SSH_COMMAND` at a stub transport.
#[cfg(test)]
pub(crate) static GIT_SSH_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_range_diff_needs_both_shas_in_the_checkout() {
        let dir = tempfile::tempdir().unwrap();
        let git = |args: &[&str]| {
            let out = std::process::Command::new("git")
                .args(args)
                .current_dir(dir.path())
                .output()
                .unwrap();
            assert!(out.status.success(), "git {args:?}");
            String::from_utf8(out.stdout).unwrap().trim().to_string()
        };
        git(&["init", "-q"]);
        git(&["config", "user.email", "t@example.com"]);
        git(&["config", "user.name", "t"]);
        std::fs::write(dir.path().join("f.txt"), "one\n").unwrap();
        git(&["add", "."]);
        git(&["commit", "-qm", "one"]);
        let first = git(&["rev-parse", "HEAD"]);
        std::fs::write(dir.path().join("f.txt"), "two\n").unwrap();
        git(&["commit", "-qam", "two"]);
        let second = git(&["rev-parse", "HEAD"]);

        let patches = local_range_diff(dir.path(), &first, &second).expect("both shas exist");
        assert_eq!(patches.len(), 1);
        let missing = "0".repeat(40);
        assert!(local_range_diff(dir.path(), &first, &missing).is_none());
    }

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

    #[test]
    fn should_percent_encode_path_keeping_separators_and_escaping_percent() {
        assert_eq!(
            percent_encode_path("src/a file#?%.rs"),
            "src/a%20file%23%3F%25.rs"
        );
        assert_eq!(percent_encode_component("feat/x"), "feat%2Fx");
    }

    #[cfg(unix)]
    #[test]
    fn should_keep_a_backslash_in_a_unix_forge_path() {
        assert_eq!(forge_path(Path::new("a\\b/c.rs")), "a\\b/c.rs");
    }

    #[test]
    fn should_list_origin_once_in_remotes() {
        let dir = std::env::temp_dir().join(format!("tuicr-remotes-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let git = |args: &[&str]| {
            std::process::Command::new("git")
                .current_dir(&dir)
                .args(args)
                .output()
                .unwrap()
        };
        git(&["init", "-q"]);
        git(&["remote", "add", "origin", "https://github.com/a/b.git"]);
        git(&["remote", "add", "up", "https://github.com/c/d.git"]);
        let names: Vec<String> = remotes(&dir).into_iter().map(|(n, _)| n).collect();
        assert_eq!(names, ["origin", "up"]);
        std::fs::remove_dir_all(&dir).ok();
    }
}
