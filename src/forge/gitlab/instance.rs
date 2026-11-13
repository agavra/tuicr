//! Instance-level facts about the GitLab `glab` points at, as opposed to a
//! single repository.
//!
//! Today that is one fact: the path prefix of a GitLab installed under a
//! relative URL (`https://example.com/gitlab/`). `glab` prepends its
//! configured subfolder to every API URL it builds, so a project id must not
//! repeat it — see gotcha G10 in `docs/FORGES.md`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use super::glab::{GlabCommandRunner, strip_port};

/// `glab`'s per-host config key naming a relative-URL install's path prefix.
const SUBFOLDER_KEY: &str = "subfolder";

/// `glab` resolves this itself, ahead of its config file. tuicr reads it too so
/// the common case costs no process spawn. Like `glab`, it applies to every
/// host rather than being scoped to one.
pub(super) const SUBFOLDER_ENV_VAR: &str = "GITLAB_SUBFOLDER";

/// Normalize a subfolder value as `glab` does: surrounding slashes are
/// meaningless, so `gitlab`, `/gitlab` and `gitlab/` are the same prefix.
/// Nested paths (`apps/gitlab`) are kept whole. `None` means "root install".
pub(super) fn normalize_subfolder(raw: &str) -> Option<String> {
    let trimmed = raw.trim().trim_matches('"').trim_matches('\'');
    let path = trimmed.trim_matches('/');
    (!path.is_empty()).then(|| path.to_string())
}

/// The hostname out of a `glab config get host` value. Users write it either
/// bare (`git.example.com`) or as a URL (`https://git.example.com:8443/gitlab/`),
/// and the host has to be compared on its own: the whole-string comparison
/// this replaced never matched the URL form, so a self-hosted instance on a
/// domain without "gitlab" in it was not recognized as GitLab at all.
///
/// Any path the value carries is dropped rather than returned: it is *not* a
/// subfolder source, because `glab` prefixes its API URLs from the `subfolder`
/// key alone.
pub(super) fn parse_configured_hostname(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    let without_scheme = trimmed
        .strip_prefix("https://")
        .or_else(|| trimmed.strip_prefix("http://"))
        .or_else(|| trimmed.strip_prefix("ssh://"))
        .unwrap_or(trimmed);
    let without_user = without_scheme
        .rsplit_once('@')
        .map(|(_, rest)| rest)
        .unwrap_or(without_scheme);
    let authority = without_user
        .split_once('/')
        .map(|(authority, _path)| authority)
        .unwrap_or(without_user);
    let host = strip_port(authority);
    (!host.is_empty()).then(|| host.to_ascii_lowercase())
}

/// Remove an install subfolder from the front of a project path, so
/// `gitlab/group/project` becomes `group/project` on an instance served under
/// `/gitlab`. `None` means "leave the path alone".
///
/// The match is segment-aligned and happens exactly once:
///
/// - Segment-aligned, because a textual prefix match would turn subfolder
///   `git` plus project `gitlab-org/gitlab` into `lab-org/gitlab`.
/// - Only once, because an instance under `/gitlab` may well host a group
///   literally named `gitlab`; for `gitlab/gitlab/proj` one pass is right.
/// - Never down to a single segment, because a GitLab project always has a
///   namespace. `gitlab/proj` under `/gitlab` is therefore a group called
///   `gitlab`, not the subfolder.
///
/// SSH remotes carry no subfolder (`glab`'s `subfolder` covers HTTP(S) only),
/// so `group/proj` is correctly a no-op. The one false positive left is an SSH
/// remote at least three segments deep whose top-level group is named like the
/// subfolder.
pub(super) fn strip_subfolder_prefix(project_path: &str, subfolder: &str) -> Option<String> {
    let path: Vec<&str> = project_path.split('/').filter(|p| !p.is_empty()).collect();
    let prefix: Vec<&str> = subfolder.split('/').filter(|p| !p.is_empty()).collect();
    if prefix.is_empty() || path.len() < prefix.len() + 2 {
        return None;
    }
    if path[..prefix.len()] != prefix[..] {
        return None;
    }
    Some(path[prefix.len()..].join("/"))
}

