use std::ffi::OsStr;
use std::path::Path;

use tempfile::TempDir;

use crate::error::{Result, TuicrError};
use crate::forge::traits::PullRequestDetails;
use crate::process::{CommandOutputError, run_command_output};

use super::gh::{GhCommandRunner, gh_repo_arg, map_gh_error};

/// Build an uncapped PR diff without changing the user's checkout.
///
/// A checkout already on the exact PR head can provide the diff directly. All
/// other cases use a temporary bare clone, borrowing matching local objects
/// when available before fetching the forge refs.
pub(super) fn fetch_large_diff<R>(
    runner: &R,
    pr: &PullRequestDetails,
    local_checkout: Option<&Path>,
) -> Result<String>
where
    R: GhCommandRunner,
{
    if let Some(diff) = local_checkout.and_then(|root| diff_from_current_pr_branch(root, pr)) {
        return Ok(diff);
    }

    let reference_repo = local_checkout.and_then(local_common_git_dir);
    let repo = TemporaryClone::new()?;
    repo.clone_from_github(runner, pr, reference_repo.as_deref())?;
    repo.fetch_pr_refs(pr)?;
    repo.diff(pr)
}

fn diff_from_current_pr_branch(root: &Path, pr: &PullRequestDetails) -> Option<String> {
    let top_level = local_git(
        root,
        ["--no-replace-objects", "rev-parse", "--show-toplevel"],
    )?;
    if std::fs::canonicalize(top_level.trim()).ok()? != std::fs::canonicalize(root).ok()? {
        return None;
    }

    let branch = local_git(
        root,
        [
            "--no-replace-objects",
            "symbolic-ref",
            "--quiet",
            "--short",
            "HEAD",
        ],
    )?;
    if branch.trim() != pr.head_ref_name {
        return None;
    }

    let head = local_git(
        root,
        [
            "--no-replace-objects",
            "rev-parse",
            "--verify",
            "HEAD^{commit}",
        ],
    )?;
    if head.trim() != pr.head_sha {
        return None;
    }

    for sha in [&pr.base_sha, &pr.head_sha] {
        let commit = format!("{sha}^{{commit}}");
        local_git(
            root,
            ["--no-replace-objects", "cat-file", "-e", commit.as_str()],
        )?;
    }

    let merge_base = local_git(
        root,
        [
            "--no-replace-objects",
            "merge-base",
            &pr.base_sha,
            &pr.head_sha,
        ],
    )?;
    local_git(
        root,
        [
            "--no-replace-objects",
            "diff",
            "--no-ext-diff",
            "--no-textconv",
            "--find-renames",
            merge_base.trim(),
            &pr.head_sha,
        ],
    )
}

fn local_common_git_dir(root: &Path) -> Option<std::path::PathBuf> {
    let path = local_git(
        root,
        [
            "--no-replace-objects",
            "rev-parse",
            "--path-format=absolute",
            "--git-common-dir",
        ],
    )?;
    Some(path.trim().into())
}

fn local_git<I, S>(root: &Path, args: I) -> Option<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    run_command_output("git", Some(root), args).ok()
}

struct TemporaryClone {
    dir: TempDir,
}

impl TemporaryClone {
    fn new() -> Result<Self> {
        let dir = tempfile::Builder::new().prefix("tuicr-pr-").tempdir()?;
        Ok(Self { dir })
    }

    fn path(&self) -> &std::path::Path {
        self.dir.path()
    }

    fn clone_from_github<R>(
        &self,
        runner: &R,
        pr: &PullRequestDetails,
        reference_repo: Option<&Path>,
    ) -> Result<()>
    where
        R: GhCommandRunner,
    {
        let mut args = vec![
            "repo".to_string(),
            "clone".to_string(),
            gh_repo_arg(&pr.repository),
            self.path().to_string_lossy().into_owned(),
            "--".to_string(),
            "--bare".to_string(),
            "--filter=blob:none".to_string(),
            "--no-tags".to_string(),
            "--single-branch".to_string(),
            format!("--branch={}", pr.base_ref_name),
        ];
        if let Some(repository) = reference_repo {
            args.push(format!(
                "--reference-if-able={}",
                repository.to_string_lossy()
            ));
        }
        runner.run(&args).map_err(|error| {
            TuicrError::Forge(format!(
                "Could not clone pull request repository: {}",
                map_gh_error(error, &pr.repository.host)
            ))
        })?;
        Ok(())
    }

