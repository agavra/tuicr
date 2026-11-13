//! Marks changed lines of the combined `HEAD -> worktree` diff as staged or
//! unstaged, using two more diffs that each share one side with it:
//!
//! - A removed line is staged if the staged diff (`HEAD -> index`) also
//!   removes that `HEAD` line.
//! - An added line is unstaged if the unstaged diff (`index -> worktree`) also
//!   adds that worktree line.
//!
//! Lines match by number on the shared side, so no content is compared.
//! Files match by path.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use crate::model::{DiffFile, LineOrigin, StageState};

/// Line numbers a diff changes in `file`. `None` for binary and too-large
/// files, which have no lines to read.
fn changed_lines(file: &DiffFile, origin: LineOrigin) -> Option<HashSet<u32>> {
    if file.is_binary || file.is_too_large {
        return None;
    }
    let lines = file.hunks.iter().flat_map(|hunk| &hunk.lines);
    Some(
        lines
            .filter(|line| line.origin == origin)
            .filter_map(|line| match origin {
                LineOrigin::Deletion => line.old_lineno,
                _ => line.new_lineno,
            })
            .collect(),
    )
}

/// Lines of `file` that `side` also changes: removals for the staged diff,
/// additions for the unstaged diff.
fn changed_by(
    side: &HashMap<&Path, &DiffFile>,
    file: &DiffFile,
    origin: LineOrigin,
) -> Option<HashSet<u32>> {
    let (path, paired, other_end): (_, _, fn(&DiffFile) -> Option<&Path>) = match origin {
        LineOrigin::Deletion => (&file.old_path, &file.new_path, |f| f.new_path.as_deref()),
        _ => (&file.new_path, &file.old_path, |f| f.old_path.as_deref()),
    };
    let Some(entry) = path.as_deref().and_then(|p| side.get(p)) else {
        return Some(HashSet::new());
    };
    let end = other_end(entry);
    if end == paired.as_deref() || end == path.as_deref() {
        changed_lines(entry, origin)
    } else {
        // The side moved or dropped the path, so all of these lines changed there.
        changed_lines(file, origin)
    }
}

fn mark(
    changed: &Option<HashSet<u32>>,
    n: Option<u32>,
    hit: StageState,
    miss: StageState,
) -> Option<StageState> {
    Some(if changed.as_ref()?.contains(&n?) {
        hit
    } else {
        miss
    })
}

/// Sets `DiffLine::stage` on every added and removed line of `combined`.
pub(crate) fn apply_stage_marks(
    combined: &mut [DiffFile],
    staged: &[DiffFile],
    unstaged: &[DiffFile],
) {
    let staged: HashMap<_, _> = staged
        .iter()
        .filter_map(|f| Some((f.old_path.as_deref()?, f)))
        .collect();
    let unstaged: HashMap<_, _> = unstaged
        .iter()
        .filter_map(|f| Some((f.new_path.as_deref()?, f)))
        .collect();

    for file in combined.iter_mut().filter(|f| !f.is_commit_message) {
        let removed = changed_by(&staged, file, LineOrigin::Deletion);
        let added = changed_by(&unstaged, file, LineOrigin::Addition);
        for line in file.hunks.iter_mut().flat_map(|hunk| hunk.lines.iter_mut()) {
            line.stage = match line.origin {
                LineOrigin::Deletion => mark(
                    &removed,
                    line.old_lineno,
                    StageState::Staged,
                    StageState::Unstaged,
                ),
                LineOrigin::Addition => mark(
                    &added,
                    line.new_lineno,
                    StageState::Unstaged,
                    StageState::Staged,
                ),
                LineOrigin::Context => None,
            };
        }
    }
}

