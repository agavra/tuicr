use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use crate::error::{Result, TuicrError};
use crate::forge::remote_comments::RemoteReviewThread;
use crate::forge::traits::{
    ForgeBackend, ForgeFileLinesRequest, ForgeRepository, PagedPullRequests, PullRequestDetails,
    PullRequestListQuery, PullRequestTarget,
};
use crate::model::{DiffLine, LineOrigin};
use crate::process::{CommandOutputError, CommandOutputErrorKind, run_command_output};

use super::models::{GhPullRequestDetails, GhPullRequestSummary};
use super::review_threads::{build_query, parse_graphql_page};

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

/// Read a git blob from a checkout at `repo_root` using `git show <sha>:<path>`.
/// Returns `None` if the object is missing or the command fails for any reason.
fn read_blob_with_repo(repo_root: &Path, sha: &str, path: &Path) -> Option<String> {
    let spec = format!("{}:{}", sha, path.to_string_lossy());
    let exists = run_command_output(
        "git",
        Some(repo_root),
        ["cat-file", "-e", spec.as_str()]
            .iter()
            .map(|s| OsStr::new(*s)),
    );
    if exists.is_err() {
        return None;
    }
    run_command_output(
        "git",
        Some(repo_root),
        ["show", spec.as_str()].iter().map(|s| OsStr::new(*s)),
    )
    .ok()
}

#[derive(Debug, Clone)]
pub struct GitHubGhBackend<R = SystemGhRunner> {
    default_repository: Option<ForgeRepository>,
    runner: R,
    /// Optional path to a local checkout. When present, the backend may
    /// satisfy `fetch_file_lines` from local git objects before falling back
    /// to `gh api`. It is **never** used as the source of truth for PR
    /// contents; the source of truth is always GitHub.
    local_checkout: Option<PathBuf>,
}

impl GitHubGhBackend<SystemGhRunner> {
    pub fn new(default_repository: Option<ForgeRepository>) -> Self {
        Self {
            default_repository,
            runner: SystemGhRunner,
            local_checkout: None,
        }
    }

    pub fn with_local_checkout(mut self, checkout: Option<PathBuf>) -> Self {
        self.local_checkout = checkout;
        self
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
            local_checkout: None,
        }
    }

    pub fn set_local_checkout(&mut self, checkout: Option<PathBuf>) {
        self.local_checkout = checkout;
    }

    pub fn local_checkout(&self) -> Option<&Path> {
        self.local_checkout.as_deref()
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
        // We want the *cumulative* diff between base and head for the PR.
        // `gh pr diff --patch` returns mbox-style `git format-patch` output
        // — one patch per commit — so a 7-commit PR yields 7 separate
        // `diff --git` blocks per file, which our parser dutifully turns
        // into duplicate `DiffFile`s. Plain `gh pr diff` (no `--patch`)
        // returns the single cumulative diff. Hard-won lesson; see the
        // duplicate-files-in-list bug.
        self.run_gh(
            vec![
                "pr".to_string(),
                "diff".to_string(),
                pr.number.to_string(),
                "--repo".to_string(),
                gh_repo_arg(&pr.repository),
                "--color".to_string(),
                "never".to_string(),
            ],
            &pr.repository.host,
        )
    }

    fn local_checkout_path(&self) -> Option<PathBuf> {
        self.local_checkout.clone()
    }

    fn list_review_threads(&self, pr: &PullRequestDetails) -> Result<Vec<RemoteReviewThread>> {
        let mut all: Vec<RemoteReviewThread> = Vec::new();
        let mut cursor: Option<String> = None;
        // Bound the pagination loop so a buggy/cyclic server can't hang us.
        // 100 threads * 100 pages = 10k threads; well beyond realistic PRs.
        for _ in 0..100 {
            let args = self.build_review_threads_args(pr, cursor.as_deref());
            let output = self.run_gh(args, &pr.repository.host)?;
            let parsed = parse_graphql_page(&output)?;
            all.extend(parsed.threads);
            let Some(page_info) = parsed.page_info else {
                break;
            };
            if !page_info.has_next_page {
                break;
            }
            let Some(end_cursor) = page_info.end_cursor else {
                break;
            };
            cursor = Some(end_cursor);
        }
        Ok(all)
    }

    fn fetch_file_lines(&self, request: ForgeFileLinesRequest) -> Result<Vec<DiffLine>> {
        if request.start_line == 0 || request.start_line > request.end_line {
            return Ok(Vec::new());
        }

        // Local optimization: read the blob from a configured checkout when
        // we have it. The PR's exact SHAs may or may not be present locally;
        // we silently fall back if they aren't.
        let local_content = self
            .local_checkout
            .as_deref()
            .and_then(|root| read_blob_with_repo(root, request.sha(), request.path.as_path()));

        let content = if let Some(content) = local_content {
            content
        } else {
            self.fetch_file_via_api(&request)?
        };

        Ok(slice_to_diff_lines(
            &content,
            request.start_line,
            request.end_line,
        ))
    }
}

