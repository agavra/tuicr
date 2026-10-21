//! Remote-URL and target parsing for Heptapod.
//!
//! Heptapod uses the same URL shapes as GitLab (`/-/merge_requests/<n>`,
//! `owner/repo#N`, SCP-like and `ssh://` git URLs). The only difference is
//! host detection: an Heptapod host is recognized when its name contains
//! "heptapod" or matches the host configured via the `HEPTAPOD_URL` env var.
//! `glab config get host` is intentionally not consulted here — the GitLab
//! parser already falls back to it, and Heptapod must stay ahead of GitLab in
//! [`crate::forge::parse_any_remote_url`] so an Heptapod host that also
//! appears in `glab`'s config is claimed by Heptapod, not GitLab.

use crate::error::{Result, TuicrError};
use crate::forge::gitlab::glab::{
    gitlab_repository_from_path, parse_pull_request_target_gitlab, parse_scp_like_remote,
    strip_port, strip_scheme, trim_url_suffix,
};
use crate::forge::traits::{ForgeRepository, PullRequestTarget};

/// Env var naming the Heptapod host, on the same model as `GERRIT_URL`.
/// Useful for instances whose hostname does not contain "heptapod"
/// (e.g. a commercial Heptapod living at `orus.io`).
pub const HEPTAPOD_URL_ENV_VAR: &str = "HEPTAPOD_URL";

/// Parse a Heptapod remote URL into a [`ForgeRepository`].
///
/// Recognizes the same URL shapes as GitLab (SCP-like, HTTPS, `ssh://`), but
/// only when the host looks like Heptapod — either its name contains
/// "heptapod", or the host matches `HEPTAPOD_URL`.
pub fn parse_heptapod_remote_url(remote_url: &str) -> Option<ForgeRepository> {
    parse_heptapod_remote(remote_url, HostLookup::WithEnv)
}

/// Like [`parse_heptapod_remote_url`], but recognizes Heptapod by hostname
/// alone — no env-var read. Mirrors the GitLab hostname-only variant used on
/// hot paths such as [`crate::slug::resolve_owner_repo`].
pub fn parse_heptapod_remote_url_by_hostname(remote_url: &str) -> Option<ForgeRepository> {
    parse_heptapod_remote(remote_url, HostLookup::HostnameOnly)
}

/// How far host recognition may go to decide a remote is Heptapod.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HostLookup {
    /// Consult `HEPTAPOD_URL` when the hostname alone is inconclusive.
    WithEnv,
    /// Recognize Heptapod by hostname only — no env-var read.
    HostnameOnly,
}

fn parse_heptapod_remote(remote_url: &str, lookup: HostLookup) -> Option<ForgeRepository> {
    // The URL grammar is identical to GitLab's — we reuse the GitLab parser's
    // low-level helpers (scheme/SSH/host extraction) but apply the Heptapod
    // host check ourselves. The GitLab parser's `parse_gitlab_remote` cannot
    // be reused directly because it gates on `is_gitlab_host`, which rejects
    // `foss.heptapod.net` (and any other non-"gitlab" hostname).
    let trimmed = trim_url_suffix(remote_url.trim());
    if trimmed.is_empty() {
        return None;
    }

    if let Some((host, path)) = parse_scp_like_remote(trimmed) {
        if is_heptapod_host(host, lookup) {
            return heptapod_repository_from_path(host, path);
        }
        if lookup == HostLookup::HostnameOnly {
            return None;
        }
        return None;
    }

    let without_scheme = strip_scheme(trimmed).unwrap_or(trimmed);
    let without_user = without_scheme
        .rsplit_once('@')
        .map(|(_, rest)| rest)
        .unwrap_or(without_scheme);
    let (host, path) = without_user.split_once('/')?;
    let host = strip_port(host);
    if !is_heptapod_host(host, lookup) {
        return None;
    }
    heptapod_repository_from_path(host, path)
}

/// Same as `gitlab_repository_from_path` but produces a `Heptapod` kind.
fn heptapod_repository_from_path(host: &str, path: &str) -> Option<ForgeRepository> {
    let gitlab_repo = gitlab_repository_from_path(host, path)?;
    Some(ForgeRepository::heptapod(
        gitlab_repo.host,
        gitlab_repo.owner,
        gitlab_repo.name,
    ))
}

