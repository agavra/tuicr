use git2::{Delta, Diff, DiffOptions, Repository};
use std::path::{Path, PathBuf};

use crate::error::{Result, TuicrError};
use crate::model::{DiffFile, DiffHunk, DiffLine, FileStatus, LineColoring, LineOrigin};
use crate::syntax::SyntaxHighlighter;
use crate::vcs::traits::{
    ChangeKind, DiffWhitespaceMode, ResolvedRevisionRange, RevisionDiffTarget,
};
use crate::vcs::{enhance_with_full_file_highlight, tabify};

pub fn get_working_tree_diff(
    repo: &Repository,
    whitespace_mode: DiffWhitespaceMode,
    highlighter: &SyntaxHighlighter,
) -> Result<Vec<DiffFile>> {
    // Unborn HEAD (fresh `git init` / `git clone` of an empty remote) has no
    // tree to compare against; diff against an empty baseline so freshly
    // staged/added files still surface in the working-tree review.
    let head = repo.head().ok().and_then(|h| h.peel_to_tree().ok());

    let mut opts = diff_options(whitespace_mode);
    opts.include_untracked(true);
    opts.show_untracked_content(true);
    opts.recurse_untracked_dirs(true);

    let diff = repo.diff_tree_to_workdir_with_index(head.as_ref(), Some(&mut opts))?;
    let mut files = parse_diff(&diff)?;
    enhance_with_full_file_highlight(
        &mut files,
        highlighter,
        |path| {
            head.as_ref()
                .and_then(|tree| read_path_from_tree(repo, tree, path))
        },
        |path| read_path_from_workdir(repo, path),
    );
    Ok(files)
}

/// Get the staged diff (index vs HEAD)
/// On repos with no commits (unborn HEAD), diffs against an empty tree.
pub fn get_staged_diff(
    repo: &Repository,
    whitespace_mode: DiffWhitespaceMode,
    highlighter: &SyntaxHighlighter,
) -> Result<Vec<DiffFile>> {
    let head = repo.head().ok().and_then(|h| h.peel_to_tree().ok());
    let index = repo.index()?;
    let mut opts = diff_options(whitespace_mode);

    let diff = repo.diff_tree_to_index(head.as_ref(), Some(&index), Some(&mut opts))?;
    let mut files = parse_diff(&diff)?;
    enhance_with_full_file_highlight(
        &mut files,
        highlighter,
        |path| {
            head.as_ref()
                .and_then(|tree| read_path_from_tree(repo, tree, path))
        },
        |path| read_path_from_index(repo, &index, path),
    );
    Ok(files)
}

/// Get the unstaged diff (working tree vs index)
/// List the changed paths for either the staged or unstaged diff, without
/// materializing hunks or running the syntax highlighter. Lets callers verify
/// `get_change_status` against ignore rules cheaply — full diff parsing only
/// happens once the user actually selects the staged/unstaged view.
pub fn list_changed_paths(repo: &Repository, kind: ChangeKind) -> Result<Vec<PathBuf>> {
    let diff = match kind {
        ChangeKind::Staged => {
            let head = repo.head().ok().and_then(|h| h.peel_to_tree().ok());
            let index = repo.index()?;
            let mut opts = diff_options(DiffWhitespaceMode::Normal);
            repo.diff_tree_to_index(head.as_ref(), Some(&index), Some(&mut opts))?
        }
        ChangeKind::Unstaged => {
            let index = repo.index()?;
            let mut opts = diff_options(DiffWhitespaceMode::Normal);
            opts.include_untracked(true);
            // `show_untracked_content(false)` keeps libgit2 from reading each
            // untracked file's bytes — only the paths are needed here.
            opts.show_untracked_content(false);
            opts.recurse_untracked_dirs(true);
            opts.skip_binary_check(true);
            repo.diff_index_to_workdir(Some(&index), Some(&mut opts))?
        }
    };

    let mut paths: Vec<PathBuf> = Vec::with_capacity(diff.deltas().len());
    for delta in diff.deltas() {
        let path = delta
            .new_file()
            .path()
            .or_else(|| delta.old_file().path())
            .map(PathBuf::from);
        if let Some(p) = path {
            paths.push(p);
        }
    }
    Ok(paths)
}