    /// GitHub exposes fork PR heads through `refs/pull`, so fetching that ref
    /// avoids trusting a similarly named branch in either repository.
    fn fetch_pr_refs(&self, pr: &PullRequestDetails) -> Result<()> {
        let base_ref = format!("+refs/heads/{}:refs/tuicr/base", pr.base_ref_name);
        let head_ref = format!("+refs/pull/{}/head:refs/tuicr/head", pr.number);
        self.git([
            "fetch",
            "--quiet",
            "--no-tags",
            "origin",
            &base_ref,
            &head_ref,
        ])?;
        Ok(())
    }

    /// GitHub shows merge-base-to-head changes. Disabling external diff and
    /// textconv keeps local Git configuration from changing output or running
    /// user-configured programs while processing an untrusted PR.
    fn diff(&self, pr: &PullRequestDetails) -> Result<String> {
        for sha in [&pr.base_sha, &pr.head_sha] {
            let commit = format!("{sha}^{{commit}}");
            self.git(["cat-file", "-e", &commit])?;
        }
        let merge_base = self
            .git(["merge-base", &pr.base_sha, &pr.head_sha])?
            .trim()
            .to_string();
        self.git([
            "diff",
            "--no-ext-diff",
            "--no-textconv",
            "--find-renames",
            &merge_base,
            &pr.head_sha,
        ])
    }

    fn git<I, S>(&self, args: I) -> Result<String>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        run_command_output("git", Some(self.path()), args).map_err(map_git_error)
    }
}