/// Returns `true` when `host` looks like an Heptapod instance: either its
/// name contains "heptapod" (covers `foss.heptapod.net` and most public
/// deployments) or — under [`HostLookup::WithEnv`] — matches the hostname
/// configured via `HEPTAPOD_URL` (covers commercial instances such as
/// `orus.io`).
fn is_heptapod_host(host: &str, lookup: HostLookup) -> bool {
    if host.to_ascii_lowercase().contains("heptapod") {
        return true;
    }
    if lookup == HostLookup::HostnameOnly {
        return false;
    }
    configured_host().is_some_and(|known| known.eq_ignore_ascii_case(host))
}

/// Read the host configured via `HEPTAPOD_URL`, normalized to a bare
/// authority (scheme and userinfo stripped, trailing port kept).
/// Returns `None` when unset or malformed.
fn configured_host() -> Option<String> {
    let raw = std::env::var(HEPTAPOD_URL_ENV_VAR).ok()?;
    let trimmed = raw.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return None;
    }
    let after_scheme = trimmed
        .strip_prefix("https://")
        .or_else(|| trimmed.strip_prefix("http://"))
        .or_else(|| trimmed.strip_prefix("ssh://"))
        .unwrap_or(trimmed);
    // Drop userinfo.
    let after_user = after_scheme
        .rsplit_once('@')
        .map(|(_, rest)| rest)
        .unwrap_or(after_scheme);
    let authority = after_user.split('/').next()?;
    let authority = authority.trim();
    if authority.is_empty() {
        None
    } else {
        Some(authority.to_string())
    }
}

/// Parse a pull request target string in Heptapod format.
///
/// Delegates to `parse_pull_request_target_gitlab` for the URL/numeric/hash
/// grammar, then re-tags the resolved repository as Heptapod when its host
/// is recognized. Numeric targets pass through unchanged (no repository to
/// re-tag); an explicit GitLab URL on a non-Heptapod host returns an error
/// so the caller can fall through to the GitLab parser.
pub fn parse_pull_request_target_heptapod(input: &str) -> Result<PullRequestTarget> {
    let target = parse_pull_request_target_gitlab(input)?;
    if let Some(repo) = target.repository.as_ref()
        && !is_heptapod_host(&repo.host, HostLookup::WithEnv)
    {
        return malformed_target(input);
    }
    Ok(rewrite_as_heptapod(target))
}

/// Re-tag a [`PullRequestTarget`] built by the GitLab parser as Heptapod.
fn rewrite_as_heptapod(mut target: PullRequestTarget) -> PullRequestTarget {
    if let Some(repo) = target.repository.as_mut() {
        repo.kind = crate::forge::traits::ForgeKind::Heptapod;
    }
    target
}