impl<R> GitHubGhBackend<R>
where
    R: GhCommandRunner,
{
    fn build_review_threads_args(
        &self,
        pr: &PullRequestDetails,
        cursor: Option<&str>,
    ) -> Vec<String> {
        let query = build_query(cursor);
        let mut args = vec![
            "api".to_string(),
            "graphql".to_string(),
            "-f".to_string(),
            format!("query={query}"),
            "-F".to_string(),
            format!("owner={}", pr.repository.owner),
            "-F".to_string(),
            format!("name={}", pr.repository.name),
            "-F".to_string(),
            format!("number={}", pr.number),
        ];
        if let Some(c) = cursor {
            args.push("-F".to_string());
            args.push(format!("after={c}"));
        }
        // Enterprise host routing — `gh api graphql --hostname` picks the
        // correct endpoint when the repo lives on a non-default host.
        if pr.repository.host != "github.com" {
            args.push("--hostname".to_string());
            args.push(pr.repository.host.clone());
        }
        args
    }

    fn fetch_file_via_api(&self, request: &ForgeFileLinesRequest) -> Result<String> {
        // `gh api repos/<owner>/<repo>/contents/<path>?ref=<sha>` returns a
        // JSON object with base64-encoded `content` for text files. The
        // `Accept: application/vnd.github.raw` header skips JSON wrapping
        // and returns raw bytes, which we use here to keep parsing simple
        // and binary-safe (callers already gate binary files out).
        let path_str = request.path.to_string_lossy().replace('\\', "/");
        let endpoint = format!(
            "repos/{}/{}/contents/{}?ref={}",
            request.repository.owner,
            request.repository.name,
            path_str,
            request.sha(),
        );
        let mut args = vec![
            "api".to_string(),
            "-H".to_string(),
            "Accept: application/vnd.github.raw".to_string(),
        ];
        if request.repository.host != "github.com" {
            args.push("--hostname".to_string());
            args.push(request.repository.host.clone());
        }
        args.push(endpoint);
        self.run_gh(args, &request.repository.host)
    }
}