fn map_git_error(error: CommandOutputError) -> TuicrError {
    let detail = if error.stderr.is_empty() {
        error
            .status
            .map(|status| format!("git exited with status {status}"))
            .unwrap_or_else(|| "git command failed".to_string())
    } else {
        error.stderr
    };
    TuicrError::Forge(format!(
        "Could not build pull request diff from temporary clone: {detail}"
    ))
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::rc::Rc;

    use super::*;
    use crate::forge::github::gh::{GhCommandError, GhCommandResult, GitHubGhBackend};
    use crate::forge::pr_open::open_pull_request;
    use crate::forge::traits::{ForgeBackend, ForgeRepository, PullRequestTarget};
    use crate::syntax::SyntaxHighlighter;

    struct LargeDiffFixture {
        repo: tempfile::TempDir,
        base_sha: String,
        head_sha: String,
    }

    impl LargeDiffFixture {
        fn new(file_count: usize) -> Self {
            let repo = tempfile::tempdir().unwrap();
            run_git(repo.path(), &["init", "--quiet"]);
            run_git(repo.path(), &["config", "user.name", "Test User"]);
            run_git(repo.path(), &["config", "user.email", "test@example.com"]);
            fs::write(repo.path().join("README.md"), "base\n").unwrap();
            run_git(repo.path(), &["add", "."]);
            run_git(repo.path(), &["commit", "--quiet", "-m", "base"]);
            run_git(repo.path(), &["branch", "-M", "main"]);
            let base_sha = run_git(repo.path(), &["rev-parse", "HEAD"]);
            run_git(repo.path(), &["checkout", "--quiet", "-b", "large-pr"]);

            fs::create_dir(repo.path().join("src")).unwrap();
            for index in 0..file_count {
                fs::write(
                    repo.path().join(format!("src/file-{index:03}.rs")),
                    format!("pub const VALUE: usize = {index};\n"),
                )
                .unwrap();
            }
            run_git(repo.path(), &["add", "."]);
            run_git(repo.path(), &["commit", "--quiet", "-m", "large PR"]);
            let head_sha = run_git(repo.path(), &["rev-parse", "HEAD"]);
            run_git(
                repo.path(),
                &["update-ref", "refs/pull/125/head", &head_sha],
            );
            run_git(repo.path(), &["checkout", "--quiet", "main"]);

            Self {
                repo,
                base_sha,
                head_sha,
            }
        }

        fn advance_base(&mut self) {
            fs::write(self.repo.path().join("base-only.txt"), "base advanced\n").unwrap();
            run_git(self.repo.path(), &["add", "."]);
            run_git(
                self.repo.path(),
                &["commit", "--quiet", "-m", "advance base"],
            );
            self.base_sha = run_git(self.repo.path(), &["rev-parse", "HEAD"]);
        }

        fn checkout_pr_head(&self) {
            run_git(self.repo.path(), &["checkout", "--quiet", "large-pr"]);
        }

        fn advance_pr_head_locally(&self) {
            fs::write(
                self.repo.path().join("local-only.txt"),
                "new local commit\n",
            )
            .unwrap();
            run_git(self.repo.path(), &["add", "."]);
            run_git(
                self.repo.path(),
                &["commit", "--quiet", "-m", "advance local branch"],
            );
        }

        fn linked_pr_worktree(&self) -> LinkedWorktree {
            LinkedWorktree::new(self.repo.path(), "large-pr")
        }

        fn shallow_pr_clone(&self) -> tempfile::TempDir {
            let clone = tempfile::tempdir().unwrap();
            run_git(
                clone.path(),
                &[
                    "clone",
                    "--quiet",
                    "--depth=1",
                    "--branch=large-pr",
                    &format!("file://{}", self.repo.path().display()),
                    ".",
                ],
            );
            clone
        }

        fn pr_view_json(&self) -> String {
            format!(
                r#"{{
                    "number": 125,
                    "state": "OPEN",
                    "headRefName": "large-pr",
                    "baseRefName": "main",
                    "headRefOid": "{}",
                    "baseRefOid": "{}"
                }}"#,
                self.head_sha, self.base_sha,
            )
        }
    }

    struct LinkedWorktree {
        source: PathBuf,
        parent: tempfile::TempDir,
        root: PathBuf,
    }

    impl LinkedWorktree {
        fn new(source: &Path, branch: &str) -> Self {
            let parent = tempfile::tempdir().unwrap();
            let root = parent.path().join("checkout");
            run_git(
                source,
                &["worktree", "add", "--quiet", root.to_str().unwrap(), branch],
            );
            Self {
                source: source.to_path_buf(),
                parent,
                root,
            }
        }

        fn path(&self) -> &Path {
            &self.root
        }
    }

    impl Drop for LinkedWorktree {
        fn drop(&mut self) {
            let _keep_parent_alive = &self.parent;
            let _ = Command::new("git")
                .current_dir(&self.source)
                .args(["worktree", "remove", "--force"])
                .arg(&self.root)
                .output();
        }
    }

    fn run_git(repo: &Path, args: &[&str]) -> String {
        let output = Command::new("git")
            .current_dir(repo)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    }

    #[derive(Clone)]
    struct LargeDiffGhRunner {
        state: Rc<LargeDiffRunnerState>,
    }

    struct LargeDiffRunnerState {
        calls: RefCell<Vec<Vec<String>>>,
        clone_source: PathBuf,
        pr_view_json: String,
    }

    impl LargeDiffGhRunner {
        fn new(fixture: &LargeDiffFixture) -> Self {
            Self {
                state: Rc::new(LargeDiffRunnerState {
                    calls: RefCell::new(Vec::new()),
                    clone_source: fixture.repo.path().to_path_buf(),
                    pr_view_json: fixture.pr_view_json(),
                }),
            }
        }

        fn clone_args(&self) -> Option<Vec<String>> {
            self.state
                .calls
                .borrow()
                .iter()
                .find(|args| {
                    args.first().map(String::as_str) == Some("repo")
                        && args.get(1).map(String::as_str) == Some("clone")
                })
                .cloned()
        }

        fn used_temporary_clone(&self) -> bool {
            self.clone_args().is_some()
        }
    }

    impl GhCommandRunner for LargeDiffGhRunner {
        fn run(&self, args: &[String]) -> GhCommandResult<String> {
            self.state.calls.borrow_mut().push(args.to_vec());
            match args.first().map(String::as_str) {
                Some("pr") if args.get(1).map(String::as_str) == Some("view") => {
                    Ok(self.state.pr_view_json.clone())
                }
                Some("pr") if args.get(1).map(String::as_str) == Some("diff") => {
                    Err(GhCommandError::Failed {
                        status: Some(1),
                        stderr: "could not find pull request diff: HTTP 406: Sorry, the diff exceeded the maximum number of files (300)".to_string(),
                    })
                }
                Some("repo") if args.get(1).map(String::as_str) == Some("clone") => {
                    self.clone_into(args.get(3).expect("clone destination"))
                }
                _ => Err(GhCommandError::Failed {
                    status: Some(1),
                    stderr: "unexpected command".to_string(),
                }),
            }
        }
    }

    impl LargeDiffGhRunner {
        fn clone_into(&self, destination: &str) -> GhCommandResult<String> {
            let output = Command::new("git")
                .args([
                    "clone",
                    "--quiet",
                    "--bare",
                    self.state.clone_source.to_str().unwrap(),
                    destination,
                ])
                .output()
                .unwrap();
            if output.status.success() {
                Ok(String::new())
            } else {
                Err(GhCommandError::Failed {
                    status: output.status.code(),
                    stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
                })
            }
        }
    }

    fn repository() -> ForgeRepository {
        ForgeRepository::github("github.com", "agavra", "tuicr")
    }

    fn details(backend: &impl ForgeBackend) -> PullRequestDetails {
        backend
            .get_pull_request(PullRequestTarget::with_repository(repository(), 125, "125"))
            .unwrap()
    }

    #[test]
    fn opens_prs_over_300_files_from_an_isolated_partial_clone() {
        let fixture = LargeDiffFixture::new(301);
        let runner = LargeDiffGhRunner::new(&fixture);
        let backend = GitHubGhBackend::with_runner(Some(repository()), runner.clone());

        let opened = open_pull_request(
            &backend,
            PullRequestTarget::with_repository(repository(), 125, "125"),
            None,
            &SyntaxHighlighter::default(),
        )
        .unwrap();

        assert_eq!(opened.diff_files.len(), 301);
        assert_eq!(
            opened.diff_files.last().unwrap().display_path(),
            Path::new("src/file-300.rs")
        );
        assert!(runner.used_temporary_clone());
    }

    #[test]
    fn diffs_from_merge_base_when_the_base_branch_has_advanced() {
        let mut fixture = LargeDiffFixture::new(1);
        fixture.advance_base();
        let runner = LargeDiffGhRunner::new(&fixture);
        let backend = GitHubGhBackend::with_runner(Some(repository()), runner);

        let patch = backend.get_pull_request_diff(&details(&backend)).unwrap();

        assert!(patch.contains("src/file-000.rs"));
        assert!(!patch.contains("base-only.txt"));
    }

    #[test]
    fn uses_the_current_checkout_when_branch_and_head_match_the_pr() {
        let fixture = LargeDiffFixture::new(1);
        fixture.checkout_pr_head();
        let runner = LargeDiffGhRunner::new(&fixture);
        let mut backend = GitHubGhBackend::with_runner(Some(repository()), runner.clone());
        backend.set_local_checkout(Some(fixture.repo.path().to_path_buf()));

        let patch = backend.get_pull_request_diff(&details(&backend)).unwrap();

        assert!(patch.contains("src/file-000.rs"));
        assert!(!runner.used_temporary_clone());
    }

    #[test]
    fn local_diff_uses_merge_base_when_the_base_branch_has_advanced() {
        let mut fixture = LargeDiffFixture::new(1);
        fixture.advance_base();
        fixture.checkout_pr_head();
        let runner = LargeDiffGhRunner::new(&fixture);
        let mut backend = GitHubGhBackend::with_runner(Some(repository()), runner.clone());
        backend.set_local_checkout(Some(fixture.repo.path().to_path_buf()));

        let patch = backend.get_pull_request_diff(&details(&backend)).unwrap();

        assert!(patch.contains("src/file-000.rs"));
        assert!(!patch.contains("base-only.txt"));
        assert!(!runner.used_temporary_clone());
    }

    #[test]
    fn rejects_a_matching_branch_when_head_has_advanced_locally() {
        let fixture = LargeDiffFixture::new(1);
        fixture.checkout_pr_head();
        fixture.advance_pr_head_locally();
        let runner = LargeDiffGhRunner::new(&fixture);
        let mut backend = GitHubGhBackend::with_runner(Some(repository()), runner.clone());
        backend.set_local_checkout(Some(fixture.repo.path().to_path_buf()));

        let patch = backend.get_pull_request_diff(&details(&backend)).unwrap();

        assert!(patch.contains("src/file-000.rs"));
        assert!(!patch.contains("local-only.txt"));
        assert!(runner.used_temporary_clone());
    }

    #[test]
    fn dirty_files_do_not_enter_the_local_pr_diff() {
        let fixture = LargeDiffFixture::new(1);
        fixture.checkout_pr_head();
        fs::write(fixture.repo.path().join("README.md"), "staged checkout\n").unwrap();
        run_git(fixture.repo.path(), &["add", "README.md"]);
        fs::write(fixture.repo.path().join("untracked.txt"), "untracked\n").unwrap();
        let runner = LargeDiffGhRunner::new(&fixture);
        let mut backend = GitHubGhBackend::with_runner(Some(repository()), runner.clone());
        backend.set_local_checkout(Some(fixture.repo.path().to_path_buf()));

        let patch = backend.get_pull_request_diff(&details(&backend)).unwrap();

        assert!(!patch.contains("staged checkout"));
        assert!(!patch.contains("untracked.txt"));
        assert!(!runner.used_temporary_clone());
    }

    #[test]
    fn local_diff_ignores_replace_objects() {
        let fixture = LargeDiffFixture::new(1);
        fixture.checkout_pr_head();
        run_git(
            fixture.repo.path(),
            &["replace", &fixture.head_sha, &fixture.base_sha],
        );
        let runner = LargeDiffGhRunner::new(&fixture);
        let mut backend = GitHubGhBackend::with_runner(Some(repository()), runner.clone());
        backend.set_local_checkout(Some(fixture.repo.path().to_path_buf()));

        let patch = backend.get_pull_request_diff(&details(&backend)).unwrap();

        assert!(patch.contains("src/file-000.rs"));
        assert!(!runner.used_temporary_clone());
    }

    #[cfg(unix)]
    #[test]
    fn local_diff_does_not_run_external_diff_or_textconv_commands() {
        use std::os::unix::fs::PermissionsExt;

        let fixture = LargeDiffFixture::new(1);
        fixture.checkout_pr_head();
        let marker = fixture.repo.path().join("diff-command-ran");
        let script = fixture.repo.path().join("diff-command.sh");
        fs::write(
            &script,
            format!("#!/bin/sh\ntouch '{}'\nexit 1\n", marker.display()),
        )
        .unwrap();
        let mut permissions = fs::metadata(&script).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&script, permissions).unwrap();
        run_git(
            fixture.repo.path(),
            &["config", "diff.external", script.to_str().unwrap()],
        );
        run_git(
            fixture.repo.path(),
            &["config", "diff.evil.textconv", script.to_str().unwrap()],
        );
        fs::write(
            fixture.repo.path().join(".git/info/attributes"),
            "*.rs diff=evil\n",
        )
        .unwrap();
        let runner = LargeDiffGhRunner::new(&fixture);
        let mut backend = GitHubGhBackend::with_runner(Some(repository()), runner.clone());
        backend.set_local_checkout(Some(fixture.repo.path().to_path_buf()));

        let patch = backend.get_pull_request_diff(&details(&backend)).unwrap();

        assert!(patch.contains("src/file-000.rs"));
        assert!(!marker.exists());
        assert!(!runner.used_temporary_clone());
    }

    #[test]
    fn detached_head_uses_the_isolated_fallback() {
        let fixture = LargeDiffFixture::new(1);
        fixture.checkout_pr_head();
        run_git(fixture.repo.path(), &["checkout", "--quiet", "--detach"]);
        let runner = LargeDiffGhRunner::new(&fixture);
        let mut backend = GitHubGhBackend::with_runner(Some(repository()), runner.clone());
        backend.set_local_checkout(Some(fixture.repo.path().to_path_buf()));

        backend.get_pull_request_diff(&details(&backend)).unwrap();

        assert!(runner.used_temporary_clone());
    }

    #[test]
    fn shallow_checkout_without_the_base_uses_the_isolated_fallback() {
        let fixture = LargeDiffFixture::new(1);
        let shallow = fixture.shallow_pr_clone();
        assert!(
            Command::new("git")
                .current_dir(shallow.path())
                .args(["cat-file", "-e", &fixture.base_sha])
                .output()
                .is_ok_and(|output| !output.status.success())
        );
        let runner = LargeDiffGhRunner::new(&fixture);
        let mut backend = GitHubGhBackend::with_runner(Some(repository()), runner.clone());
        backend.set_local_checkout(Some(shallow.path().to_path_buf()));

        backend.get_pull_request_diff(&details(&backend)).unwrap();

        assert!(runner.used_temporary_clone());
    }

    #[test]
    fn exact_pr_branch_in_a_linked_worktree_uses_local_objects() {
        let fixture = LargeDiffFixture::new(1);
        let worktree = fixture.linked_pr_worktree();
        let runner = LargeDiffGhRunner::new(&fixture);
        let mut backend = GitHubGhBackend::with_runner(Some(repository()), runner.clone());
        backend.set_local_checkout(Some(worktree.path().to_path_buf()));

        let patch = backend.get_pull_request_diff(&details(&backend)).unwrap();

        assert!(patch.contains("src/file-000.rs"));
        assert!(!runner.used_temporary_clone());
        assert_eq!(
            local_common_git_dir(worktree.path()).unwrap(),
            fixture.repo.path().join(".git")
        );
    }

    #[test]
    fn temporary_clone_borrows_objects_from_a_matching_checkout() {
        let fixture = LargeDiffFixture::new(1);
        let runner = LargeDiffGhRunner::new(&fixture);
        let mut backend = GitHubGhBackend::with_runner(Some(repository()), runner.clone());
        backend.set_local_checkout(Some(fixture.repo.path().to_path_buf()));

        backend.get_pull_request_diff(&details(&backend)).unwrap();

        let reference_arg = format!(
            "--reference-if-able={}",
            fixture.repo.path().join(".git").display()
        );
        let clone_args = runner.clone_args().unwrap();
        assert!(clone_args.contains(&reference_arg));
        assert!(clone_args.contains(&"--single-branch".to_string()));
        assert!(clone_args.contains(&"--branch=main".to_string()));
    }

    #[test]
    fn temporary_clone_without_a_checkout_has_no_local_reference() {
        let fixture = LargeDiffFixture::new(1);
        let runner = LargeDiffGhRunner::new(&fixture);
        let backend = GitHubGhBackend::with_runner(Some(repository()), runner.clone());

        backend.get_pull_request_diff(&details(&backend)).unwrap();

        assert!(
            runner
                .clone_args()
                .unwrap()
                .iter()
                .all(|arg| !arg.starts_with("--reference"))
        );
    }

    #[test]
    fn temporary_clone_removes_its_directory_on_drop() {
        let path = {
            let repo = TemporaryClone::new().unwrap();
            let path = repo.path().to_path_buf();
            assert!(path.exists());
            path
        };

        assert!(!path.exists());
    }
}