/// Per-host install subfolder, memoized for the life of the backend.
///
/// Project ids are built inside pagination loops, so the lookup has to be
/// cached — including its negative result. A `Mutex` rather than a `RefCell`
/// because `fetch_pr_data` fans the backend out across scoped threads (forge
/// gotcha F7 in docs/FORGES.md); the lock is held across the lookup so concurrent callers for
/// one host spawn `glab` at most once between them.
#[derive(Debug, Clone, Default)]
pub(super) struct SubfolderCache(Arc<Mutex<HashMap<String, Option<String>>>>);

impl SubfolderCache {
    pub(super) fn resolve<R: GlabCommandRunner>(&self, host: &str, runner: &R) -> Option<String> {
        let key = host.to_ascii_lowercase();
        let mut cache = self.0.lock().unwrap_or_else(|err| err.into_inner());
        if let Some(cached) = cache.get(&key) {
            return cached.clone();
        }
        let resolved = lookup_subfolder(&key, runner);
        cache.insert(key, resolved.clone());
        resolved
    }

    /// Seed a host's subfolder without consulting `glab`. Test-only seam for
    /// the pure-helper and endpoint tests.
    #[cfg(test)]
    pub(super) fn seeded(host: &str, subfolder: Option<&str>) -> Self {
        let cache = Self::default();
        cache.0.lock().unwrap().insert(
            host.to_ascii_lowercase(),
            subfolder.and_then(normalize_subfolder),
        );
        cache
    }
}