fn slice_to_diff_lines(content: &str, start_line: u32, end_line: u32) -> Vec<DiffLine> {
    let lines: Vec<&str> = content.lines().collect();
    let mut result = Vec::new();
    for line_num in start_line..=end_line {
        let idx = (line_num - 1) as usize;
        if idx < lines.len() {
            result.push(DiffLine {
                origin: LineOrigin::Context,
                content: lines[idx].to_string(),
                old_lineno: Some(line_num),
                new_lineno: Some(line_num),
                highlighted_spans: None,
            });
        }
    }
    result
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
pub(crate) mod tests_fixture {
    pub const SIMPLE_PATCH: &str = r##"diff --git a/src/lib.rs b/src/lib.rs
index 1111111..2222222 100644
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -1,3 +1,3 @@
 pub fn answer() -> u32 {
-    41
+    42
 }
"##;
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
            match args.first().map(String::as_str) {
                // gh pr list/view/diff — second arg is the subcommand.
                Some("pr") => match args.get(1).map(String::as_str) {
                    Some("list") => Ok(PR_LIST_JSON.to_string()),
                    Some("view") => Ok(PR_VIEW_JSON.to_string()),
                    Some("diff") => Ok(PR_PATCH.to_string()),
                    _ => Err(GhCommandError::Failed {
                        status: Some(1),
                        stderr: "unexpected pr command".to_string(),
                    }),
                },
                // gh api graphql for review threads.
                Some("api") if args.get(1).map(String::as_str) == Some("graphql") => {
                    Ok(REVIEW_THREADS_JSON.to_string())
                }
                _ => Err(GhCommandError::Failed {
                    status: Some(1),
                    stderr: "unexpected command".to_string(),
                }),
            }
        }
    }

    const REVIEW_THREADS_JSON: &str = r##"{
        "data": {
            "repository": {
                "pullRequest": {
                    "reviewThreads": {
                        "pageInfo": { "hasNextPage": false, "endCursor": null },
                        "nodes": [
                            {
                                "id": "PRRT_1",
                                "isResolved": false,
                                "isOutdated": false,
                                "path": "src/lib.rs",
                                "line": 42,
                                "diffSide": "RIGHT",
                                "comments": {
                                    "nodes": [
                                        {
                                            "id": "PRRC_1",
                                            "body": "remote one",
                                            "author": { "login": "alice" },
                                            "url": "https://example.com/1"
                                        }
                                    ]
                                }
                            }
                        ]
                    }
                }
            }
        }
    }"##;

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
    fn should_not_pass_patch_flag_to_gh_pr_diff() {
        // Regression: `gh pr diff --patch` returns mbox-style per-commit
        // patches concatenated, which the diff parser turns into duplicate
        // DiffFile entries (one per commit-touching-the-same-file). We
        // want the cumulative diff. Lock the argv so this can't silently
        // regress.
        let runner = FakeGhRunner::default();
        let backend = GitHubGhBackend::with_runner(Some(repo()), runner);
        let details = backend
            .get_pull_request(parse_pull_request_target("125").unwrap())
            .unwrap();
        let _ = backend.get_pull_request_diff(&details).unwrap();

        let calls = backend.runner.calls.borrow();
        let diff_call = calls
            .iter()
            .find(|args| {
                args.first().map(String::as_str) == Some("pr")
                    && args.get(1).map(String::as_str) == Some("diff")
            })
            .expect("expected a `gh pr diff` call");
        assert!(
            !diff_call.iter().any(|a| a == "--patch"),
            "`gh pr diff` must NOT pass --patch (mbox output duplicates files); got {diff_call:?}"
        );
    }

    #[test]
    fn should_list_review_threads_via_graphql_api_call() {
        // given
        let runner = FakeGhRunner::default();
        let backend = GitHubGhBackend::with_runner(Some(repo()), runner);
        let details = backend
            .get_pull_request(parse_pull_request_target("125").unwrap())
            .unwrap();
        // when
        let threads = backend.list_review_threads(&details).unwrap();
        // then
        assert_eq!(threads.len(), 1);
        assert_eq!(threads[0].id, "PRRT_1");
        assert_eq!(threads[0].path, "src/lib.rs");
        // and — the API call was placed against `gh api graphql` with the
        // owner/name/number parameters.
        let calls = backend.runner.calls.borrow();
        let graphql_call = calls
            .iter()
            .find(|args| args.first().map(String::as_str) == Some("api"))
            .expect("expected a gh api call");
        assert_eq!(graphql_call[1], "graphql");
        assert!(graphql_call.iter().any(|a| a == "owner=agavra"));
        assert!(graphql_call.iter().any(|a| a == "name=tuicr"));
        assert!(graphql_call.iter().any(|a| a == "number=125"));
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
