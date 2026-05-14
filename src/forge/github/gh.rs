use std::ffi::OsStr;

use crate::error::{Result, TuicrError};
use crate::forge::traits::{
    ForgeBackend, ForgeRepository, PagedPullRequests, PullRequestDetails, PullRequestListQuery,
    PullRequestTarget,
};
use crate::process::{CommandOutputError, CommandOutputErrorKind, run_command_output};

use super::models::{GhPullRequestDetails, GhPullRequestSummary};

const DEFAULT_GITHUB_HOST: &str = "github.com";
const PR_LIST_JSON_FIELDS: &str =
    "number,title,author,headRefName,baseRefName,updatedAt,url,state,isDraft";
const PR_VIEW_JSON_FIELDS: &str = concat!(
    "number,title,url,state,isDraft,author,headRefName,baseRefName,",
    "headRefOid,baseRefOid,body,updatedAt,closed,mergedAt"
);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GhCommandError {
    MissingGh,
    Failed { status: Option<i32>, stderr: String },
}

pub type GhCommandResult<T> = std::result::Result<T, GhCommandError>;

pub trait GhCommandRunner {
    fn run(&self, args: &[String]) -> GhCommandResult<String>;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SystemGhRunner;

impl GhCommandRunner for SystemGhRunner {
    fn run(&self, args: &[String]) -> GhCommandResult<String> {
        run_command_output("gh", None, args.iter().map(|arg| OsStr::new(arg.as_str())))
            .map_err(GhCommandError::from)
    }
}

impl From<CommandOutputError> for GhCommandError {
    fn from(error: CommandOutputError) -> Self {
        match error.kind {
            CommandOutputErrorKind::NotFound => Self::MissingGh,
            CommandOutputErrorKind::SpawnFailed | CommandOutputErrorKind::Unsuccessful => {
                Self::Failed {
                    status: error.status,
                    stderr: error.stderr,
                }
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct GitHubGhBackend<R = SystemGhRunner> {
    default_repository: Option<ForgeRepository>,
    runner: R,
}

impl GitHubGhBackend<SystemGhRunner> {
    pub fn new(default_repository: Option<ForgeRepository>) -> Self {
        Self {
            default_repository,
            runner: SystemGhRunner,
        }
    }
}

impl<R> GitHubGhBackend<R>
where
    R: GhCommandRunner,
{
    pub fn with_runner(default_repository: Option<ForgeRepository>, runner: R) -> Self {
        Self {
            default_repository,
            runner,
        }
    }

    fn resolve_repository(&self, target: &PullRequestTarget) -> Result<ForgeRepository> {
        target
            .repository
            .clone()
            .or_else(|| self.default_repository.clone())
            .ok_or_else(|| {
                TuicrError::Forge(format!(
                    "GitHub pull request target `{}` does not include a repository",
                    target.original
                ))
            })
    }

    fn run_gh(&self, args: Vec<String>, host: &str) -> Result<String> {
        self.runner
            .run(&args)
            .map_err(|err| map_gh_error(err, host))
    }
}

impl<R> ForgeBackend for GitHubGhBackend<R>
where
    R: GhCommandRunner,
{
    fn list_pull_requests(&self, query: PullRequestListQuery) -> Result<PagedPullRequests> {
        let page_size = query.page_size.max(1);
        let requested = query.already_loaded + page_size + 1;
        let output = self.run_gh(
            vec![
                "pr".to_string(),
                "list".to_string(),
                "--repo".to_string(),
                gh_repo_arg(&query.repository),
                "--state".to_string(),
                "open".to_string(),
                "--limit".to_string(),
                requested.to_string(),
                "--json".to_string(),
                PR_LIST_JSON_FIELDS.to_string(),
            ],
            &query.repository.host,
        )?;
        let rows: Vec<GhPullRequestSummary> = serde_json::from_str(&output)?;
        let has_more = rows.len() > query.already_loaded + page_size;
        let pull_requests = rows
            .into_iter()
            .skip(query.already_loaded)
            .take(page_size)
            .map(|row| row.into_summary(&query.repository))
            .collect::<Vec<_>>();
        let total_loaded = query.already_loaded + pull_requests.len();

        Ok(PagedPullRequests {
            pull_requests,
            has_more,
            total_loaded,
        })
    }

    fn get_pull_request(&self, target: PullRequestTarget) -> Result<PullRequestDetails> {
        let repository = self.resolve_repository(&target)?;
        let output = self.run_gh(
            vec![
                "pr".to_string(),
                "view".to_string(),
                target.number.to_string(),
                "--repo".to_string(),
                gh_repo_arg(&repository),
                "--json".to_string(),
                PR_VIEW_JSON_FIELDS.to_string(),
            ],
            &repository.host,
        )?;
        let pr: GhPullRequestDetails = serde_json::from_str(&output)?;
        pr.into_details(&repository)
    }

    fn get_pull_request_diff(&self, pr: &PullRequestDetails) -> Result<String> {
        self.run_gh(
            vec![
                "pr".to_string(),
                "diff".to_string(),
                pr.number.to_string(),
                "--repo".to_string(),
                gh_repo_arg(&pr.repository),
                "--patch".to_string(),
                "--color".to_string(),
                "never".to_string(),
            ],
            &pr.repository.host,
        )
    }
}

pub fn parse_pull_request_target(input: &str) -> Result<PullRequestTarget> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return malformed_target(input);
    }

    if let Some(target) = parse_numeric_target(trimmed) {
        return Ok(target);
    }
    if let Some(target) = parse_url_target(trimmed) {
        return Ok(target);
    }
    if let Some(target) = parse_repo_hash_target(trimmed) {
        return Ok(target);
    }

    malformed_target(input)
}

pub fn parse_github_remote_url(remote_url: &str) -> Option<ForgeRepository> {
    let trimmed = trim_url_suffix(remote_url.trim());
    if trimmed.is_empty() {
        return None;
    }

    if let Some((host, path)) = parse_scp_like_remote(trimmed) {
        return repository_from_path(host, path);
    }

    let without_scheme = strip_scheme(trimmed).unwrap_or(trimmed);
    let without_user = without_scheme
        .rsplit_once('@')
        .map(|(_, rest)| rest)
        .unwrap_or(without_scheme);
    let (host, path) = without_user.split_once('/')?;
    repository_from_path(host, path)
}

fn parse_numeric_target(target: &str) -> Option<PullRequestTarget> {
    if !target.chars().all(|ch| ch.is_ascii_digit()) {
        return None;
    }
    let number = target.parse::<u64>().ok()?;
    if number == 0 {
        return None;
    }
    Some(PullRequestTarget::number(number, target))
}

fn parse_url_target(target: &str) -> Option<PullRequestTarget> {
    let without_scheme = strip_scheme(target)?;
    let trimmed = trim_url_suffix(without_scheme);
    let mut parts = trimmed.split('/').filter(|part| !part.is_empty());
    let host = parts.next()?;
    let owner = parts.next()?;
    let repo = parts.next()?;
    if parts.next()? != "pull" {
        return None;
    }
    let number = parts.next()?.parse::<u64>().ok()?;
    if number == 0 {
        return None;
    }

    Some(PullRequestTarget::with_repository(
        ForgeRepository::github(host, owner, strip_git_suffix(repo)),
        number,
        target,
    ))
}

fn parse_repo_hash_target(target: &str) -> Option<PullRequestTarget> {
    let (repo_part, number_part) = target.split_once('#')?;
    let number = number_part.parse::<u64>().ok()?;
    if number == 0 {
        return None;
    }
    let parts = repo_part
        .split('/')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    let repository = match parts.as_slice() {
        [owner, repo] => {
            ForgeRepository::github(DEFAULT_GITHUB_HOST, *owner, strip_git_suffix(repo))
        }
        [host, owner, repo] => ForgeRepository::github(*host, *owner, strip_git_suffix(repo)),
        _ => return None,
    };

    Some(PullRequestTarget::with_repository(
        repository, number, target,
    ))
}

fn parse_scp_like_remote(remote_url: &str) -> Option<(&str, &str)> {
    if remote_url.contains("://") {
        return None;
    }

    let (host_part, path) = remote_url.split_once(':')?;
    if host_part.contains('/') || path.is_empty() {
        return None;
    }
    let host = host_part
        .rsplit_once('@')
        .map(|(_, host)| host)
        .unwrap_or(host_part);
    Some((host, path))
}

fn repository_from_path(host: &str, path: &str) -> Option<ForgeRepository> {
    let mut parts = path.split('/').filter(|part| !part.is_empty());
    let owner = parts.next()?;
    let repo = parts.next()?;
    Some(ForgeRepository::github(
        host,
        owner,
        strip_git_suffix(trim_url_suffix(repo)),
    ))
}

fn strip_scheme(value: &str) -> Option<&str> {
    value
        .strip_prefix("https://")
        .or_else(|| value.strip_prefix("http://"))
        .or_else(|| value.strip_prefix("ssh://"))
}

fn trim_url_suffix(value: &str) -> &str {
    value
        .split(['?', '#'])
        .next()
        .unwrap_or(value)
        .trim_end_matches('/')
}

fn strip_git_suffix(value: &str) -> &str {
    value.strip_suffix(".git").unwrap_or(value)
}

fn gh_repo_arg(repository: &ForgeRepository) -> String {
    if repository.host == DEFAULT_GITHUB_HOST {
        repository.slug()
    } else {
        format!("{}/{}", repository.host, repository.slug())
    }
}

fn map_gh_error(error: GhCommandError, host: &str) -> TuicrError {
    match error {
        GhCommandError::MissingGh => TuicrError::Forge(
            "GitHub integration requires `gh`.\nInstall GitHub CLI and run `gh auth login`."
                .to_string(),
        ),
        GhCommandError::Failed { stderr, .. } if looks_like_auth_failure(&stderr) => {
            TuicrError::Forge(format!(
                "GitHub authentication failed.\nRun `gh auth login` for {host}."
            ))
        }
        GhCommandError::Failed { stderr, status } => {
            let detail = if stderr.is_empty() {
                status
                    .map(|code| format!("gh exited with status {code}"))
                    .unwrap_or_else(|| "gh command failed".to_string())
            } else {
                stderr
            };
            TuicrError::Forge(format!("GitHub command failed: {detail}"))
        }
    }
}

fn looks_like_auth_failure(stderr: &str) -> bool {
    let lower = stderr.to_ascii_lowercase();
    lower.contains("gh auth login")
        || lower.contains("not logged in")
        || lower.contains("not logged into")
        || lower.contains("authentication failed")
        || lower.contains("requires authentication")
}

fn malformed_target<T>(input: &str) -> Result<T> {
    Err(TuicrError::Forge(format!(
        "Malformed GitHub pull request target: `{input}`"
    )))
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;
    use crate::forge::traits::ForgeBackend;

    const PR_LIST_JSON: &str = r##"
[
  {
    "number": 148,
    "title": "Add forge-backed PR review",
    "author": { "login": "alice" },
    "headRefName": "forge-review",
    "baseRefName": "main",
    "updatedAt": "2026-05-12T18:30:00Z",
    "url": "https://github.com/agavra/tuicr/pull/148",
    "state": "OPEN",
    "isDraft": false
  },
  {
    "number": 125,
    "title": "Support fetching and pushing reviews",
    "author": { "login": "YPares" },
    "headRefName": "reviews",
    "baseRefName": "main",
    "updatedAt": "2026-05-08T10:00:00Z",
    "url": "https://github.com/agavra/tuicr/pull/125",
    "state": "OPEN",
    "isDraft": true
  }
]
"##;

    const PR_VIEW_JSON: &str = r##"
{
  "number": 125,
  "title": "Support fetching and pushing reviews",
  "url": "https://github.com/agavra/tuicr/pull/125",
  "state": "OPEN",
  "isDraft": false,
  "author": { "login": "alice" },
  "headRefName": "reviews",
  "baseRefName": "main",
  "headRefOid": "abcdef1234567890",
  "baseRefOid": "1234567890abcdef",
  "body": "Review workflow",
  "updatedAt": "2026-05-12T18:30:00Z",
  "closed": false,
  "mergedAt": null
}
"##;

    const PR_PATCH: &str = r##"
diff --git a/src/lib.rs b/src/lib.rs
index 1111111..2222222 100644
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -1,3 +1,3 @@
 pub fn answer() -> u32 {
-    41
+    42
 }
"##;

    #[derive(Default)]
    struct FakeGhRunner {
        calls: RefCell<Vec<Vec<String>>>,
    }

    impl GhCommandRunner for FakeGhRunner {
        fn run(&self, args: &[String]) -> GhCommandResult<String> {
            self.calls.borrow_mut().push(args.to_vec());
            match args.get(1).map(String::as_str) {
                Some("list") => Ok(PR_LIST_JSON.to_string()),
                Some("view") => Ok(PR_VIEW_JSON.to_string()),
                Some("diff") => Ok(PR_PATCH.to_string()),
                _ => Err(GhCommandError::Failed {
                    status: Some(1),
                    stderr: "unexpected command".to_string(),
                }),
            }
        }
    }

    fn repo() -> ForgeRepository {
        ForgeRepository::github("github.com", "agavra", "tuicr")
    }

    #[test]
    fn repository_display_name_omits_github_com() {
        assert_eq!(repo().display_name(), "agavra/tuicr");
        assert_eq!(
            ForgeRepository::github("github.example.com", "agavra", "tuicr").display_name(),
            "github.example.com/agavra/tuicr"
        );
    }

    #[test]
    fn constructs_system_runner_backend() {
        let _backend = GitHubGhBackend::new(Some(repo()));
    }

    #[test]
    fn parses_numeric_pull_request_target() {
        let target = parse_pull_request_target("125").unwrap();
        assert_eq!(target.number, 125);
        assert_eq!(target.repository, None);
    }

    #[test]
    fn parses_owner_repo_pull_request_target() {
        let target = parse_pull_request_target("agavra/tuicr#125").unwrap();
        assert_eq!(target.number, 125);
        assert_eq!(target.repository.unwrap(), repo());
    }

    #[test]
    fn parses_enterprise_owner_repo_pull_request_target() {
        let target = parse_pull_request_target("github.example.com/agavra/tuicr#125").unwrap();
        let repository = target.repository.unwrap();
        assert_eq!(repository.host, "github.example.com");
        assert_eq!(repository.slug(), "agavra/tuicr");
    }

    #[test]
    fn parses_full_pull_request_url() {
        let target = parse_pull_request_target("https://github.com/agavra/tuicr/pull/125").unwrap();
        assert_eq!(target.number, 125);
        assert_eq!(target.repository.unwrap(), repo());
    }

    #[test]
    fn rejects_malformed_pull_request_target() {
        let err = parse_pull_request_target("agavra/tuicr/pulls/125").unwrap_err();
        assert!(
            err.to_string()
                .contains("Malformed GitHub pull request target")
        );
    }

    #[test]
    fn parses_https_remote_url() {
        let repository = parse_github_remote_url("https://github.com/agavra/tuicr.git").unwrap();
        assert_eq!(repository, repo());
    }

    #[test]
    fn parses_scp_like_ssh_remote_url() {
        let repository = parse_github_remote_url("git@github.com:agavra/tuicr.git").unwrap();
        assert_eq!(repository, repo());
    }

    #[test]
    fn parses_ssh_remote_url() {
        let repository =
            parse_github_remote_url("ssh://git@github.example.com/agavra/tuicr.git").unwrap();
        assert_eq!(repository.host, "github.example.com");
        assert_eq!(repository.slug(), "agavra/tuicr");
    }

    #[test]
    fn list_pull_requests_uses_gh_json_output() {
        let runner = FakeGhRunner::default();
        let backend = GitHubGhBackend::with_runner(None, runner);
        let result = backend
            .list_pull_requests(PullRequestListQuery::first_page(repo(), 1))
            .unwrap();

        assert_eq!(result.pull_requests.len(), 1);
        assert!(result.has_more);
        assert_eq!(result.total_loaded, 1);
        assert_eq!(result.pull_requests[0].number, 148);
        assert_eq!(result.pull_requests[0].author.as_deref(), Some("alice"));

        let calls = backend.runner.calls.borrow();
        assert_eq!(
            calls[0],
            vec![
                "pr",
                "list",
                "--repo",
                "agavra/tuicr",
                "--state",
                "open",
                "--limit",
                "2",
                "--json",
                PR_LIST_JSON_FIELDS,
            ]
        );
    }

    #[test]
    fn list_pull_requests_can_load_next_slice() {
        let runner = FakeGhRunner::default();
        let backend = GitHubGhBackend::with_runner(None, runner);
        let result = backend
            .list_pull_requests(PullRequestListQuery {
                repository: repo(),
                already_loaded: 1,
                page_size: 1,
            })
            .unwrap();

        assert_eq!(result.pull_requests.len(), 1);
        assert!(!result.has_more);
        assert_eq!(result.total_loaded, 2);
        assert_eq!(result.pull_requests[0].number, 125);

        let calls = backend.runner.calls.borrow();
        assert_eq!(
            calls[0],
            vec![
                "pr",
                "list",
                "--repo",
                "agavra/tuicr",
                "--state",
                "open",
                "--limit",
                "3",
                "--json",
                PR_LIST_JSON_FIELDS,
            ]
        );
    }

    #[test]
    fn list_pull_requests_uses_host_qualified_repo_for_enterprise() {
        let runner = FakeGhRunner::default();
        let backend = GitHubGhBackend::with_runner(None, runner);
        let repository = ForgeRepository::github("github.example.com", "agavra", "tuicr");
        backend
            .list_pull_requests(PullRequestListQuery::first_page(repository, 1))
            .unwrap();

        let calls = backend.runner.calls.borrow();
        assert_eq!(calls[0][3], "github.example.com/agavra/tuicr");
    }

    #[test]
    fn get_pull_request_uses_default_repository_for_numeric_target() {
        let runner = FakeGhRunner::default();
        let backend = GitHubGhBackend::with_runner(Some(repo()), runner);
        let details = backend
            .get_pull_request(parse_pull_request_target("125").unwrap())
            .unwrap();

        assert_eq!(details.number, 125);
        assert_eq!(details.head_sha, "abcdef1234567890");
        assert!(!details.is_read_only());
    }

    #[test]
    fn get_pull_request_requires_repository() {
        let runner = FakeGhRunner::default();
        let backend = GitHubGhBackend::with_runner(None, runner);
        let err = backend
            .get_pull_request(parse_pull_request_target("125").unwrap())
            .unwrap_err();
        assert!(err.to_string().contains("does not include a repository"));
    }

    #[test]
    fn get_pull_request_diff_returns_patch_text() {
        let runner = FakeGhRunner::default();
        let backend = GitHubGhBackend::with_runner(Some(repo()), runner);
        let details = backend
            .get_pull_request(parse_pull_request_target("125").unwrap())
            .unwrap();
        let patch = backend.get_pull_request_diff(&details).unwrap();

        assert_eq!(patch, PR_PATCH);
    }

    #[test]
    fn maps_missing_gh_to_install_message() {
        let err = map_gh_error(GhCommandError::MissingGh, "github.com");
        assert!(err.to_string().contains("GitHub integration requires `gh`"));
    }

    #[test]
    fn maps_auth_failure_to_login_message() {
        let err = map_gh_error(
            GhCommandError::Failed {
                status: Some(4),
                stderr: "run `gh auth login` to authenticate".to_string(),
            },
            "github.example.com",
        );
        assert!(err.to_string().contains("GitHub authentication failed"));
        assert!(err.to_string().contains("github.example.com"));
    }
}