/// `GITLAB_SUBFOLDER` first — that is `glab`'s own precedence, and it saves a
/// spawn. Otherwise ask `glab`, which resolves its per-host config file. An
/// unset key prints nothing and still exits 0, so an empty read means "root
/// install", same as a failed call.
fn lookup_subfolder<R: GlabCommandRunner>(host: &str, runner: &R) -> Option<String> {
    if let Ok(from_env) = std::env::var(SUBFOLDER_ENV_VAR)
        && let Some(subfolder) = normalize_subfolder(&from_env)
    {
        return Some(subfolder);
    }
    let args = vec![
        "config".to_string(),
        "get".to_string(),
        SUBFOLDER_KEY.to_string(),
        "--host".to_string(),
        host.to_string(),
    ];
    let output = runner.run(&args).ok()?;
    normalize_subfolder(&output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::forge::gitlab::glab::GlabCommandResult;

    /// `GITLAB_SUBFOLDER` is process-wide, so the tests that set it have to
    /// serialize against each other (same pattern as `GERRIT_URL`'s tests).
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[derive(Default)]
    struct CountingRunner {
        calls: Mutex<Vec<Vec<String>>>,
        response: String,
    }

    impl CountingRunner {
        fn new(response: &str) -> Self {
            Self {
                calls: Mutex::new(Vec::new()),
                response: response.to_string(),
            }
        }

        fn calls(&self) -> Vec<Vec<String>> {
            self.calls.lock().unwrap().clone()
        }
    }

    impl GlabCommandRunner for CountingRunner {
        fn run(&self, args: &[String]) -> GlabCommandResult<String> {
            self.calls.lock().unwrap().push(args.to_vec());
            Ok(self.response.clone())
        }
    }

    #[test]
    fn normalize_subfolder_trims_slashes_and_rejects_empty() {
        assert_eq!(normalize_subfolder("gitlab").as_deref(), Some("gitlab"));
        assert_eq!(normalize_subfolder("/gitlab/").as_deref(), Some("gitlab"));
        assert_eq!(
            normalize_subfolder(" /apps/gitlab/\n").as_deref(),
            Some("apps/gitlab")
        );
        assert_eq!(normalize_subfolder("\"gitlab\"").as_deref(), Some("gitlab"));
        assert_eq!(normalize_subfolder(""), None);
        assert_eq!(normalize_subfolder("/"), None);
        assert_eq!(normalize_subfolder("   "), None);
    }

    #[test]
    fn parse_configured_hostname_accepts_bare_host_and_url_forms() {
        assert_eq!(
            parse_configured_hostname("git.example.com").as_deref(),
            Some("git.example.com")
        );
        // The path is dropped: `glab` prefixes from the `subfolder` key alone.
        assert_eq!(
            parse_configured_hostname("https://git.example.com/gitlab").as_deref(),
            Some("git.example.com")
        );
        assert_eq!(
            parse_configured_hostname("http://GIT.example.com:8443/gitlab/").as_deref(),
            Some("git.example.com")
        );
        assert_eq!(
            parse_configured_hostname("https://user@git.example.com").as_deref(),
            Some("git.example.com")
        );
        assert_eq!(parse_configured_hostname("  "), None);
        assert_eq!(parse_configured_hostname("https:///gitlab"), None);
    }

    #[test]
    fn strip_subfolder_prefix_removes_a_leading_install_path() {
        assert_eq!(
            strip_subfolder_prefix("gitlab/group/project", "gitlab").as_deref(),
            Some("group/project")
        );
        assert_eq!(
            strip_subfolder_prefix("gitlab/group/team/project", "/gitlab/").as_deref(),
            Some("group/team/project")
        );
    }

    #[test]
    fn strip_subfolder_prefix_handles_a_nested_subfolder() {
        assert_eq!(
            strip_subfolder_prefix("apps/gitlab/group/project", "apps/gitlab").as_deref(),
            Some("group/project")
        );
        // Only the first segment matches, so the prefix is not this install's.
        assert_eq!(
            strip_subfolder_prefix("apps/group/project", "apps/gitlab"),
            None
        );
    }

    #[test]
    fn strip_subfolder_prefix_requires_segment_alignment() {
        // A textual prefix match would yield `lab-org/gitlab`.
        assert_eq!(strip_subfolder_prefix("gitlab-org/gitlab", "git"), None);
    }

    #[test]
    fn strip_subfolder_prefix_strips_only_once_for_a_group_named_like_the_subfolder() {
        assert_eq!(
            strip_subfolder_prefix("gitlab/gitlab/project", "gitlab").as_deref(),
            Some("gitlab/project")
        );
    }

    #[test]
    fn strip_subfolder_prefix_keeps_a_path_that_would_drop_below_two_segments() {
        // A project always has a namespace, so this `gitlab` is a group.
        assert_eq!(strip_subfolder_prefix("gitlab/project", "gitlab"), None);
    }

    #[test]
    fn strip_subfolder_prefix_is_a_noop_for_ssh_remote_paths() {
        // SSH remotes never carry the subfolder.
        assert_eq!(strip_subfolder_prefix("group/project", "gitlab"), None);
    }

    #[test]
    fn subfolder_cache_asks_glab_once_per_host() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|err| err.into_inner());
        unsafe { std::env::remove_var(SUBFOLDER_ENV_VAR) };
        let runner = CountingRunner::new("/gitlab\n");
        let cache = SubfolderCache::default();

        assert_eq!(
            cache.resolve("git.example.com", &runner).as_deref(),
            Some("gitlab")
        );
        assert_eq!(
            cache.resolve("git.example.com", &runner).as_deref(),
            Some("gitlab")
        );

        assert_eq!(
            runner.calls(),
            vec![vec![
                "config".to_string(),
                "get".to_string(),
                "subfolder".to_string(),
                "--host".to_string(),
                "git.example.com".to_string(),
            ]],
            "expected one lookup for two resolves"
        );
    }

    #[test]
    fn subfolder_cache_remembers_a_root_install() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|err| err.into_inner());
        unsafe { std::env::remove_var(SUBFOLDER_ENV_VAR) };
        // An unset key prints nothing.
        let runner = CountingRunner::new("");
        let cache = SubfolderCache::default();

        assert_eq!(cache.resolve("git.example.com", &runner), None);
        assert_eq!(cache.resolve("git.example.com", &runner), None);

        assert_eq!(runner.calls().len(), 1, "expected the `None` to be cached");
    }

    #[test]
    fn subfolder_from_env_skips_the_glab_lookup() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|err| err.into_inner());
        unsafe { std::env::set_var(SUBFOLDER_ENV_VAR, "/apps/gitlab/") };
        let runner = CountingRunner::new("");
        let cache = SubfolderCache::default();

        let resolved = cache.resolve("git.example.com", &runner);
        unsafe { std::env::remove_var(SUBFOLDER_ENV_VAR) };

        assert_eq!(resolved.as_deref(), Some("apps/gitlab"));
        assert!(
            runner.calls().is_empty(),
            "env var should win without spawning glab, got {:?}",
            runner.calls()
        );
    }
}
