use std::{fs, path::Path};

use crate::{
    model::{DiffFile, FileStatus, LineOrigin},
    syntax::SyntaxHighlighter,
    vcs::{
        git::diff,
        traits::{DiffWhitespaceMode, ResolvedRevisionRange, RevisionDiffTarget},
    },
};
use git2::{Repository, Signature};

#[derive(Clone, Copy, Debug)]
enum View {
    Staged,
    Unstaged,
    WorkingTree,
    Commits,
    CommitsAndWorkingTree,
}

const VIEWS: [View; 5] = [
    View::Staged,
    View::Unstaged,
    View::WorkingTree,
    View::Commits,
    View::CommitsAndWorkingTree,
];

fn commit(repo: &Repository) -> String {
    let mut index = repo.index().unwrap();
    let tree_id = index.write_tree().unwrap();
    let tree = repo.find_tree(tree_id).unwrap();
    let signature = Signature::now("Rename test", "rename@example.test").unwrap();
    let parent = repo.head().ok().map(|head| head.peel_to_commit().unwrap());
    repo.commit(
        Some("HEAD"),
        &signature,
        &signature,
        "fixture",
        &tree,
        &parent.iter().collect::<Vec<_>>(),
    )
    .unwrap()
    .to_string()
}

fn stage(repo: &Repository, paths: &[&str]) {
    let mut index = repo.index().unwrap();
    for path in paths {
        if repo.workdir().unwrap().join(path).exists() {
            index.add_path(Path::new(path)).unwrap();
        } else {
            index.remove_path(Path::new(path)).unwrap();
        }
    }
    index.write().unwrap();
}

fn original() -> String {
    (0..40)
        .map(|i| format!("record {i:02}: preserve this distinct payload in the moved file\n"))
        .collect()
}

fn fixture() -> (tempfile::TempDir, Repository) {
    let dir = tempfile::tempdir().unwrap();
    let repo = Repository::init(dir.path()).unwrap();
    // Local settings keep the fixture independent of the developer's Git defaults.
    repo.config()
        .unwrap()
        .set_bool("diff.renames", true)
        .unwrap();
    repo.config()
        .unwrap()
        .set_i64("diff.renameLimit", 1000)
        .unwrap();
    fs::write(dir.path().join("old.txt"), original()).unwrap();
    stage(&repo, &["old.txt"]);
    commit(&repo);
    (dir, repo)
}

fn read_view(repo: &Repository, view: View, base: &str) -> Vec<DiffFile> {
    let mode = DiffWhitespaceMode::Normal;
    let highlighter = SyntaxHighlighter::default();
    match view {
        View::Staged => diff::get_staged_diff(repo, &mode, &highlighter),
        View::Unstaged => diff::get_unstaged_diff(repo, &mode, &highlighter),
        View::WorkingTree => diff::get_working_tree_diff(repo, &mode, &highlighter),
        View::Commits => {
            let head = repo
                .head()
                .unwrap()
                .peel_to_commit()
                .unwrap()
                .id()
                .to_string();
            diff::get_commit_range_diff(
                repo,
                &ResolvedRevisionRange::from_owned_commit_ids(
                    vec![head.clone()],
                    RevisionDiffTarget::Explicit {
                        base: Some(base.into()),
                        head,
                    },
                ),
                &mode,
                &highlighter,
            )
            .map(|diff| diff.files)
        }
        View::CommitsAndWorkingTree => {
            diff::get_working_tree_with_commits_diff(repo, &[base.into()], &mode, &highlighter)
        }
    }
    .unwrap()
}

fn prepare_view(repo: &Repository, view: View, paths: &[&str]) -> String {
    let base = repo
        .head()
        .unwrap()
        .peel_to_commit()
        .unwrap()
        .id()
        .to_string();
    if matches!(view, View::Staged | View::Commits) {
        stage(repo, paths);
    }
    if matches!(view, View::Commits) {
        commit(repo);
    }
    base
}

#[test]
fn detects_exact_and_edited_renames_in_every_review_mode() {
    for view in VIEWS {
        for edited in [false, true] {
            let (dir, repo) = fixture();
            // A second baseline commit makes combined review compare against the
            // existing old file, rather than the empty tree of a root commit.
            commit(&repo);
            fs::rename(dir.path().join("old.txt"), dir.path().join("new.txt")).unwrap();
            if edited {
                fs::write(
                    dir.path().join("new.txt"),
                    original().replace("record 20:", "changed 20:"),
                )
                .unwrap();
            }
            assert!(!dir.path().join("old.txt").exists());
            assert!(
                repo.index()
                    .unwrap()
                    .get_path(Path::new("new.txt"), 0)
                    .is_none()
            );
            let base = prepare_view(&repo, view, &["old.txt", "new.txt"]);
            let index_before = fs::read(repo.path().join("index")).unwrap();
            let files = read_view(&repo, view, &base);
            assert_eq!(files.len(), 1, "{view:?}, edited={edited}");
            let file = &files[0];
            assert_eq!(
                file.status,
                FileStatus::Renamed,
                "{view:?}, edited={edited}"
            );
            assert_eq!(file.old_path.as_deref(), Some(Path::new("old.txt")));
            assert_eq!(file.new_path.as_deref(), Some(Path::new("new.txt")));
            let lines: Vec<_> = file.hunks.iter().flat_map(|h| &h.lines).collect();
            assert_eq!(
                lines
                    .iter()
                    .filter(|l| l.origin == LineOrigin::Addition)
                    .count(),
                usize::from(edited)
            );
            assert_eq!(
                lines
                    .iter()
                    .filter(|l| l.origin == LineOrigin::Deletion)
                    .count(),
                usize::from(edited)
            );
            if edited {
                assert!(lines.iter().any(|l| l.origin == LineOrigin::Context));
                assert!(
                    lines.iter().any(|l| l.origin == LineOrigin::Addition
                        && l.content.starts_with("changed 20:"))
                );
            }
            assert_eq!(fs::read(repo.path().join("index")).unwrap(), index_before);
        }
    }
}