pub fn get_unstaged_diff(
    repo: &Repository,
    whitespace_mode: DiffWhitespaceMode,
    highlighter: &SyntaxHighlighter,
) -> Result<Vec<DiffFile>> {
    let index = repo.index()?;
    let mut opts = diff_options(whitespace_mode);
    opts.include_untracked(true);
    opts.show_untracked_content(true);
    opts.recurse_untracked_dirs(true);

    let diff = repo.diff_index_to_workdir(Some(&index), Some(&mut opts))?;
    let mut files = parse_diff(&diff)?;
    enhance_with_full_file_highlight(
        &mut files,
        highlighter,
        |path| read_path_from_index(repo, &index, path),
        |path| read_path_from_workdir(repo, path),
    );
    Ok(files)
}

/// Get the diff for a range of commits.
/// `commit_ids` should be ordered from oldest to newest.
/// The diff compares the oldest commit's parent to the newest commit.
pub fn get_commit_range_diff(
    repo: &Repository,
    revision_range: &ResolvedRevisionRange<'_>,
    whitespace_mode: DiffWhitespaceMode,
    highlighter: &SyntaxHighlighter,
) -> Result<Vec<DiffFile>> {
    let (old_tree, new_tree) = match &revision_range.diff_target {
        RevisionDiffTarget::CommitList => {
            commit_list_range_trees(repo, &revision_range.commit_ids)?
        }
        RevisionDiffTarget::Explicit { base, head } => {
            let old_tree = match base {
                Some(base) => Some(tree_for_commit(repo, base)?),
                None => None,
            };
            let new_tree = tree_for_commit(repo, head)?;
            (old_tree, new_tree)
        }
    };

    diff_commit_trees(repo, old_tree, new_tree, whitespace_mode, highlighter)
}

fn commit_list_range_trees<'repo>(
    repo: &'repo Repository,
    commit_ids: &[String],
) -> Result<(Option<git2::Tree<'repo>>, git2::Tree<'repo>)> {
    if commit_ids.is_empty() {
        return Err(TuicrError::NoChanges);
    }

    let oldest_id = git2::Oid::from_str(&commit_ids[0])?;
    let oldest_commit = repo.find_commit(oldest_id)?;

    let newest_id = git2::Oid::from_str(commit_ids.last().unwrap())?;
    let newest_commit = repo.find_commit(newest_id)?;

    let old_tree = if oldest_commit.parent_count() > 0 {
        Some(oldest_commit.parent(0)?.tree()?)
    } else {
        None
    };

    Ok((old_tree, newest_commit.tree()?))
}

// Explicit revision ranges carry commit IDs for old/new endpoints,
// but libgit2 diffs require the endpoint trees.
fn tree_for_commit<'repo>(repo: &'repo Repository, commit_id: &str) -> Result<git2::Tree<'repo>> {
    let oid = git2::Oid::from_str(commit_id)?;
    Ok(repo.find_commit(oid)?.tree()?)
}

fn diff_commit_trees(
    repo: &Repository,
    old_tree: Option<git2::Tree<'_>>,
    new_tree: git2::Tree<'_>,
    whitespace_mode: DiffWhitespaceMode,
    highlighter: &SyntaxHighlighter,
) -> Result<Vec<DiffFile>> {
    let mut opts = diff_options(whitespace_mode);

    let diff = repo.diff_tree_to_tree(old_tree.as_ref(), Some(&new_tree), Some(&mut opts))?;
    let mut files = parse_diff(&diff)?;
    enhance_with_full_file_highlight(
        &mut files,
        highlighter,
        |path| {
            old_tree
                .as_ref()
                .and_then(|tree| read_path_from_tree(repo, tree, path))
        },
        |path| read_path_from_tree(repo, &new_tree, path),
    );
    Ok(files)
}

/// Get a combined diff from the parent of the oldest commit through to the working tree.
/// This shows both committed and working tree changes in a single diff.
pub fn get_working_tree_with_commits_diff(
    repo: &Repository,
    commit_ids: &[String],
    whitespace_mode: DiffWhitespaceMode,
    highlighter: &SyntaxHighlighter,
) -> Result<Vec<DiffFile>> {
    if commit_ids.is_empty() {
        return Err(TuicrError::NoChanges);
    }

    let oldest_id = git2::Oid::from_str(&commit_ids[0])?;
    let oldest_commit = repo.find_commit(oldest_id)?;

    let old_tree = if oldest_commit.parent_count() > 0 {
        Some(oldest_commit.parent(0)?.tree()?)
    } else {
        None
    };

    let mut opts = diff_options(whitespace_mode);
    opts.include_untracked(true);
    opts.show_untracked_content(true);
    opts.recurse_untracked_dirs(true);

    let diff = repo.diff_tree_to_workdir_with_index(old_tree.as_ref(), Some(&mut opts))?;
    let mut files = parse_diff(&diff)?;
    enhance_with_full_file_highlight(
        &mut files,
        highlighter,
        |path| {
            old_tree
                .as_ref()
                .and_then(|tree| read_path_from_tree(repo, tree, path))
        },
        |path| read_path_from_workdir(repo, path),
    );
    Ok(files)
}

