use git2::{Delta, Diff, DiffOptions, Repository};
use std::path::{Path, PathBuf};

use crate::error::{Result, TuicrError};
use crate::model::{DiffFile, DiffHunk, DiffLine, FileStatus, LineOrigin};
use crate::syntax::{SyntaxHighlighter, needs_full_file_highlight};
use crate::vcs::{enhance_with_full_file_highlight, tabify};

pub fn get_working_tree_diff(
    repo: &Repository,
    highlighter: &SyntaxHighlighter,
) -> Result<Vec<DiffFile>> {
    let head = repo.head()?.peel_to_tree()?;

    let mut opts = DiffOptions::new();
    opts.include_untracked(true);
    opts.show_untracked_content(true);
    opts.recurse_untracked_dirs(true);

    let diff = repo.diff_tree_to_workdir_with_index(Some(&head), Some(&mut opts))?;
    let mut files = parse_diff(&diff, highlighter)?;
    enhance_with_full_file_highlight(
        &mut files,
        highlighter,
        |path| read_path_from_tree(repo, &head, path),
        |path| read_path_from_workdir(repo, path),
    );
    Ok(files)
}

/// Get the staged diff (index vs HEAD)
/// On repos with no commits (unborn HEAD), diffs against an empty tree.
pub fn get_staged_diff(
    repo: &Repository,
    highlighter: &SyntaxHighlighter,
) -> Result<Vec<DiffFile>> {
    let head = repo.head().ok().and_then(|h| h.peel_to_tree().ok());
    let index = repo.index()?;
    let diff = repo.diff_tree_to_index(head.as_ref(), Some(&index), None)?;
    let mut files = parse_diff(&diff, highlighter)?;
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
pub fn get_unstaged_diff(
    repo: &Repository,
    highlighter: &SyntaxHighlighter,
) -> Result<Vec<DiffFile>> {
    let index = repo.index()?;
    let mut opts = DiffOptions::new();
    opts.include_untracked(true);
    opts.show_untracked_content(true);
    opts.recurse_untracked_dirs(true);

    let diff = repo.diff_index_to_workdir(Some(&index), Some(&mut opts))?;
    let mut files = parse_diff(&diff, highlighter)?;
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
    commit_ids: &[String],
    highlighter: &SyntaxHighlighter,
) -> Result<Vec<DiffFile>> {
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

    let new_tree = newest_commit.tree()?;

    let diff = repo.diff_tree_to_tree(old_tree.as_ref(), Some(&new_tree), None)?;
    let mut files = parse_diff(&diff, highlighter)?;
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

    let mut opts = DiffOptions::new();
    opts.include_untracked(true);
    opts.show_untracked_content(true);
    opts.recurse_untracked_dirs(true);

    let diff = repo.diff_tree_to_workdir_with_index(old_tree.as_ref(), Some(&mut opts))?;
    let mut files = parse_diff(&diff, highlighter)?;
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

fn parse_diff(diff: &Diff, highlighter: &SyntaxHighlighter) -> Result<Vec<DiffFile>> {
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

        let syntax_path = new_path.as_ref().or(old_path.as_ref()).map(|p| p.as_path());
        let hunks = if is_binary || is_too_large {
            Vec::new()
        } else {
            parse_hunks(diff, delta_idx, highlighter, syntax_path)?
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

fn parse_hunks(
    diff: &Diff,
    delta_idx: usize,
    highlighter: &SyntaxHighlighter,
    file_path: Option<&Path>,
) -> Result<Vec<DiffHunk>> {
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

            let sequences =
                SyntaxHighlighter::split_diff_lines_for_highlighting(&line_contents, &line_origins);
            // Container grammars skip per-hunk highlighting; the full-file
            // post-pass overwrites these spans anyway.
            let (old_highlighted, new_highlighted) = match file_path {
                Some(path) if !needs_full_file_highlight(path) => (
                    highlighter.highlight_file_lines(path, &sequences.old_lines),
                    highlighter.highlight_file_lines(path, &sequences.new_lines),
                ),
                _ => (None, None),
            };

            let mut lines: Vec<DiffLine> = Vec::with_capacity(line_contents.len());
            for (idx, content) in line_contents.into_iter().enumerate() {
                let origin = line_origins[idx];
                let (old_lineno, new_lineno) = line_numbers[idx];

                let highlighted_spans = highlighter.highlighted_line_for_diff_with_background(
                    old_highlighted.as_deref(),
                    new_highlighted.as_deref(),
                    sequences.old_line_indices[idx],
                    sequences.new_line_indices[idx],
                    origin,
                );

                lines.push(DiffLine {
                    origin,
                    content,
                    old_lineno,
                    new_lineno,
                    highlighted_spans,
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
        let repo = Repository::discover(".").unwrap();
        let head = repo.head().unwrap().peel_to_tree().unwrap();
        let diff = repo
            .diff_tree_to_tree(Some(&head), Some(&head), None)
            .unwrap();
        let highlighter = SyntaxHighlighter::default();

        let result = parse_diff(&diff, &highlighter);

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

        let files = get_working_tree_diff(&repo, &SyntaxHighlighter::default())
            .expect("failed to get diff");

        assert_eq!(files.len(), 1);
        let lines = &files[0].hunks[0].lines;

        assert!(
            lines.iter().any(|l| l.content == "    new"),
            "expected tab-expanded content in git diff lines"
        );
        assert!(lines.iter().all(|l| !l.content.contains('\t')));
    }

    #[test]
    fn should_highlight_vue_script_hunk_using_full_file_context() {
        let temp_dir = tempfile::tempdir().expect("failed to create temp dir");
        let repo = Repository::init(temp_dir.path()).expect("failed to init repo");

        let initial = "<template>\n  <div>{{ msg }}</div>\n</template>\n\n<script setup>\nimport { ref } from 'vue'\nconst msg = ref('hi')\nconst other = 1\n</script>\n";
        create_initial_commit(&repo, "App.vue", initial);

        let edited = "<template>\n  <div>{{ msg }}</div>\n</template>\n\n<script setup>\nimport { ref } from 'vue'\nconst msg = ref('hello')\nconst other = 1\n</script>\n";
        fs::write(temp_dir.path().join("App.vue"), edited).expect("failed to update file");

        let files = get_working_tree_diff(&repo, &SyntaxHighlighter::default())
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
                .highlighted_spans
                .as_ref()
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

        let unstaged = get_unstaged_diff(&repo, &highlighter).expect("unstaged diff failed");
        assert_eq!(unstaged.len(), 1);
        assert!(matches!(
            get_staged_diff(&repo, &highlighter),
            Err(TuicrError::NoChanges)
        ));

        let mut index = repo.index().expect("failed to open index");
        index
            .add_path(Path::new("file.txt"))
            .expect("failed to add file to index");
        index.write().expect("failed to write index");

        let staged = get_staged_diff(&repo, &highlighter).expect("staged diff failed");
        assert_eq!(staged.len(), 1);
        assert!(matches!(
            get_unstaged_diff(&repo, &highlighter),
            Err(TuicrError::NoChanges)
        ));
    }
}
