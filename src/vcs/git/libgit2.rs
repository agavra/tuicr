use git2::Repository;
use std::path::{Path, PathBuf};
use std::sync::Once;

use crate::error::{Result, TuicrError};
use crate::model::{DiffFile, DiffLine, FileStatus};
use crate::syntax::SyntaxHighlighter;

use super::{context, diff, repository, staging};
use crate::vcs::traits::{ChangeKind, CommitInfo, VcsBackend, VcsInfo, VcsType};

/// Git backend implementation using the git2/libgit2 library.
pub struct Libgit2Backend {
    repo: Repository,
    info: VcsInfo,
}

/// Declare libgit2 extensions tuicr understands so discovery doesn't refuse
/// repos that opt into newer git on-disk features.
///
/// Currently: `relativeworktrees` (git 2.48+ `worktree.useRelativePaths`).
/// Without this declaration libgit2 refuses to open a worktree created from a
/// bare clone with that setting — tuicr would surface as "Not a repository"
/// while plain `git status` works fine. Path resolution for relative
/// `gitdir:` pointers already works in libgit2; only the safety gate refuses.
fn register_supported_extensions() {
    static REGISTER: Once = Once::new();
    REGISTER.call_once(|| {
        // SAFETY: libgit2 stores extensions in a process-wide static. We call
        // this exactly once, before any `Repository::discover`, via `Once`.
        unsafe {
            let _ = git2::opts::set_extensions(&["relativeworktrees"]);
        }
    });
}

impl Libgit2Backend {
    pub(super) fn discover_from(cwd: &Path) -> Result<Self> {
        register_supported_extensions();
        let repo = Repository::discover(cwd).map_err(|_| TuicrError::NotARepository)?;

        let root_path = repo
            .workdir()
            .ok_or(TuicrError::NotARepository)?
            .to_path_buf();

        let head_commit = repo
            .head()
            .ok()
            .and_then(|h| h.peel_to_commit().ok())
            .map(|c| c.id().to_string())
            .unwrap_or_else(|| "HEAD".to_string());

        // For unborn HEAD (fresh `git init` / `git clone` of an empty remote),
        // `repo.head()` errors, so fall back to reading HEAD's symbolic target
        // directly. That way the status bar still shows e.g. `git:main` instead
        // of `git:detached` before the first commit lands.
        let branch_name = repo
            .head()
            .ok()
            .and_then(|h| {
                if h.is_branch() {
                    h.shorthand().map(|s| s.to_string())
                } else {
                    None
                }
            })
            .or_else(|| {
                repo.find_reference("HEAD")
                    .ok()
                    .and_then(|r| r.symbolic_target().map(str::to_string))
                    .and_then(|t| t.strip_prefix("refs/heads/").map(str::to_string))
            });

        let info = VcsInfo {
            root_path,
            head_commit,
            branch_name,
            vcs_type: VcsType::Git,
        };

        Ok(Self { repo, info })
    }
}

impl VcsBackend for Libgit2Backend {
    fn info(&self) -> &VcsInfo {
        &self.info
    }

    fn supports_sparse_checkout(&self) -> bool {
        false
    }

    fn get_working_tree_diff(&self, highlighter: &SyntaxHighlighter) -> Result<Vec<DiffFile>> {
        diff::get_working_tree_diff(&self.repo, highlighter)
    }

    fn get_staged_diff(&self, highlighter: &SyntaxHighlighter) -> Result<Vec<DiffFile>> {
        diff::get_staged_diff(&self.repo, highlighter)
    }

    fn get_unstaged_diff(&self, highlighter: &SyntaxHighlighter) -> Result<Vec<DiffFile>> {
        diff::get_unstaged_diff(&self.repo, highlighter)
    }

    fn list_changed_paths(&self, kind: ChangeKind) -> Result<Vec<PathBuf>> {
        diff::list_changed_paths(&self.repo, kind)
    }

    fn fetch_context_lines(
        &self,
        file_path: &Path,
        file_status: FileStatus,
        ref_commit: Option<&str>,
        start_line: u32,
        end_line: u32,
    ) -> Result<Vec<DiffLine>> {
        context::fetch_context_lines(
            &self.repo,
            file_path,
            file_status,
            ref_commit,
            start_line,
            end_line,
        )
    }

