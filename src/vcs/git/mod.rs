mod cli;
pub mod context;
pub mod diff;
mod libgit2;
pub(crate) mod raw;
pub mod repository;
pub mod staging;

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use crate::error::{Result, TuicrError};
use crate::model::{DiffFile, DiffLine, FileStatus};
use crate::process::{CommandOutputError, CommandOutputErrorKind, run_command_output};
use crate::syntax::SyntaxHighlighter;

use super::traits::{
    ChangeKind, CommitInfo, DiffWhitespaceMode, ResolvedRevisionRange, VcsBackend, VcsChangeStatus,
    VcsInfo,
};
use cli::GitCliBackend;
pub use libgit2::Libgit2Backend;

// Re-exported for UI/app gap calculations.
pub use context::calculate_gap;

/// RevisionExpression is Git's parsed view of a user-supplied revision string.
///
/// Accepted forms are:
/// `REV` for a single commit, such as `HEAD`;
/// `A..B`, `A..`, or `..B` for a two-dot range; and
/// `A...B` for a merge-base range.
pub(super) enum RevisionExpression<'a> {
    /// A single commit expression, for example `HEAD`.
    Single(&'a str),

    /// A two-dot range.
    ///
    /// `A..B` is represented as `base = "A"` and `head = "B"`.
    /// `A..` is represented as `base = "A"` and `head = "HEAD"`.
    /// `..B` is represented as `base = "HEAD"` and `head = "B"`.
    Range { base: &'a str, head: &'a str },

    /// A three-dot range, for example `A...B`.
    MergeBaseRange { left: &'a str, right: &'a str },
}

impl<'a> RevisionExpression<'a> {
    /// Parse Git revision syntax accepted by tuicr's `-r` option.
    ///
    /// This accepts `REV`, `A..B`, `A..`, `..B`, and `A...B`.
    /// Open-ended two-dot ranges are normalized to use `HEAD` for the
    /// missing endpoint.
    pub(super) fn parse(revisions: &'a str) -> Result<Self> {
        if let Some((left, right)) = revisions.split_once("...") {
            if left.is_empty() || right.is_empty() {
                return Err(TuicrError::VcsCommand(
                    "Invalid revision range: missing endpoint".into(),
                ));
            }
            return Ok(Self::MergeBaseRange { left, right });
        }

        if let Some((base, head)) = revisions.split_once("..") {
            let base = if base.is_empty() { "HEAD" } else { base };
            let head = if head.is_empty() { "HEAD" } else { head };
            return Ok(Self::Range { base, head });
        }

        Ok(Self::Single(revisions))
    }
}