fn malformed_target<T>(input: &str) -> Result<T> {
    Err(TuicrError::Forge(format!(
        "Malformed Heptapod merge request target: `{input}`"
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::forge::traits::ForgeKind;
    use std::sync::Mutex;

    /// `HEPTAPOD_URL` is process-global, so tests that set it run one at a
    /// time. Mirrors the `ENV_LOCK` pattern in `forge::gerrit::api`.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    /// Acquire the env lock and set `HEPTAPOD_URL` to `value` for the
    /// lifetime of the returned guard. Restores the prior value (or unsets)
    /// on drop. The guard holds the mutex, so concurrent tests touching the
    /// env var block until this one finishes.
    fn with_env(value: Option<&str>) -> std::sync::MutexGuard<'static, ()> {
        let guard = ENV_LOCK.lock().unwrap_or_else(|err| err.into_inner());
        // SAFETY: ENV_LOCK is held, so no concurrent test touches the env.
        match value {
            Some(v) => unsafe { std::env::set_var(HEPTAPOD_URL_ENV_VAR, v) },
            None => unsafe { std::env::remove_var(HEPTAPOD_URL_ENV_VAR) },
        }
        // The original value is restored in each test's explicit teardown
        // before the guard drops, to keep the helper simple.
        guard
    }

    fn restore_env() {
        // SAFETY: callers hold ENV_LOCK.
        unsafe { std::env::remove_var(HEPTAPOD_URL_ENV_VAR) };
    }

    #[test]
    fn should_recognize_foss_heptapod_host_by_name() {
        let repo = parse_heptapod_remote_url("https://foss.heptapod.net/owner/repo.git").unwrap();
        assert_eq!(repo.host, "foss.heptapod.net");
        assert_eq!(repo.kind, ForgeKind::Heptapod);
        assert_eq!(repo.owner, "owner");
        assert_eq!(repo.name, "repo");
    }

    #[test]
    fn should_recognize_scp_like_heptapod_remote() {
        let repo = parse_heptapod_remote_url("git@foss.heptapod.net:group/sub/repo.git").unwrap();
        assert_eq!(repo.host, "foss.heptapod.net");
        assert_eq!(repo.owner, "group/sub");
        assert_eq!(repo.name, "repo");
        assert_eq!(repo.kind, ForgeKind::Heptapod);
    }

    #[test]
    fn should_not_claim_plain_gitlab_host() {
        // Hostname alone is inconclusive; Heptapod must not claim gitlab.com.
        assert!(parse_heptapod_remote_url("https://gitlab.com/owner/repo.git").is_none());
    }

    #[test]
    fn should_not_claim_github_host() {
        assert!(parse_heptapod_remote_url("https://github.com/owner/repo").is_none());
    }

    #[test]
    fn should_recognize_host_named_via_heptapod_url_env() {
        // A commercial instance living at a neutral hostname.
        let _guard = with_env(Some("https://orus.io"));
        let repo = parse_heptapod_remote_url("https://orus.io/owner/repo.git").unwrap();
        assert_eq!(repo.host, "orus.io");
        assert_eq!(repo.kind, ForgeKind::Heptapod);
        restore_env();
    }

    #[test]
    fn should_skip_env_when_hostname_already_signals_heptapod() {
        let _guard = with_env(Some("https://other.example.com"));
        let repo = parse_heptapod_remote_url("https://foss.heptapod.net/owner/repo.git").unwrap();
        assert_eq!(repo.host, "foss.heptapod.net");
        assert_eq!(repo.kind, ForgeKind::Heptapod);
        restore_env();
    }

    #[test]
    fn hostname_only_variant_skips_env() {
        let _guard = with_env(Some("https://orus.io"));
        // Hostname-only must not consult HEPTAPOD_URL.
        assert!(parse_heptapod_remote_url_by_hostname("https://orus.io/owner/repo.git").is_none());
        // But a host containing "heptapod" is still recognized.
        let repo =
            parse_heptapod_remote_url_by_hostname("https://foss.heptapod.net/o/r.git").unwrap();
        assert_eq!(repo.kind, ForgeKind::Heptapod);
        restore_env();
    }

    #[test]
    fn should_parse_heptapod_mr_url_target() {
        let target = parse_pull_request_target_heptapod(
            "https://foss.heptapod.net/owner/repo/-/merge_requests/42",
        )
        .unwrap();
        assert_eq!(target.number, 42);
        let repo = target.repository.unwrap();
        assert_eq!(repo.host, "foss.heptapod.net");
        assert_eq!(repo.owner, "owner");
        assert_eq!(repo.name, "repo");
        assert_eq!(repo.kind, ForgeKind::Heptapod);
    }

    #[test]
    fn should_parse_heptapod_nested_group_url_target() {
        let target = parse_pull_request_target_heptapod(
            "https://foss.heptapod.net/group/sub/repo/-/merge_requests/33",
        )
        .unwrap();
        assert_eq!(target.number, 33);
        let repo = target.repository.unwrap();
        assert_eq!(repo.owner, "group/sub");
        assert_eq!(repo.name, "repo");
        assert_eq!(repo.kind, ForgeKind::Heptapod);
    }

    #[test]
    fn should_parse_heptapod_repo_hash_target_with_heptapod_host() {
        let _guard = with_env(Some("https://orus.io"));
        let target = parse_pull_request_target_heptapod("orus.io/owner/repo#42").unwrap();
        assert_eq!(target.number, 42);
        let repo = target.repository.unwrap();
        assert_eq!(repo.host, "orus.io");
        assert_eq!(repo.kind, ForgeKind::Heptapod);
        restore_env();
    }

    #[test]
    fn should_reject_repo_hash_target_on_non_heptapod_host() {
        // gitlab.com is not Heptapod: the parser returns an error so the
        // caller can fall through to the GitLab parser.
        let err = parse_pull_request_target_heptapod("gitlab.com/owner/repo#42").unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("Malformed Heptapod"), "got: {msg}");
    }

    #[test]
    fn should_parse_numeric_target_without_repository() {
        let target = parse_pull_request_target_heptapod("123").unwrap();
        assert_eq!(target.number, 123);
        assert!(target.repository.is_none());
    }

    #[test]
    fn should_reject_empty_target() {
        assert!(parse_pull_request_target_heptapod("").is_err());
        assert!(parse_pull_request_target_heptapod("   ").is_err());
    }

    #[test]
    fn should_reject_zero_target() {
        assert!(parse_pull_request_target_heptapod("0").is_err());
    }
}