/// Hash of every line's stage. The diff watcher needs it because staging
/// changes the marks but not the diff's content.
pub(crate) fn stage_marks_hash(file: &DiffFile) -> u64 {
    let mut hasher = crate::hash::Fnv1aHasher::new();
    for line in file.hunks.iter().flat_map(|hunk| &hunk.lines) {
        hasher.write(&[line.stage.map_or(0, |stage| stage as u8 + 1)]);
    }
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{DiffHunk, DiffLine, FileStatus};
    use std::path::PathBuf;

    fn line(origin: LineOrigin, old: Option<u32>, new: Option<u32>) -> DiffLine {
        DiffLine {
            origin,
            content: String::new(),
            raw_content: None,
            old_lineno: old,
            new_lineno: new,
            highlighted_spans: None,
            stage: None,
        }
    }
    fn add(n: u32) -> DiffLine {
        line(LineOrigin::Addition, None, Some(n))
    }
    fn del(n: u32) -> DiffLine {
        line(LineOrigin::Deletion, Some(n), None)
    }
    fn ctx(o: u32, n: u32) -> DiffLine {
        line(LineOrigin::Context, Some(o), Some(n))
    }

    fn file(old: Option<&str>, new: Option<&str>, lines: Vec<DiffLine>) -> DiffFile {
        let status = match (old, new) {
            (None, Some(_)) => FileStatus::Added,
            (Some(_), None) => FileStatus::Deleted,
            (Some(o), Some(n)) if o != n => FileStatus::Renamed,
            _ => FileStatus::Modified,
        };
        DiffFile {
            old_path: old.map(PathBuf::from),
            new_path: new.map(PathBuf::from),
            status,
            hunks: vec![DiffHunk {
                header: String::new(),
                lines,
                old_start: 1,
                old_count: 0,
                new_start: 1,
                new_count: 0,
            }],
            is_binary: false,
            is_too_large: false,
            is_commit_message: false,
            content_hash: 0,
        }
    }

    fn stages(file: &DiffFile) -> Vec<Option<StageState>> {
        file.hunks[0].lines.iter().map(|l| l.stage).collect()
    }

    use StageState::{Staged as S, Unstaged as U};

    #[test]
    fn should_mark_a_hunk_mixing_staged_and_unstaged_lines() {
        // HEAD: a b c. Index: a B c (b->B staged). Worktree: a B c D (D unstaged).
        let mut combined = vec![file(
            Some("f"),
            Some("f"),
            vec![ctx(1, 1), del(2), add(2), ctx(3, 3), add(4)],
        )];
        let staged = vec![file(Some("f"), Some("f"), vec![ctx(1, 1), del(2), add(2)])];
        let unstaged = vec![file(Some("f"), Some("f"), vec![ctx(3, 3), add(4)])];
        apply_stage_marks(&mut combined, &staged, &unstaged);
        assert_eq!(
            stages(&combined[0]),
            vec![None, Some(S), Some(S), None, Some(U)]
        );
    }

    #[test]
    fn should_mark_a_staged_line_edited_again_as_unstaged() {
        // HEAD: a. Index: b. Worktree: c.
        let mut combined = vec![file(Some("f"), Some("f"), vec![del(1), add(1)])];
        let staged = vec![file(Some("f"), Some("f"), vec![del(1), add(1)])];
        let unstaged = vec![file(Some("f"), Some("f"), vec![del(1), add(1)])];
        apply_stage_marks(&mut combined, &staged, &unstaged);
        assert_eq!(stages(&combined[0]), vec![Some(S), Some(U)]);
    }

    #[test]
    fn should_mark_a_removal_only_in_the_worktree_as_unstaged() {
        let mut combined = vec![file(Some("f"), Some("f"), vec![ctx(1, 1), del(2)])];
        let unstaged = vec![file(Some("f"), Some("f"), vec![ctx(1, 1), del(2)])];
        apply_stage_marks(&mut combined, &[], &unstaged);
        assert_eq!(stages(&combined[0]), vec![None, Some(U)]);
    }

    #[test]
    fn should_mark_whole_files_staged_or_unstaged() {
        let mut combined = vec![
            file(Some("staged"), Some("staged"), vec![del(1), add(1)]),
            file(Some("unstaged"), Some("unstaged"), vec![del(1), add(1)]),
        ];
        let staged = vec![file(Some("staged"), Some("staged"), vec![del(1), add(1)])];
        let unstaged = vec![file(
            Some("unstaged"),
            Some("unstaged"),
            vec![del(1), add(1)],
        )];
        apply_stage_marks(&mut combined, &staged, &unstaged);
        assert_eq!(stages(&combined[0]), vec![Some(S), Some(S)]);
        assert_eq!(stages(&combined[1]), vec![Some(U), Some(U)]);
    }

    #[test]
    fn should_mark_untracked_files_unstaged() {
        let mut combined = vec![file(None, Some("new"), vec![add(1), add(2)])];
        let unstaged = vec![file(None, Some("new"), vec![add(1), add(2)])];
        apply_stage_marks(&mut combined, &[], &unstaged);
        assert_eq!(stages(&combined[0]), vec![Some(U), Some(U)]);
    }

    #[test]
    fn should_mark_a_staged_rename_by_its_old_and_new_paths() {
        // `git mv old new`, then one more unstaged line in `new`.
        let mut combined = vec![file(Some("old"), Some("new"), vec![del(1), add(1), add(2)])];
        let staged = vec![file(Some("old"), Some("new"), vec![del(1), add(1)])];
        let unstaged = vec![file(Some("new"), Some("new"), vec![add(2)])];
        apply_stage_marks(&mut combined, &staged, &unstaged);
        assert_eq!(stages(&combined[0]), vec![Some(S), Some(S), Some(U)]);
    }

    #[test]
    fn should_mark_all_removals_staged_when_the_index_moved_the_old_path() {
        // The combined diff sees `old` deleted and `new` added; the staged
        // diff pairs them as a rename with one changed line.
        let mut combined = vec![
            file(Some("old"), None, vec![del(1), del(2)]),
            file(None, Some("new"), vec![add(1), add(2)]),
        ];
        let staged = vec![file(Some("old"), Some("new"), vec![del(2), add(2)])];
        let unstaged = vec![file(Some("new"), Some("new"), vec![del(1), add(1)])];
        apply_stage_marks(&mut combined, &staged, &unstaged);
        assert_eq!(stages(&combined[0]), vec![Some(S), Some(S)]);
        assert_eq!(stages(&combined[1]), vec![Some(U), Some(S)]);
    }

    #[test]
    fn should_mark_all_additions_unstaged_when_the_index_lacks_the_new_path() {
        // The unstaged diff pairs a worktree move as a rename; the combined
        // diff shows `new` as an added file.
        let mut combined = vec![file(None, Some("new"), vec![add(1), add(2)])];
        let unstaged = vec![file(Some("old"), Some("new"), vec![del(2), add(2)])];
        apply_stage_marks(&mut combined, &[], &unstaged);
        assert_eq!(stages(&combined[0]), vec![Some(U), Some(U)]);
    }

    #[test]
    fn should_leave_lines_unmarked_when_the_deciding_file_has_no_hunks() {
        let mut combined = vec![file(Some("f"), Some("f"), vec![del(1), add(1)])];
        let mut too_large = file(Some("f"), Some("f"), vec![]);
        too_large.is_too_large = true;
        apply_stage_marks(&mut combined, &[], std::slice::from_ref(&too_large));
        assert_eq!(stages(&combined[0]), vec![Some(U), None]);
    }

    #[test]
    fn should_change_the_marks_hash_when_a_line_is_staged() {
        let mut combined = vec![file(Some("f"), Some("f"), vec![add(1)])];
        let unstaged = vec![file(Some("f"), Some("f"), vec![add(1)])];
        apply_stage_marks(&mut combined, &[], &unstaged);
        let before = stage_marks_hash(&combined[0]);
        apply_stage_marks(&mut combined, &[], &[]);
        assert_ne!(before, stage_marks_hash(&combined[0]));
    }
}