/// Top-level Git backend.
///
/// This wrapper keeps Git backend selection in one place. Today it delegates to
/// the git2/libgit2 implementation; sparse-checkout support can add another
/// variant without pushing backend-specific branches into every operation.
pub enum GitBackend {
    Libgit2(Libgit2Backend),
    Cli(GitCliBackend),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitBackendPreference {
    Libgit2,
    Cli,
}

impl GitBackendPreference {
    pub fn from_config(value: Option<&str>) -> Self {
        match value {
            Some("cli") => Self::Cli,
            _ => Self::Libgit2,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitRepoMode {
    Standard,
    SparseCheckout,
    SparseIndex,
}

impl GitRepoMode {
    fn detect(root_path: &Path) -> Result<Self> {
        let output = run_git_command(
            root_path,
            &[
                "config",
                "--get-regexp",
                r"^(core\.sparsecheckout|index\.sparse)$",
            ],
        )
        .unwrap_or_default();

        Ok(Self::from_config(&output))
    }

    fn from_config(output: &str) -> Self {
        let mut sparse_checkout = false;
        let mut sparse_index = false;

        for line in output.lines() {
            let mut parts = line.splitn(2, char::is_whitespace);
            let Some(key) = parts.next() else {
                continue;
            };
            let raw_value = parts.next().unwrap_or_default();

            match key {
                "core.sparsecheckout" => sparse_checkout = git_bool_config_enabled(raw_value),
                "index.sparse" => sparse_index = git_bool_config_enabled(raw_value),
                _ => {}
            }
        }

        if sparse_index {
            Self::SparseIndex
        } else if sparse_checkout {
            Self::SparseCheckout
        } else {
            Self::Standard
        }
    }

    fn is_sparse_checkout(self) -> bool {
        matches!(self, Self::SparseCheckout | Self::SparseIndex)
    }
}

impl GitBackend {
    /// Discover a git repository from the current directory.
    pub fn discover(
        preference: GitBackendPreference,
        whitespace_mode: DiffWhitespaceMode,
    ) -> Result<Self> {
        let cwd = std::env::current_dir().map_err(|_| TuicrError::NotARepository)?;
        Self::discover_from(&cwd, preference, whitespace_mode)
    }

    pub(crate) fn discover_from(
        cwd: &Path,
        preference: GitBackendPreference,
        whitespace_mode: DiffWhitespaceMode,
    ) -> Result<Self> {
        // libgit2 doesn't support reftable, split index, or SHA-256 repositories.
        // TODO: remove reftable fallback logic when libgit2 supports it as part of https://github.com/libgit2/libgit2/issues/5352
        // TODO: remove split index fallback logic when libgit2 supports it as part of https://github.com/libgit2/libgit2/issues/6132
        let use_cli = preference == GitBackendPreference::Cli
            || uses_reftable(cwd)
            || uses_split_index(cwd)
            || uses_sha256(cwd);

        if use_cli {
            return Ok(Self::Cli(GitCliBackend::discover_from(
                cwd,
                whitespace_mode,
            )?));
        }

        let backend = Self::Libgit2(Libgit2Backend::discover_from(cwd, whitespace_mode)?);
        let repo_mode = GitRepoMode::detect(&backend.info().root_path)?;
        if repo_mode.is_sparse_checkout() && !backend.supports_sparse_checkout() {
            return Ok(Self::Cli(GitCliBackend::discover_from(
                cwd,
                whitespace_mode,
            )?));
        }

        Ok(backend)
    }
}

/// Read a named remote's fetch URL, including Git's `insteadOf` rewrites.
pub(super) fn remote_url(repo_root: &Path, name: &str) -> Result<String> {
    run_git_command(repo_root, &["remote", "get-url", "--", name])
}

fn run_git_command(workdir: &Path, args: &[&str]) -> Result<String> {
    // `-c commit.gpgsign=false` is a no-op for the read-only `config`/`init`
    // calls this makes in production, but it keeps the test-only `commit`
    // calls below from prompting contributors with commit signing enabled
    // globally to sign throwaway commits in temp repos.
    let full_args: Vec<&str> = [
        "-c",
        "commit.gpgsign=false",
        "-c",
        "init.defaultRefFormat=files",
    ]
    .into_iter()
    .chain(args.iter().copied())
    .collect();
    run_command_output(
        "git",
        Some(workdir),
        full_args.iter().map(|arg| OsStr::new(*arg)),
    )
    .map_err(git_command_error)
}

pub(super) fn git_command_error(error: CommandOutputError) -> TuicrError {
    match error.kind {
        CommandOutputErrorKind::Unsuccessful => TuicrError::VcsCommand(error.stderr),
        CommandOutputErrorKind::NotFound | CommandOutputErrorKind::SpawnFailed => {
            TuicrError::VcsCommand(format!("Failed to run git: {}", error.stderr))
        }
    }
}

fn git_bool_config_enabled(value: &str) -> bool {
    matches!(value.trim(), "true" | "1" | "yes" | "on")
}

fn git_fsmonitor_config_enabled(value: &str) -> bool {
    let value = value.trim();
    git_bool_config_enabled(value)
        || (!value.is_empty() && !matches!(value, "false" | "0" | "no" | "off"))
}

fn uses_sha256(cwd: &Path) -> bool {
    run_git_command(cwd, &["rev-parse", "--show-object-format"])
        .is_ok_and(|format| format.trim() == "sha256")
}

fn uses_reftable(cwd: &Path) -> bool {
    run_git_command(cwd, &["config", "--get", "extensions.refStorage"])
        .ok()
        .map(|v| v.trim().eq_ignore_ascii_case("reftable"))
        .unwrap_or(false)
}

fn uses_split_index(cwd: &Path) -> bool {
    run_git_command(cwd, &["config", "--get", "core.splitIndex"])
        .ok()
        .map(|v| git_bool_config_enabled(&v))
        .unwrap_or(false)
}

impl VcsBackend for GitBackend {
    fn info(&self) -> &VcsInfo {
        match self {
            Self::Libgit2(backend) => backend.info(),
            Self::Cli(backend) => backend.info(),
        }
    }

    fn remote_url(&self, name: &str) -> Result<String> {
        remote_url(&self.info().root_path, name)
    }

    fn startup_warnings(&self) -> Vec<String> {
        match self {
            Self::Libgit2(backend) => backend.startup_warnings(),
            Self::Cli(backend) => backend.startup_warnings(),
        }
    }

    fn supports_sparse_checkout(&self) -> bool {
        match self {
            Self::Libgit2(backend) => backend.supports_sparse_checkout(),
            Self::Cli(backend) => backend.supports_sparse_checkout(),
        }
    }

    fn get_working_tree_diff(&self, highlighter: &SyntaxHighlighter) -> Result<Vec<DiffFile>> {
        match self {
            Self::Libgit2(backend) => backend.get_working_tree_diff(highlighter),
            Self::Cli(backend) => backend.get_working_tree_diff(highlighter),
        }
    }

    fn get_staged_diff(&self, highlighter: &SyntaxHighlighter) -> Result<Vec<DiffFile>> {
        match self {
            Self::Libgit2(backend) => backend.get_staged_diff(highlighter),
            Self::Cli(backend) => backend.get_staged_diff(highlighter),
        }
    }

    fn get_unstaged_diff(&self, highlighter: &SyntaxHighlighter) -> Result<Vec<DiffFile>> {
        match self {
            Self::Libgit2(backend) => backend.get_unstaged_diff(highlighter),
            Self::Cli(backend) => backend.get_unstaged_diff(highlighter),
        }
    }

    fn get_change_status(&self) -> Result<VcsChangeStatus> {
        match self {
            Self::Libgit2(backend) => backend.get_change_status(),
            Self::Cli(backend) => backend.get_change_status(),
        }
    }

    fn list_changed_paths(&self, kind: ChangeKind) -> Result<Vec<PathBuf>> {
        match self {
            Self::Libgit2(backend) => backend.list_changed_paths(kind),
            Self::Cli(backend) => backend.list_changed_paths(kind),
        }
    }

    fn fetch_context_lines(
        &self,
        file_path: &Path,
        file_status: FileStatus,
        ref_commit: Option<&str>,
        start_line: u32,
        end_line: u32,
    ) -> Result<Vec<DiffLine>> {
        match self {
            Self::Libgit2(backend) => backend.fetch_context_lines(
                file_path,
                file_status,
                ref_commit,
                start_line,
                end_line,
            ),
            Self::Cli(backend) => backend.fetch_context_lines(
                file_path,
                file_status,
                ref_commit,
                start_line,
                end_line,
            ),
        }
    }

    fn file_line_count(
        &self,
        file_path: &Path,
        file_status: FileStatus,
        ref_commit: Option<&str>,
    ) -> Result<u32> {
        match self {
            Self::Libgit2(backend) => backend.file_line_count(file_path, file_status, ref_commit),
            Self::Cli(backend) => backend.file_line_count(file_path, file_status, ref_commit),
        }
    }

    fn get_recent_commits(&self, offset: usize, limit: usize) -> Result<Vec<CommitInfo>> {
        match self {
            Self::Libgit2(backend) => backend.get_recent_commits(offset, limit),
            Self::Cli(backend) => backend.get_recent_commits(offset, limit),
        }
    }

    fn resolve_revision_range(&self, revisions: &str) -> Result<ResolvedRevisionRange<'static>> {
        match self {
            Self::Libgit2(backend) => backend.resolve_revision_range(revisions),
            Self::Cli(backend) => backend.resolve_revision_range(revisions),
        }
    }

    fn get_commit_range_diff(
        &self,
        revision_range: &ResolvedRevisionRange<'_>,
        highlighter: &SyntaxHighlighter,
    ) -> Result<Vec<DiffFile>> {
        match self {
            Self::Libgit2(backend) => backend.get_commit_range_diff(revision_range, highlighter),
            Self::Cli(backend) => backend.get_commit_range_diff(revision_range, highlighter),
        }
    }

    fn get_commits_info(&self, ids: &[String]) -> Result<Vec<CommitInfo>> {
        match self {
            Self::Libgit2(backend) => backend.get_commits_info(ids),
            Self::Cli(backend) => backend.get_commits_info(ids),
        }
    }

    fn get_working_tree_with_commits_diff(
        &self,
        commit_ids: &[String],
        highlighter: &SyntaxHighlighter,
    ) -> Result<Vec<DiffFile>> {
        match self {
            Self::Libgit2(backend) => {
                backend.get_working_tree_with_commits_diff(commit_ids, highlighter)
            }
            Self::Cli(backend) => {
                backend.get_working_tree_with_commits_diff(commit_ids, highlighter)
            }
        }
    }

    fn stage_file(&self, path: &Path) -> Result<()> {
        match self {
            Self::Libgit2(backend) => backend.stage_file(path),
            Self::Cli(backend) => backend.stage_file(path),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::forge::{resolve_remote_repository, traits::ForgeRepository};
    use git2::Repository;
    use std::fs;
    use tempfile::tempdir;

    fn init_repo_with_origin(url: &str) -> tempfile::TempDir {
        let dir = tempdir().unwrap();
        let repo = Repository::init(dir.path()).unwrap();
        repo.remote("origin", url).unwrap();
        dir
    }

    #[test]
    fn resolves_named_remote_fetch_url_from_subdirectory() {
        let dir = init_repo_with_origin("https://github.com/owner/repo");
        let repo = Repository::open(dir.path()).unwrap();
        repo.remote("staging-upstream", "git@github.com:org/repo-staging.git")
            .unwrap();
        repo.remote_set_pushurl(
            "staging-upstream",
            Some("https://github.com/contributor/repo-staging.git"),
        )
        .unwrap();
        let nested = dir.path().join("nested");
        std::fs::create_dir(&nested).unwrap();

        for preference in [GitBackendPreference::Libgit2, GitBackendPreference::Cli] {
            let backend =
                GitBackend::discover_from(&nested, preference, DiffWhitespaceMode::Normal).unwrap();
            assert_eq!(
                resolve_remote_repository(&backend, "staging-upstream").unwrap(),
                ForgeRepository::github("github.com", "org", "repo-staging")
            );
        }
    }

    #[test]
    fn resolves_remote_url_rewrites_without_using_push_rewrites() {
        let dir = init_repo_with_origin("https://github.com/owner/repo");
        let repo = Repository::open(dir.path()).unwrap();
        repo.remote("staging", "tuicr-test:group/repo.git").unwrap();
        let mut config = repo.config().unwrap();
        config
            .set_str("url.https://gitlab.com/.insteadOf", "tuicr-test:")
            .unwrap();
        config
            .set_str("url.https://github.com/.pushInsteadOf", "tuicr-test:")
            .unwrap();
        let backend = GitBackend::discover_from(
            dir.path(),
            GitBackendPreference::Libgit2,
            DiffWhitespaceMode::Normal,
        )
        .unwrap();

        assert_eq!(
            resolve_remote_repository(&backend, "staging").unwrap(),
            ForgeRepository::gitlab("gitlab.com", "group", "repo")
        );
    }

    #[test]
    fn rejects_unusable_named_remotes_instead_of_falling_back_to_origin() {
        let dir = init_repo_with_origin("https://github.com/owner/repo");
        let repo = Repository::open(dir.path()).unwrap();
        repo.remote("invalid", "not-a-url").unwrap();
        let backend = GitBackend::discover_from(
            dir.path(),
            GitBackendPreference::Libgit2,
            DiffWhitespaceMode::Normal,
        )
        .unwrap();
        for name in ["missing", "invalid", "--all"] {
            assert!(matches!(
                resolve_remote_repository(&backend, name),
                Err(TuicrError::Forge(_))
            ));
        }
    }

    #[test]
    fn derives_git_repo_mode_from_config() {
        assert_eq!(GitRepoMode::from_config(""), GitRepoMode::Standard);
        assert_eq!(
            GitRepoMode::from_config("core.sparsecheckout true\n"),
            GitRepoMode::SparseCheckout
        );
        assert_eq!(
            GitRepoMode::from_config("core.sparsecheckout true\nindex.sparse true\n"),
            GitRepoMode::SparseIndex
        );
    }

    #[test]
    fn derives_backend_preference_from_config() {
        assert_eq!(
            GitBackendPreference::from_config(None),
            GitBackendPreference::Libgit2
        );
        assert_eq!(
            GitBackendPreference::from_config(Some("libgit2")),
            GitBackendPreference::Libgit2
        );
        assert_eq!(
            GitBackendPreference::from_config(Some("cli")),
            GitBackendPreference::Cli
        );
    }

    #[test]
    fn default_preference_routes_sparse_index_repo_to_cli_with_warning() {
        let temp_dir = tempdir().expect("failed to create temp dir");
        let root = temp_dir.path();
        setup_standard_repo(root);
        run_git_command(
            root,
            &["sparse-checkout", "init", "--cone", "--sparse-index"],
        )
        .expect("failed to enable sparse checkout");
        run_git_command(root, &["sparse-checkout", "set", "src"])
            .expect("failed to set sparse checkout paths");

        let backend = GitBackend::discover_from(
            root,
            GitBackendPreference::Libgit2,
            DiffWhitespaceMode::Normal,
        )
        .expect("failed to discover backend");

        match backend {
            GitBackend::Cli(backend) => {
                assert!(backend.supports_sparse_checkout());
                assert_eq!(
                    backend.startup_warnings().first().map(String::as_str),
                    Some("Sparse checkout detected; using Git CLI backend.")
                );
            }
            GitBackend::Libgit2(_) => panic!("sparse-index repo should use Git CLI backend"),
        }
    }

    #[test]
    fn default_preference_keeps_standard_repo_on_libgit2() {
        let temp_dir = tempdir().expect("failed to create temp dir");
        let root = temp_dir.path();
        setup_standard_repo(root);

        let backend = GitBackend::discover_from(
            root,
            GitBackendPreference::Libgit2,
            DiffWhitespaceMode::Normal,
        )
        .expect("failed to discover backend");

        match backend {
            GitBackend::Libgit2(backend) => assert!(!backend.supports_sparse_checkout()),
            GitBackend::Cli(_) => panic!("standard repo should use libgit2 by default"),
        }
    }

    #[test]
    fn default_preference_routes_reftable_repo_to_cli() {
        let temp_dir = tempdir().expect("failed to create temp dir");
        let root = temp_dir.path();
        setup_standard_repo(root);
        run_git_command(root, &["config", "core.repositoryFormatVersion", "1"])
            .expect("failed to set repositoryFormatVersion");
        run_git_command(root, &["config", "extensions.refStorage", "reftable"])
            .expect("failed to set reftable extension");

        let backend = GitBackend::discover_from(
            root,
            GitBackendPreference::Libgit2,
            DiffWhitespaceMode::Normal,
        )
        .expect("reftable repo should open via CLI fallback");

        assert!(
            matches!(backend, GitBackend::Cli(_)),
            "reftable repo should use Git CLI backend, not libgit2"
        );
    }

    #[test]
    fn default_preference_routes_split_index_repo_to_cli() {
        let temp_dir = tempdir().expect("failed to create temp dir");
        let root = temp_dir.path();
        setup_standard_repo(root);
        run_git_command(root, &["config", "core.splitIndex", "true"])
            .expect("failed to set splitIndex");

        let backend: GitBackend = GitBackend::discover_from(
            root,
            GitBackendPreference::Libgit2,
            DiffWhitespaceMode::Normal,
        )
        .expect("split-index repo should open via CLI fallback");

        assert!(
            matches!(backend, GitBackend::Cli(_)),
            "split-index repo should use Git CLI backend, not libgit2"
        );
    }

    fn setup_object_format_repo(format: &str) -> tempfile::TempDir {
        let dir = tempdir().unwrap();
        run_git_command(dir.path(), &["init", &format!("--object-format={format}")]).unwrap();
        setup_standard_repo(dir.path());
        dir
    }

    #[test]
    fn sha256_default_discovery_from_root_and_subdirectory_uses_cli() {
        let dir = setup_object_format_repo("sha256");
        for path in [dir.path().to_path_buf(), dir.path().join("src")] {
            let backend = GitBackend::discover_from(
                &path,
                GitBackendPreference::Libgit2,
                DiffWhitespaceMode::Normal,
            )
            .expect("SHA-256 repositories should open via the Git CLI");
            assert!(matches!(backend, GitBackend::Cli(_)));
            assert_eq!(backend.get_recent_commits(0, 10).unwrap()[0].id.len(), 64);
        }
    }

    fn assert_root_diff_for_object_formats(target: &str) {
        for format in ["sha1", "sha256"] {
            let dir = setup_object_format_repo(format);
            let backend = GitBackend::discover_from(
                dir.path(),
                GitBackendPreference::Cli,
                DiffWhitespaceMode::Normal,
            )
            .unwrap();
            let highlighter = SyntaxHighlighter::default();
            let explicit = backend.resolve_revision_range("HEAD").unwrap();
            let files = match target {
                "explicit" => backend.get_commit_range_diff(&explicit, &highlighter),
                "list" => backend.get_commit_range_diff(
                    &ResolvedRevisionRange::from_owned_commit_ids(
                        explicit.commit_ids.to_vec(),
                        super::super::traits::RevisionDiffTarget::CommitList,
                    ),
                    &highlighter,
                ),
                "worktree" => {
                    backend.get_working_tree_with_commits_diff(&explicit.commit_ids, &highlighter)
                }
                _ => unreachable!(),
            }
            .unwrap_or_else(|error| panic!("{format} {target} root diff failed: {error}"));
            assert_eq!(files.len(), 1);
            assert_eq!(files[0].new_path, Some(PathBuf::from("src/file.txt")));
            assert_eq!(files[0].status, FileStatus::Added);
            assert!(
                files[0]
                    .hunks
                    .iter()
                    .flat_map(|h| &h.lines)
                    .any(|l| l.content.contains("one"))
            );
        }
    }

    #[test]
    fn sha256_ranges_worktree_changes_and_context_keep_full_commit_ids() {
        let dir = setup_object_format_repo("sha256");
        let root = dir.path();
        let first = run_git_command(root, &["rev-parse", "HEAD"])
            .unwrap()
            .trim()
            .to_string();
        fs::write(root.join("src/file.txt"), "one\ntwo\n").unwrap();
        run_git_command(root, &["add", "."]).unwrap();
        run_git_command(root, &["commit", "-m", "second"]).unwrap();
        let second = run_git_command(root, &["rev-parse", "HEAD"])
            .unwrap()
            .trim()
            .to_string();
        let backend = GitBackend::discover_from(
            root,
            GitBackendPreference::Libgit2,
            DiffWhitespaceMode::Normal,
        )
        .unwrap();
        let highlighter = SyntaxHighlighter::default();
        let range = backend
            .resolve_revision_range(&format!("{first}..{second}"))
            .unwrap();
        assert_eq!(range.commit_ids.as_ref(), &[second.clone()]);
        assert_eq!(
            backend
                .get_commits_info(&[first.clone(), second.clone()])
                .unwrap()
                .len(),
            2
        );
        let files = backend.get_commit_range_diff(&range, &highlighter).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].status, FileStatus::Modified);
        let context = backend
            .fetch_context_lines(
                Path::new("src/file.txt"),
                FileStatus::Modified,
                Some(&second),
                1,
                2,
            )
            .unwrap();
        assert_eq!(context.len(), 2);
        assert!(context[1].content.contains("two"));
        fs::write(root.join("src/file.txt"), "one\ntwo\nstaged\n").unwrap();
        run_git_command(root, &["add", "."]).unwrap();
        fs::write(root.join("src/file.txt"), "one\ntwo\nstaged\nunstaged\n").unwrap();
        assert_eq!(backend.get_staged_diff(&highlighter).unwrap().len(), 1);
        assert_eq!(backend.get_unstaged_diff(&highlighter).unwrap().len(), 1);
        assert_eq!(
            backend.get_working_tree_diff(&highlighter).unwrap().len(),
            1
        );
        assert_eq!(
            run_git_command(root, &["rev-parse", "HEAD"])
                .unwrap()
                .trim(),
            second
        );
    }

    #[test]
    fn sha256_root_commit_explicit_diff() {
        assert_root_diff_for_object_formats("explicit");
    }

    #[test]
    fn sha256_root_commit_list_diff() {
        assert_root_diff_for_object_formats("list");
    }

    #[test]
    fn sha256_root_commit_with_worktree_diff() {
        assert_root_diff_for_object_formats("worktree");
    }

    fn setup_standard_repo(root: &Path) {
        fs::create_dir(root.join("src")).expect("failed to create src dir");
        fs::write(root.join("src/file.txt"), "one\n").expect("failed to write file");

        run_git_command(root, &["init"]).expect("failed to init repo");
        run_git_command(root, &["config", "user.name", "Tuicr Test"])
            .expect("failed to set user name");
        run_git_command(root, &["config", "user.email", "tuicr@example.com"])
            .expect("failed to set user email");
        run_git_command(root, &["add", "src/file.txt"]).expect("failed to add file");
        run_git_command(root, &["commit", "-m", "initial"]).expect("failed to commit");
    }
}