    fn file_line_count(
        &self,
        file_path: &Path,
        file_status: FileStatus,
        ref_commit: Option<&str>,
    ) -> Result<u32> {
        context::file_line_count(&self.repo, file_path, file_status, ref_commit)
    }

    fn get_recent_commits(&self, offset: usize, limit: usize) -> Result<Vec<CommitInfo>> {
        let git_commits = repository::get_recent_commits(&self.repo, offset, limit)?;
        Ok(git_commits
            .into_iter()
            .map(|c| CommitInfo {
                id: c.id,
                short_id: c.short_id,
                branch_name: c.branch_name,
                summary: c.summary,
                body: c.body,
                author: c.author,
                time: c.time,
            })
            .collect())
    }

    fn resolve_revisions(&self, revisions: &str) -> Result<Vec<String>> {
        repository::resolve_revisions(&self.repo, revisions)
    }

    fn get_commit_range_diff(
        &self,
        commit_ids: &[String],
        highlighter: &SyntaxHighlighter,
    ) -> Result<Vec<DiffFile>> {
        diff::get_commit_range_diff(&self.repo, commit_ids, highlighter)
    }

    fn get_commits_info(&self, ids: &[String]) -> Result<Vec<CommitInfo>> {
        let git_commits = repository::get_commits_info(&self.repo, ids)?;
        Ok(git_commits
            .into_iter()
            .map(|c| CommitInfo {
                id: c.id,
                short_id: c.short_id,
                branch_name: c.branch_name,
                summary: c.summary,
                body: c.body,
                author: c.author,
                time: c.time,
            })
            .collect())
    }

    fn get_working_tree_with_commits_diff(
        &self,
        commit_ids: &[String],
        highlighter: &SyntaxHighlighter,
    ) -> Result<Vec<DiffFile>> {
        diff::get_working_tree_with_commits_diff(&self.repo, commit_ids, highlighter)
    }

    fn stage_file(&self, path: &Path) -> Result<()> {
        staging::stage_file(&self.repo, path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::process::Command;

    fn git(workdir: &Path, args: &[&str]) {
        let output = Command::new("git")
            .current_dir(workdir)
            .args(args)
            .output()
            .expect("failed to run git");
        assert!(
            output.status.success(),
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn should_discover_worktree_with_relativeworktrees_extension() {
        // given: a bare clone with `extensions.relativeworktrees = true` set
        // and a worktree linked to it. This is the on-disk state git 2.48+
        // produces with `worktree.useRelativePaths`. Without
        // `set_extensions(["relativeworktrees"])` libgit2 refuses to open the
        // worktree at all, surfacing as "Not a repository" in tuicr.
        let temp = tempfile::tempdir().expect("temp dir");
        let source = temp.path().join("source");
        let bare = temp.path().join("bare.git");
        let worktree = temp.path().join("wt");

        fs::create_dir_all(&source).unwrap();
        git(&source, &["init", "-q", "-b", "main"]);
        git(&source, &["config", "user.email", "test@example.com"]);
        git(&source, &["config", "user.name", "Test User"]);
        fs::write(source.join("README"), "hello\n").unwrap();
        git(&source, &["add", "README"]);
        git(&source, &["commit", "-q", "-m", "init"]);

        git(
            temp.path(),
            &[
                "clone",
                "--bare",
                "-q",
                source.to_str().unwrap(),
                bare.to_str().unwrap(),
            ],
        );
        // The extension declaration is the gate libgit2 enforces. Setting
        // this alone reproduces the failure regardless of the local git
        // version — `worktree.useRelativePaths` (git 2.48+) is what writes
        // this in real-world setups. `core.repositoryFormatVersion = 1` is
        // required to opt into the `extensions.*` namespace at all.
        git(&bare, &["config", "core.repositoryFormatVersion", "1"]);
        git(&bare, &["config", "extensions.relativeworktrees", "true"]);
        git(
            &bare,
            &["worktree", "add", "-q", worktree.to_str().unwrap()],
        );

        // when
        let backend = Libgit2Backend::discover_from(&worktree)
            .expect("worktree with relativeworktrees extension should open");

        // then
        assert_eq!(backend.info().vcs_type, VcsType::Git);
        assert!(
            backend.repo.workdir().is_some(),
            "worktree must report a workdir"
        );
    }
}