fn diff_options(whitespace_mode: DiffWhitespaceMode) -> DiffOptions {
    let mut opts = DiffOptions::new();
    opts.ignore_whitespace(whitespace_mode.ignores_all());
    opts
}

fn read_path_from_tree(repo: &Repository, tree: &git2::Tree, path: &Path) -> Option<String> {
    let entry = tree.get_path(path).ok()?;
    let blob = repo.find_blob(entry.id()).ok()?;
    Some(String::from_utf8_lossy(blob.content()).into_owned())
}

fn read_path_from_workdir(repo: &Repository, path: &Path) -> Option<String> {
    crate::vcs::read_workdir_file(repo.workdir()?, path)
}

fn read_path_from_index(repo: &Repository, index: &git2::Index, path: &Path) -> Option<String> {
    let entry = index.get_path(path, 0)?;
    let blob = repo.find_blob(entry.id).ok()?;
    Some(String::from_utf8_lossy(blob.content()).into_owned())
}

fn parse_diff(diff: &Diff) -> Result<Vec<DiffFile>> {
    let mut files: Vec<DiffFile> = Vec::new();

    // Untracked files larger than this are shown in the file list but their
    // content is not parsed — they are likely logs, dumps, or build artefacts.
    const MAX_UNTRACKED_FILE_SIZE: u64 = 10 * 1_024 * 1_024;

    for (delta_idx, delta) in diff.deltas().enumerate() {
        let status = match delta.status() {
            Delta::Added | Delta::Untracked => FileStatus::Added,
            Delta::Deleted => FileStatus::Deleted,
            Delta::Modified => FileStatus::Modified,
            Delta::Renamed => FileStatus::Renamed,
            Delta::Copied => FileStatus::Copied,
            _ => FileStatus::Modified,
        };

        let old_path = delta.old_file().path().map(PathBuf::from);
        let new_path = delta.new_file().path().map(PathBuf::from);
        let is_binary = delta.old_file().is_binary() || delta.new_file().is_binary();
        let is_too_large =
            delta.status() == Delta::Untracked && delta.new_file().size() > MAX_UNTRACKED_FILE_SIZE;

        let hunks = if is_binary || is_too_large {
            Vec::new()
        } else {
            parse_hunks(diff, delta_idx)?
        };

        let content_hash = DiffFile::compute_content_hash(&hunks);
        files.push(DiffFile {
            old_path,
            new_path,
            status,
            hunks,
            is_binary,
            is_too_large,
            is_commit_message: false,
            content_hash,
        });
    }

    if files.is_empty() {
        return Err(TuicrError::NoChanges);
    }

    Ok(files)
}