#[test]
fn disabled_rename_detection_preserves_delete_and_add_in_every_review_mode() {
    for view in VIEWS {
        let (dir, repo) = fixture();
        commit(&repo);
        repo.config()
            .unwrap()
            .set_bool("diff.renames", false)
            .unwrap();
        fs::rename(dir.path().join("old.txt"), dir.path().join("new.txt")).unwrap();
        let base = prepare_view(&repo, view, &["old.txt", "new.txt"]);
        let files = read_view(&repo, view, &base);
        assert_eq!(files.len(), 2, "{view:?}");
        assert!(files.iter().any(|f| f.status == FileStatus::Deleted
            && f.old_path.as_deref() == Some(Path::new("old.txt"))));
        assert!(files.iter().any(|f| f.status == FileStatus::Added
            && f.new_path.as_deref() == Some(Path::new("new.txt"))));
    }
}

#[test]
fn unrelated_additions_deletions_and_edits_keep_their_statuses() {
    for view in VIEWS {
        let (dir, repo) = fixture();
        fs::write(
            dir.path().join("modified.txt"),
            "ordinary file\nunchanged context\n",
        )
        .unwrap();
        stage(&repo, &["modified.txt"]);
        commit(&repo);
        commit(&repo);
        fs::remove_file(dir.path().join("old.txt")).unwrap();
        fs::write(
            dir.path().join("added.txt"),
            "an entirely unrelated new document\n",
        )
        .unwrap();
        fs::write(
            dir.path().join("modified.txt"),
            "ordinary edit\nunchanged context\n",
        )
        .unwrap();
        let base = prepare_view(&repo, view, &["old.txt", "added.txt", "modified.txt"]);
        let files = read_view(&repo, view, &base);
        assert_eq!(files.len(), 3, "{view:?}: {files:?}");
        for (path, status) in [
            ("old.txt", FileStatus::Deleted),
            ("added.txt", FileStatus::Added),
            ("modified.txt", FileStatus::Modified),
        ] {
            assert!(
                files
                    .iter()
                    .any(|f| f.display_path() == Path::new(path) && f.status == status),
                "{view:?}: {path}"
            );
        }
    }
}

#[test]
fn copies_config_detects_a_copy_from_a_modified_source() {
    for policy in ["copy", "copies"] {
        let (dir, repo) = fixture();
        repo.config()
            .unwrap()
            .set_str("diff.renames", policy)
            .unwrap();
        fs::write(dir.path().join("copy.txt"), original()).unwrap();
        fs::write(
            dir.path().join("old.txt"),
            original().replace("record 20:", "changed 20:"),
        )
        .unwrap();
        for staged in [false, true] {
            let view = if staged { View::Staged } else { View::Unstaged };
            let base = prepare_view(&repo, view, &["old.txt", "copy.txt"]);
            let files = read_view(&repo, view, &base);
            assert_eq!(files.len(), 2);
            assert!(files.iter().any(|f| f.status == FileStatus::Copied
                && f.old_path.as_deref() == Some(Path::new("old.txt"))
                && f.new_path.as_deref() == Some(Path::new("copy.txt"))));
            assert!(files.iter().any(|f| f.status == FileStatus::Modified));
        }
    }
}

#[test]
fn rename_limit_bounds_similarity_candidates() {
    let (dir, repo) = fixture();
    fs::write(
        dir.path().join("middle.txt"),
        "unrelated amber orchard\n".repeat(40),
    )
    .unwrap();
    fs::write(
        dir.path().join("z-source.txt"),
        "distinct violet ocean\n".repeat(40),
    )
    .unwrap();
    stage(&repo, &["middle.txt", "z-source.txt"]);
    commit(&repo);
    for path in ["old.txt", "middle.txt", "z-source.txt"] {
        fs::remove_file(dir.path().join(path)).unwrap();
    }
    fs::write(
        dir.path().join("destination.txt"),
        format!("{}one edited line\n", "distinct violet ocean\n".repeat(39)),
    )
    .unwrap();
    stage(
        &repo,
        &["old.txt", "middle.txt", "z-source.txt", "destination.txt"],
    );
    let base = repo
        .head()
        .unwrap()
        .peel_to_commit()
        .unwrap()
        .id()
        .to_string();
    repo.config()
        .unwrap()
        .set_i64("diff.renameLimit", 1)
        .unwrap();
    let limited = read_view(&repo, View::Staged, &base);
    assert_eq!(limited.len(), 4);
    assert!(
        !limited
            .iter()
            .any(|file| file.status == FileStatus::Renamed)
    );
    repo.config()
        .unwrap()
        .set_i64("diff.renameLimit", 1000)
        .unwrap();
    let unlimited = read_view(&repo, View::Staged, &base);
    assert_eq!(unlimited.len(), 3);
    assert!(
        unlimited
            .iter()
            .any(|file| file.status == FileStatus::Renamed
                && file.old_path.as_deref() == Some(Path::new("z-source.txt"))
                && file.new_path.as_deref() == Some(Path::new("destination.txt")))
    );
}

#[test]
fn clean_repository_has_no_diff() {
    let (_dir, repo) = fixture();
    assert!(matches!(
        diff::get_working_tree_diff(
            &repo,
            &DiffWhitespaceMode::Normal,
            &SyntaxHighlighter::default()
        ),
        Err(crate::error::TuicrError::NoChanges)
    ));
}
