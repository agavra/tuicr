use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::process::Command;

use tempfile::tempdir;

use crate::app::*;
use crate::model::StageState;
use crate::vcs::GitBackendPreference;
use crate::vcs::git::GitBackend;
use crate::vcs::traits::{DiffWhitespaceMode, VcsType};

fn git(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .args([
            "-c",
            "user.name=Stage test",
            "-c",
            "user.email=stage@example.test",
        ])
        .args(["-c", "core.autocrlf=false", "-c", "commit.gpgsign=false"])
        .args(args)
        .current_dir(dir)
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?}");
}

fn fixture() -> tempfile::TempDir {
    let dir = tempdir().unwrap();
    let root = dir.path();
    git(root, &["init", "-q"]);
    fs::write(root.join("mixed.txt"), "a\nb\nc\nd\n").unwrap();
    fs::write(root.join("staged.txt"), "old\n").unwrap();
    fs::write(root.join("dropped.txt"), "keep\ndrop\n").unwrap();
    git(root, &["add", "."]);
    git(root, &["commit", "-q", "-m", "base"]);

    // b -> B staged, then E appended in the worktree only.
    fs::write(root.join("mixed.txt"), "a\nB\nc\nd\n").unwrap();
    fs::write(root.join("staged.txt"), "new\n").unwrap();
    git(root, &["add", "mixed.txt", "staged.txt"]);
    fs::write(root.join("mixed.txt"), "a\nB\nc\nd\nE\n").unwrap();
    // A removal only in the worktree, and an untracked file.
    fs::write(root.join("dropped.txt"), "keep\n").unwrap();
    fs::write(root.join("untracked.txt"), "x\n").unwrap();
    dir
}

type Marks = BTreeMap<String, Vec<(char, String, Option<StageState>)>>;

fn marks(files: &[DiffFile]) -> Marks {
    files
        .iter()
        .map(|file| {
            let lines = file
                .hunks
                .iter()
                .flat_map(|hunk| &hunk.lines)
                .filter(|line| line.origin != LineOrigin::Context)
                .map(|line| {
                    let sign = if line.origin == LineOrigin::Addition {
                        '+'
                    } else {
                        '-'
                    };
                    (sign, line.content.trim_end().to_string(), line.stage)
                })
                .collect();
            (file.display_path().to_string_lossy().into_owned(), lines)
        })
        .collect()
}

fn load(root: &Path, preference: GitBackendPreference) -> Vec<DiffFile> {
    let vcs = GitBackend::discover_from(root, preference, DiffWhitespaceMode::Normal).unwrap();
    App::get_working_tree_diff_with_ignore(&vcs, root, &SyntaxHighlighter::default(), None).unwrap()
}

#[test]
fn should_mark_staged_and_unstaged_lines_the_same_on_both_git_backends() {
    use StageState::{Staged as S, Unstaged as U};
    let dir = fixture();
    let expected: Marks = [
        (
            "mixed.txt",
            vec![
                ('-', "b", Some(S)),
                ('+', "B", Some(S)),
                ('+', "E", Some(U)),
            ],
        ),
        (
            "staged.txt",
            vec![('-', "old", Some(S)), ('+', "new", Some(S))],
        ),
        ("dropped.txt", vec![('-', "drop", Some(U))]),
        ("untracked.txt", vec![('+', "x", Some(U))]),
    ]
    .into_iter()
    .map(|(path, lines)| {
        let lines = lines
            .into_iter()
            .map(|(sign, text, stage)| (sign, text.to_string(), stage))
            .collect();
        (path.to_string(), lines)
    })
    .collect();

    for preference in [GitBackendPreference::Libgit2, GitBackendPreference::Cli] {
        assert_eq!(
            marks(&load(dir.path(), preference)),
            expected,
            "{preference:?}"
        );
    }
}

#[test]
fn should_move_the_marks_when_the_rest_is_staged() {
    let dir = fixture();
    git(dir.path(), &["add", "-A"]);
    for preference in [GitBackendPreference::Libgit2, GitBackendPreference::Cli] {
        let files = load(dir.path(), preference);
        let all_staged = marks(&files)
            .values()
            .flatten()
            .all(|(_, _, stage)| *stage == Some(StageState::Staged));
        assert!(all_staged, "{preference:?}: {:#?}", marks(&files));
    }
}

/// Stands in for jj and Mercurial, which have no index.
struct NoIndexVcs {
    info: VcsInfo,
    files: Vec<DiffFile>,
}

impl VcsBackend for NoIndexVcs {
    fn info(&self) -> &VcsInfo {
        &self.info
    }

    fn get_working_tree_diff(&self, _highlighter: &SyntaxHighlighter) -> Result<Vec<DiffFile>> {
        Ok(self.files.clone())
    }

    fn fetch_context_lines(
        &self,
        _file_path: &Path,
        _file_status: FileStatus,
        _ref_commit: Option<&str>,
        _start_line: u32,
        _end_line: u32,
    ) -> Result<Vec<DiffLine>> {
        Ok(Vec::new())
    }

    fn file_line_count(
        &self,
        _file_path: &Path,
        _file_status: FileStatus,
        _ref_commit: Option<&str>,
    ) -> Result<u32> {
        Ok(0)
    }
}

#[test]
fn should_leave_lines_unmarked_on_backends_without_an_index() {
    let dir = fixture();
    let files = load(dir.path(), GitBackendPreference::Libgit2);
    let vcs = NoIndexVcs {
        info: VcsInfo {
            root_path: dir.path().to_path_buf(),
            head_commit: "HEAD".to_string(),
            branch_name: None,
            vcs_type: VcsType::Jujutsu,
        },
        files: files
            .into_iter()
            .map(|mut file| {
                for hunk in &mut file.hunks {
                    for line in &mut hunk.lines {
                        line.stage = None;
                    }
                }
                file
            })
            .collect(),
    };
    let loaded = App::get_working_tree_diff_with_ignore(
        &vcs,
        dir.path(),
        &SyntaxHighlighter::default(),
        None,
    )
    .unwrap();
    assert!(!loaded.is_empty());
    assert!(
        marks(&loaded)
            .values()
            .flatten()
            .all(|(_, _, stage)| stage.is_none())
    );
}