/// Extracts every hunk for one delta, leaving every line `LineColoring::Pending`. See
/// `crate::vcs::color_hunk` for the pass that colors it.
fn parse_hunks(diff: &Diff, delta_idx: usize) -> Result<Vec<DiffHunk>> {
    let mut hunks: Vec<DiffHunk> = Vec::new();

    let patch = git2::Patch::from_diff(diff, delta_idx)?;

    if let Some(patch) = patch {
        for hunk_idx in 0..patch.num_hunks() {
            let (hunk, _) = patch.hunk(hunk_idx)?;

            let header = String::from_utf8_lossy(hunk.header()).trim().to_string();
            let old_start = hunk.old_start();
            let old_count = hunk.old_lines();
            let new_start = hunk.new_start();
            let new_count = hunk.new_lines();

            let mut line_contents: Vec<String> = Vec::new();
            let mut line_origins: Vec<LineOrigin> = Vec::new();
            let mut line_numbers: Vec<(Option<u32>, Option<u32>)> = Vec::new();

            for line_idx in 0..patch.num_lines_in_hunk(hunk_idx)? {
                let line = patch.line_in_hunk(hunk_idx, line_idx)?;

                let origin = match line.origin() {
                    '+' => LineOrigin::Addition,
                    '-' => LineOrigin::Deletion,
                    ' ' => LineOrigin::Context,
                    _ => LineOrigin::Context,
                };

                let raw = String::from_utf8_lossy(line.content());
                let content = tabify(raw.trim_end_matches(['\n', '\r']));

                line_contents.push(content);
                line_origins.push(origin);
                line_numbers.push((line.old_lineno(), line.new_lineno()));
            }

            let mut lines: Vec<DiffLine> = Vec::with_capacity(line_contents.len());
            for (idx, content) in line_contents.into_iter().enumerate() {
                let origin = line_origins[idx];
                let (old_lineno, new_lineno) = line_numbers[idx];

                lines.push(DiffLine {
                    origin,
                    content,
                    old_lineno,
                    new_lineno,
                    coloring: LineColoring::Pending,
                });
            }

            hunks.push(DiffHunk {
                header,
                lines,
                old_start,
                old_count,
                new_start,
                new_count,
            });
        }
    }

    Ok(hunks)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;

    use crate::vcs::git::{GitCliBackend, Libgit2Backend};
    use crate::vcs::traits::VcsBackend;

    fn create_initial_commit(repo: &Repository, file_name: &str, content: &str) {
        fs::write(repo.workdir().unwrap().join(file_name), content)
            .expect("failed to write initial file");

        let mut index = repo.index().expect("failed to open index");
        index
            .add_path(Path::new(file_name))
            .expect("failed to add file to index");
        index.write().expect("failed to write index");

        let tree_id = index.write_tree().expect("failed to write tree");
        let tree = repo.find_tree(tree_id).expect("failed to find tree");
        let sig = git2::Signature::now("Test User", "test@example.com")
            .expect("failed to create signature");

        repo.commit(Some("HEAD"), &sig, &sig, "initial", &tree, &[])
            .expect("failed to create commit");
    }

    #[test]
    fn should_return_no_changes_for_clean_repo() {
        let temp_dir = tempfile::tempdir().expect("failed to create temp dir");
        let repo = Repository::init(temp_dir.path()).expect("failed to init repo");
        create_initial_commit(&repo, "file.txt", "content\n");
        let head = repo.head().unwrap().peel_to_tree().unwrap();
        let diff = repo
            .diff_tree_to_tree(Some(&head), Some(&head), None)
            .unwrap();

        let result = parse_diff(&diff);

        assert!(matches!(result, Err(TuicrError::NoChanges)));
    }

    #[test]
    fn should_expand_tabs_to_spaces_in_git_hunks() {
        let temp_dir = tempfile::tempdir().expect("failed to create temp dir");
        let repo = Repository::init(temp_dir.path()).expect("failed to init repo");

        create_initial_commit(
            &repo, "file.txt", r#"old
"#,
        );

        fs::write(
            temp_dir.path().join("file.txt"),
            r#"	new
"#,
        )
        .expect("failed to update file");

        let files = get_working_tree_diff(
            &repo,
            DiffWhitespaceMode::Normal,
            &SyntaxHighlighter::default(),
        )
        .expect("failed to get diff");

        assert_eq!(files.len(), 1);
        let lines = &files[0].hunks[0].lines;

        assert!(
            lines.iter().any(|l| l.content == "    new"),
            "expected tab-expanded content in git diff lines"
        );
        assert!(lines.iter().all(|l| !l.content.contains('\t')));
    }

    /// Pins why the whole-file pass exists: recoloring a container-grammar hunk with
    /// only its own lines in scope produces different, worse colors.
    ///
    /// `color_visible_hunks` skips a hunk when no line in it is `Pending`, and the
    /// whole-file pass leaves none, so this recolor never happens in production. The
    /// pending assertion records that guard; the whole-file pass colors every line of this
    /// fixture, so it cannot fail today. The recolor below shows what the guard is worth.
    #[test]
    fn should_color_a_vue_hunk_differently_without_full_file_context() {
        let temp_dir = tempfile::tempdir().expect("failed to create temp dir");
        let repo = Repository::init(temp_dir.path()).expect("failed to init repo");

        // The changed line sits far below `<script setup>`, so the hunk's own three
        // lines of context cannot reach the tag that puts syntect into JavaScript.
        // That distance is the whole point: a nearby change would color correctly
        // either way and prove nothing.
        let filler: String = (0..30).map(|i| format!("const pad{i} = {i}\n")).collect();
        let head = format!(
            "<template>\n  <div>{{{{ msg }}}}</div>\n</template>\n\n<script setup>\nimport {{ ref }} from 'vue'\n{filler}"
        );

        let initial = format!("{head}const msg = ref('hi')\nconst other = 1\n</script>\n");
        create_initial_commit(&repo, "App.vue", &initial);

        let edited = format!("{head}const msg = ref('hello')\nconst other = 1\n</script>\n");
        fs::write(temp_dir.path().join("App.vue"), &edited).expect("failed to update file");

        let highlighter = SyntaxHighlighter::default();
        let mut files = get_working_tree_diff(&repo, DiffWhitespaceMode::Normal, &highlighter)
            .expect("failed to get diff");

        let pending = files[0]
            .hunks
            .iter()
            .flat_map(|h| &h.lines)
            .filter(|l| l.coloring.is_pending())
            .count();
        assert_eq!(
            pending, 0,
            "the full-file pass must leave no line pending, or `color_visible_hunks` \
             recolors this hunk with only its own lines in scope"
        );

        let before: Vec<String> = files[0].hunks[0]
            .lines
            .iter()
            .map(crate::model::diff_types::span_signature)
            .collect();

        // What the render pass would produce if the guard above ever failed.
        crate::vcs::color_hunk(&mut files[0].hunks[0], Path::new("App.vue"), &highlighter);
        let after: Vec<String> = files[0].hunks[0]
            .lines
            .iter()
            .map(crate::model::diff_types::span_signature)
            .collect();

        assert_ne!(
            before, after,
            "hunk-local coloring must differ from the full-file result, or this test \
             proves nothing about what the guard protects"
        );
    }

    #[test]
    fn should_highlight_vue_script_hunk_using_full_file_context() {
        let temp_dir = tempfile::tempdir().expect("failed to create temp dir");
        let repo = Repository::init(temp_dir.path()).expect("failed to init repo");

        let initial = "<template>\n  <div>{{ msg }}</div>\n</template>\n\n<script setup>\nimport { ref } from 'vue'\nconst msg = ref('hi')\nconst other = 1\n</script>\n";
        create_initial_commit(&repo, "App.vue", initial);

        let edited = "<template>\n  <div>{{ msg }}</div>\n</template>\n\n<script setup>\nimport { ref } from 'vue'\nconst msg = ref('hello')\nconst other = 1\n</script>\n";
        fs::write(temp_dir.path().join("App.vue"), edited).expect("failed to update file");

        let files = get_working_tree_diff(
            &repo,
            DiffWhitespaceMode::Normal,
            &SyntaxHighlighter::default(),
        )
        .expect("failed to get diff");
        assert_eq!(files.len(), 1);

        let changed_lines: Vec<_> = files[0].hunks[0]
            .lines
            .iter()
            .filter(|l| matches!(l.origin, LineOrigin::Addition | LineOrigin::Deletion))
            .collect();
        assert!(!changed_lines.is_empty(), "expected change lines in hunk");

        for line in changed_lines {
            let spans = line
                .coloring
                .spans()
                .unwrap_or_else(|| panic!("vue line should be highlighted: {line:?}"));
            let unique_fgs: std::collections::HashSet<_> =
                spans.iter().filter_map(|(s, _)| s.fg).collect();
            assert!(
                unique_fgs.len() >= 2,
                "vue hunk line {line:?} should have varied fg colors, got {unique_fgs:?}"
            );
        }
    }

    #[test]
    fn should_separate_staged_and_unstaged_diffs() {
        let temp_dir = tempfile::tempdir().expect("failed to create temp dir");
        let repo = Repository::init(temp_dir.path()).expect("failed to init repo");

        create_initial_commit(&repo, "file.txt", "base\n");

        fs::write(temp_dir.path().join("file.txt"), "unstaged\n").expect("failed to update file");

        let highlighter = SyntaxHighlighter::default();

        let unstaged = get_unstaged_diff(&repo, DiffWhitespaceMode::Normal, &highlighter)
            .expect("unstaged diff failed");
        assert_eq!(unstaged.len(), 1);
        assert!(matches!(
            get_staged_diff(&repo, DiffWhitespaceMode::Normal, &highlighter),
            Err(TuicrError::NoChanges)
        ));

        let mut index = repo.index().expect("failed to open index");
        index
            .add_path(Path::new("file.txt"))
            .expect("failed to add file to index");
        index.write().expect("failed to write index");

        let staged = get_staged_diff(&repo, DiffWhitespaceMode::Normal, &highlighter)
            .expect("staged diff failed");
        assert_eq!(staged.len(), 1);
        assert!(matches!(
            get_unstaged_diff(&repo, DiffWhitespaceMode::Normal, &highlighter),
            Err(TuicrError::NoChanges)
        ));
    }

    #[test]
    fn should_surface_staged_files_on_unborn_head() {
        // given a freshly initialised repo with a staged file but no commits
        // (the "naked clone" / `git init` state — HEAD is unborn)
        let temp_dir = tempfile::tempdir().expect("failed to create temp dir");
        let repo = Repository::init(temp_dir.path()).expect("failed to init repo");
        fs::write(temp_dir.path().join("file.txt"), "hello\n").expect("write file");
        let mut index = repo.index().expect("open index");
        index.add_path(Path::new("file.txt")).expect("stage file");
        index.write().expect("write index");

        // when
        let files = get_working_tree_diff(
            &repo,
            DiffWhitespaceMode::Normal,
            &SyntaxHighlighter::default(),
        )
        .expect("unborn HEAD should produce a diff against an empty tree");

        // then the staged file shows up as an addition rather than crashing
        // with `reference 'refs/heads/main' not found`
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].display_path(), Path::new("file.txt"));
        assert!(matches!(files[0].status, FileStatus::Added));
    }

    #[test]
    fn should_surface_noop_file_when_whitespace_only_diff_is_empty() {
        let temp_dir = tempfile::tempdir().expect("failed to create temp dir");
        let repo = Repository::init(temp_dir.path()).expect("failed to init repo");

        create_initial_commit(&repo, "file.txt", "alpha\nbeta\n");
        fs::write(temp_dir.path().join("file.txt"), " alpha \n beta\n")
            .expect("failed to update file");

        let files = get_working_tree_diff(
            &repo,
            DiffWhitespaceMode::IgnoreAll,
            &SyntaxHighlighter::default(),
        )
        .expect("whitespace-only edit may surface as a no-op diff file");
        assert_eq!(files.len(), 1);
        assert!(files[0].hunks.is_empty());

        fs::write(temp_dir.path().join("file.txt"), " alpha \ngamma\n")
            .expect("failed to update file");

        let files = get_working_tree_diff(
            &repo,
            DiffWhitespaceMode::IgnoreAll,
            &SyntaxHighlighter::default(),
        )
        .expect("non-whitespace edit should still produce a diff");
        assert_eq!(files.len(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn should_keep_mode_only_changes_when_ignoring_whitespace() {
        use std::os::unix::fs::PermissionsExt;

        let temp_dir = tempfile::tempdir().expect("failed to create temp dir");
        let repo = Repository::init(temp_dir.path()).expect("failed to init repo");
        create_initial_commit(&repo, "file.txt", "alpha\n");

        let path = temp_dir.path().join("file.txt");
        let mut permissions = fs::metadata(&path)
            .expect("failed to stat file")
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(path, permissions).expect("failed to update mode");

        let files = get_working_tree_diff(
            &repo,
            DiffWhitespaceMode::IgnoreAll,
            &SyntaxHighlighter::default(),
        )
        .expect("mode-only edit should still produce a diff");
        assert_eq!(files.len(), 1);
        assert!(files[0].hunks.is_empty());
    }

    /// Temporary git repo for the plain against highlighted test below.
    /// `create_initial_commit` above adds one commit to an already-open
    /// `Repository`. This builds the whole repo instead, so both the libgit2 and
    /// Git CLI backends can open it from its path.
    struct TempRepo {
        dir: tempfile::TempDir,
    }

    impl TempRepo {
        /// Creates a git repo with each `(path, content)` pair committed once
        /// with empty content, then rewritten to `content` as an unstaged
        /// change. Starts from a real commit rather than an unborn HEAD, which
        /// keeps both backends on their ordinary "diff against HEAD" path.
        fn with_changes(files: &[(&str, &str)]) -> Self {
            let dir = tempfile::tempdir().expect("failed to create temp dir");
            run_git(dir.path(), &["init"]);
            run_git(dir.path(), &["config", "user.name", "Tuicr Test"]);
            run_git(dir.path(), &["config", "user.email", "tuicr@example.com"]);
            for (path, _) in files {
                fs::write(dir.path().join(path), "")
                    .unwrap_or_else(|e| panic!("failed to write {path}: {e}"));
            }
            run_git(dir.path(), &["add", "-A"]);
            run_git(dir.path(), &["commit", "-m", "base"]);
            for (path, content) in files {
                fs::write(dir.path().join(path), content)
                    .unwrap_or_else(|e| panic!("failed to update {path}: {e}"));
            }
            Self { dir }
        }

        fn path(&self) -> &Path {
            self.dir.path()
        }
    }

    fn run_git(dir: &Path, args: &[&str]) {
        let output = std::process::Command::new("git")
            .args([
                "-c",
                "commit.gpgsign=false",
                "-c",
                "init.defaultRefFormat=files",
            ])
            .args(args)
            .current_dir(dir)
            .output()
            .unwrap_or_else(|e| panic!("failed to run git {args:?}: {e}"));
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    /// Compares two parses of the same working tree on the fields the diff-watch
    /// gate (`App::fetch_changed_diff_files`) keys on: path, status, binary and
    /// too-large flags, and content hash. Sorted so the comparison does not depend
    /// on backend return order.
    fn assert_files_match(highlighted: &[DiffFile], plain: &[DiffFile], context: &str) {
        // Two empty parses match, so the comparison below passes if the fixture
        // stops producing a diff.
        assert_eq!(
            highlighted.len(),
            2,
            "{context}: fixture should produce both changed files"
        );

        assert_eq!(
            highlighted.len(),
            plain.len(),
            "{context}: file count differs between highlighted and plain parses"
        );

        let key = |f: &DiffFile| {
            (
                f.display_path().clone(),
                f.status.as_char(),
                f.is_binary,
                f.is_too_large,
                f.content_hash,
            )
        };
        let mut highlighted_keys: Vec<_> = highlighted.iter().map(key).collect();
        let mut plain_keys: Vec<_> = plain.iter().map(key).collect();
        highlighted_keys.sort();
        plain_keys.sort();

        assert_eq!(
            highlighted_keys, plain_keys,
            "{context}: plain and highlighted parses must match"
        );
    }

    /// The gate in `App::fetch_changed_diff_files` (`src/app/diff_load.rs`) is sound
    /// only because parsing without a syntax set produces the same per-file result as
    /// parsing with one. Highlighting assigns spans; `content_hash` comes from line
    /// text at parse time. If that stops being true the watcher breaks silently, so
    /// pin it here.
    ///
    /// The `.vue` file is deliberate. Extensions accepted by
    /// `needs_full_file_highlight` are the only ones where highlighting does extra
    /// work after parsing, via `enhance_with_full_file_highlight`. Without one, the
    /// test never exercises the path most likely to break the equality.
    ///
    /// Runs against both Git backends: `content_hash` is computed separately from
    /// styling at each backend's own parse site, so the equality is a property of
    /// each backend, not a shared guarantee. Mercurial and Jujutsu are out of scope.
    /// Exercising them needs those CLIs installed, which this test suite does not
    /// assume.
    #[test]
    fn should_match_plain_and_highlighted_parses_for_both_git_backends() {
        let repo = TempRepo::with_changes(&[
            ("a.rs", "fn main() { let x = 1; }\n"),
            ("b.vue", "<template><div>{{ x }}</div></template>\n"),
        ]);

        let backends: Vec<(&str, Box<dyn VcsBackend>)> = vec![
            (
                "libgit2",
                Box::new(
                    Libgit2Backend::discover_from(repo.path(), DiffWhitespaceMode::Normal)
                        .expect("failed to open libgit2 backend"),
                ),
            ),
            (
                "git cli",
                Box::new(
                    GitCliBackend::discover_from(repo.path(), DiffWhitespaceMode::Normal)
                        .expect("failed to open git cli backend"),
                ),
            ),
        ];

        for (label, backend) in backends {
            let highlighted = backend
                .get_working_tree_diff(&SyntaxHighlighter::default())
                .unwrap_or_else(|e| panic!("{label}: highlighted fetch failed: {e}"));
            let plain = backend
                .get_working_tree_diff(&SyntaxHighlighter::plain())
                .unwrap_or_else(|e| panic!("{label}: plain fetch failed: {e}"));

            assert_files_match(&highlighted, &plain, label);
        }
    }

    /// Before/after source for a real edit: one hunk with a deletion and an
    /// addition (`alpha`), one hunk that is untouched context (`beta`), one
    /// hunk with only an addition against an empty-bodied function (`gamma`).
    fn characterization_fixture() -> (&'static str, &'static str) {
        let before = "use std::collections::HashMap;\n\
                      \n\
                      pub fn alpha(x: u32) -> u32 {\n\
                      \x20   x + 1\n\
                      }\n\
                      \n\
                      pub fn beta(s: &str) -> String {\n\
                      \x20   s.to_uppercase()\n\
                      }\n\
                      \n\
                      pub fn gamma() -> bool {\n\
                      \x20   true\n\
                      }\n";
        let after = "use std::collections::HashMap;\n\
                     \n\
                     pub fn alpha(x: u32) -> u32 {\n\
                     \x20   let total = x * 2;\n\
                     \x20   total\n\
                     }\n\
                     \n\
                     pub fn beta(s: &str) -> String {\n\
                     \x20   s.to_uppercase()\n\
                     }\n\
                     \n\
                     pub fn gamma() -> bool {\n\
                     \x20   let m: HashMap<u8, u8> = HashMap::new();\n\
                     \x20   m.is_empty()\n\
                     }\n";
        (before, after)
    }

    /// Commits `before` then rewrites the file to `after`, unstaged, so the
    /// working-tree diff carries a real deletion alongside the addition.
    /// `TempRepo::with_changes` above always starts from an empty file and so
    /// never exercises the old-side highlighting path.
    fn repo_with_edit(name: &str, before: &str, after: &str) -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("failed to create temp dir");
        run_git(dir.path(), &["init"]);
        run_git(dir.path(), &["config", "user.name", "Tuicr Test"]);
        run_git(dir.path(), &["config", "user.email", "tuicr@example.com"]);
        fs::write(dir.path().join(name), before).expect("write before");
        run_git(dir.path(), &["add", "-A"]);
        run_git(dir.path(), &["commit", "-m", "base"]);
        fs::write(dir.path().join(name), after).expect("write after");
        dir
    }

    fn signature_of(files: &[DiffFile], origin: LineOrigin, content: &str) -> String {
        files
            .iter()
            .flat_map(|f| f.hunks.iter())
            .flat_map(|h| h.lines.iter())
            .find(|l| l.origin == origin && l.content == content)
            .map(crate::model::diff_types::span_signature)
            .unwrap_or_else(|| panic!("no {origin:?} line with content {content:?} in the diff"))
    }

    /// Characterization: the colors `color_hunk` produces for a hunk built by the
    /// libgit2 backend. Rationale on `should_preserve_shared_parser_hunk_colors` in
    /// `vcs::diff_parser`.
    #[test]
    fn should_preserve_libgit2_hunk_colors() {
        let (before, after) = characterization_fixture();
        let dir = repo_with_edit("sample.rs", before, after);
        let backend = Libgit2Backend::discover_from(dir.path(), DiffWhitespaceMode::Normal)
            .expect("open repo");

        let highlighter = SyntaxHighlighter::default();
        let mut files = backend.get_working_tree_diff(&highlighter).expect("diff");
        crate::vcs::color_all_hunks(&mut files, &highlighter);

        let hunks: usize = files.iter().map(|f| f.hunks.len()).sum();
        assert_eq!(
            hunks, 2,
            "fixture must produce two hunks so the parser's per-hunk loop runs more than \
             once; got {hunks}"
        );

        // A deletion reads the old-side sequence, and only deletions do. Its
        // background is the delete tint; `1` is colored as a numeric literal.
        let deletion = signature_of(&files, LineOrigin::Deletion, "    x + 1");
        assert_ne!(
            deletion, "<uncolored>",
            "fixture must resolve a grammar, not just pin the absence of one"
        );
        assert_eq!(
            deletion,
            r#""    x "[Rgb(211, 208, 200)/Rgb(45, 0, 0)] "+"[Rgb(211, 208, 200)/Rgb(45, 0, 0)] " "[Rgb(211, 208, 200)/Rgb(45, 0, 0)] "1"[Rgb(249, 145, 87)/Rgb(45, 0, 0)]"#
        );

        // An addition reads the new-side sequence and carries the add tint.
        assert_eq!(
            signature_of(&files, LineOrigin::Addition, "    total"),
            r#""    total"[Rgb(211, 208, 200)/Rgb(0, 35, 12)]"#
        );

        // A context line: keywords, the function name, and the parameter each in their
        // own color, so one span of drift in tokenization fails this assertion.
        let context = signature_of(
            &files,
            LineOrigin::Context,
            "pub fn beta(s: &str) -> String {",
        );
        assert_ne!(
            context, "<uncolored>",
            "fixture must resolve a grammar, not just pin the absence of one"
        );
        assert_eq!(
            context,
            r#""pub"[Rgb(204, 153, 204)/-] " "[Rgb(211, 208, 200)/-] "fn"[Rgb(204, 153, 204)/-] " "[Rgb(211, 208, 200)/-] "beta"[Rgb(102, 153, 204)/-] "("[Rgb(211, 208, 200)/-] "s"[Rgb(242, 119, 122)/-] ":"[Rgb(211, 208, 200)/-] " "[Rgb(211, 208, 200)/-] "&"[Rgb(211, 208, 200)/-] "str"[Rgb(204, 153, 204)/-] ")"[Rgb(211, 208, 200)/-] " "[Rgb(211, 208, 200)/-] "->"[Rgb(211, 208, 200)/-] " String"[Rgb(211, 208, 200)/-] " "[Rgb(211, 208, 200)/-] "{"[Rgb(211, 208, 200)/-]"#
        );
    }
}
