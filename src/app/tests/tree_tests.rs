use crate::app::*;
use crate::model::{DiffFile, DiffLine, FileStatus};
use crate::vcs::traits::{VcsBackend, VcsInfo, VcsType};

fn make_file(path: &str) -> DiffFile {
    DiffFile {
        old_path: None,
        new_path: Some(PathBuf::from(path)),
        status: FileStatus::Modified,
        hunks: vec![],
        is_binary: false,
        is_too_large: false,
        is_commit_message: false,
        content_hash: 0,
    }
}

struct TreeTestHarness {
    diff_files: Vec<DiffFile>,
    expanded_dirs: HashSet<String>,
}

impl TreeTestHarness {
    fn new(paths: &[&str]) -> Self {
        Self {
            diff_files: paths.iter().map(|p| make_file(p)).collect(),
            expanded_dirs: HashSet::new(),
        }
    }

    fn expand_all(&mut self) {
        use std::path::Path;
        for file in &self.diff_files {
            let path = file.display_path();
            let mut current = path.parent();
            while let Some(parent) = current {
                if parent != Path::new("") {
                    self.expanded_dirs
                        .insert(parent.to_string_lossy().to_string());
                }
                current = parent.parent();
            }
        }
    }

    fn collapse_all(&mut self) {
        self.expanded_dirs.clear();
    }

    fn toggle(&mut self, dir: &str) {
        if self.expanded_dirs.contains(dir) {
            self.expanded_dirs.remove(dir);
        } else {
            self.expanded_dirs.insert(dir.to_string());
        }
    }

    fn build_visible_items(&self) -> Vec<FileTreeItem> {
        use std::path::Path;
        let mut items = Vec::new();
        let mut seen_dirs: HashSet<String> = HashSet::new();

        for (file_idx, file) in self.diff_files.iter().enumerate() {
            let path = file.display_path();
            let mut ancestors: Vec<String> = Vec::new();
            let mut current = path.parent();
            while let Some(parent) = current {
                if parent != Path::new("") {
                    ancestors.push(parent.to_string_lossy().to_string());
                }
                current = parent.parent();
            }
            ancestors.reverse();

            let mut visible = true;
            for (depth, dir) in ancestors.iter().enumerate() {
                if !seen_dirs.contains(dir) && visible {
                    let expanded = self.expanded_dirs.contains(dir);
                    items.push(FileTreeItem::Directory {
                        path: dir.clone(),
                        label: Path::new(dir)
                            .file_name()
                            .unwrap()
                            .to_string_lossy()
                            .into_owned(),
                        depth,
                        expanded,
                    });
                    seen_dirs.insert(dir.clone());
                }
                if !self.expanded_dirs.contains(dir) {
                    visible = false;
                }
            }

            if visible {
                items.push(FileTreeItem::File {
                    file_idx,
                    depth: ancestors.len(),
                });
            }
        }
        items
    }

    fn visible_file_count(&self) -> usize {
        self.build_visible_items()
            .iter()
            .filter(|i| matches!(i, FileTreeItem::File { .. }))
            .count()
    }

    fn visible_dir_count(&self) -> usize {
        self.build_visible_items()
            .iter()
            .filter(|i| matches!(i, FileTreeItem::Directory { .. }))
            .count()
    }
}

#[test]
fn test_expand_all_shows_all_files() {
    let mut h = TreeTestHarness::new(&["src/ui/app.rs", "src/ui/help.rs", "src/main.rs"]);
    h.expand_all();

    assert_eq!(h.visible_file_count(), 3);
}

#[test]
fn test_collapse_all_hides_all_files() {
    let mut h = TreeTestHarness::new(&["src/ui/app.rs", "src/main.rs"]);
    h.expand_all();
    h.collapse_all();

    assert_eq!(h.visible_file_count(), 0);
    assert_eq!(h.visible_dir_count(), 1); // only "src" visible
}

#[test]
fn test_collapse_parent_hides_nested_dirs() {
    let mut h = TreeTestHarness::new(&["src/ui/components/button.rs"]);
    h.expand_all();
    assert_eq!(h.visible_dir_count(), 3); // src, src/ui, src/ui/components

    h.toggle("src");
    let items = h.build_visible_items();
    assert_eq!(items.len(), 1); // only collapsed "src" dir
    assert!(matches!(
        &items[0],
        FileTreeItem::Directory {
            expanded: false,
            ..
        }
    ));
}

#[test]
fn test_root_files_always_visible() {
    let mut h = TreeTestHarness::new(&["README.md", "Cargo.toml"]);
    h.collapse_all();

    assert_eq!(h.visible_file_count(), 2);
}

#[test]
fn test_tree_depth_correct() {
    let mut h = TreeTestHarness::new(&["a/b/c/file.rs"]);
    h.expand_all();

    let items = h.build_visible_items();
    assert!(matches!(&items[0], FileTreeItem::Directory { depth: 0, path, .. } if path == "a"));
    assert!(matches!(&items[1], FileTreeItem::Directory { depth: 1, path, .. } if path == "a/b"));
    assert!(matches!(&items[2], FileTreeItem::Directory { depth: 2, path, .. } if path == "a/b/c"));
    assert!(matches!(&items[3], FileTreeItem::File { depth: 3, .. }));
}

#[test]
fn test_toggle_expands_collapsed_dir() {
    let mut h = TreeTestHarness::new(&["src/main.rs"]);
    h.collapse_all();
    assert_eq!(h.visible_file_count(), 0);

    h.toggle("src");
    assert_eq!(h.visible_file_count(), 1);
}

#[test]
fn test_sibling_dirs_independent() {
    let mut h = TreeTestHarness::new(&["src/app.rs", "tests/test.rs"]);
    h.expand_all();
    h.toggle("src"); // collapse src

    assert_eq!(h.visible_file_count(), 1); // only tests/test.rs
}

struct StubVcs(VcsInfo);
impl VcsBackend for StubVcs {
    fn info(&self) -> &VcsInfo {
        &self.0
    }
    fn get_working_tree_diff(
        &self,
        _hl: &crate::syntax::SyntaxHighlighter,
    ) -> crate::error::Result<Vec<DiffFile>> {
        Ok(Vec::new())
    }
    fn fetch_context_lines(
        &self,
        _path: &std::path::Path,
        _status: FileStatus,
        _ref_commit: Option<&str>,
        _start: u32,
        _end: u32,
    ) -> crate::error::Result<Vec<DiffLine>> {
        Ok(Vec::new())
    }
    fn file_line_count(
        &self,
        _path: &std::path::Path,
        _status: FileStatus,
        _ref_commit: Option<&str>,
    ) -> crate::error::Result<u32> {
        Ok(0)
    }
}

fn app_with(paths: &[&str]) -> App {
    let vcs_info = VcsInfo {
        root_path: PathBuf::from("/tmp"),
        head_commit: "head".into(),
        branch_name: Some("main".into()),
        vcs_type: VcsType::Git,
    };
    let session = ReviewSession::new(
        vcs_info.root_path.clone(),
        vcs_info.head_commit.clone(),
        vcs_info.branch_name.clone(),
        SessionDiffSource::WorkingTree,
    );
    App::build(
        Box::new(StubVcs(vcs_info.clone())),
        vcs_info,
        crate::theme::Theme::dark(),
        None,
        false,
        paths.iter().map(|p| make_file(p)).collect(),
        session,
        DiffSource::WorkingTree,
        InputMode::Normal,
        Vec::new(),
        None,
        None,
    )
    .expect("build app")
}

/// Renders the visible tree as (label, depth) pairs, with directories
/// suffixed by '/' so a mis-parented file is obvious in the assertion.
fn rendered_tree(app: &App) -> Vec<(String, usize)> {
    app.build_visible_items()
        .iter()
        .map(|item| match item {
            FileTreeItem::Directory { path, depth, .. } => (format!("{path}/"), *depth),
            FileTreeItem::File { file_idx, depth } => (
                app.diff_files[*file_idx]
                    .display_path()
                    .to_string_lossy()
                    .to_string(),
                *depth,
            ),
        })
        .collect()
}

#[test]
fn test_interleaved_paths_stay_under_own_directory() {
    // A sibling directory sharing a prefix used to sort between a directory
    // and its own subdirectory, because '.' sorts before '/'. That split the
    // ChronoStream subtree in two, and since build_visible_items emits a
    // directory header only once, ChronoStream/Subdir rendered indented under
    // ChronoStream.BuildTests.
    let mut app = app_with(&[
        "ChronoStream/Subdir/file.cs",
        "ChronoStream.BuildTests/test.cs",
        "ChronoStream/root.cs",
    ]);
    app.sort_files_by_directory(true);
    app.expand_all_dirs();

    assert_eq!(
        rendered_tree(&app),
        vec![
            ("ChronoStream/".to_string(), 0),
            ("ChronoStream/root.cs".to_string(), 1),
            ("ChronoStream/Subdir/".to_string(), 1),
            ("ChronoStream/Subdir/file.cs".to_string(), 2),
            ("ChronoStream.BuildTests/".to_string(), 0),
            ("ChronoStream.BuildTests/test.cs".to_string(), 1),
        ]
    );
}

#[test]
fn compact_folders_join_deep_chains_and_toggle_as_one_row() {
    let mut app = app_with(&["app/src/main/kotlin/Editor.kt"]);
    app.compact_folders = true;
    app.expand_all_dirs();
    assert_eq!(
        rendered_tree(&app),
        vec![
            ("app/src/main/kotlin/".into(), 0),
            ("app/src/main/kotlin/Editor.kt".into(), 1),
        ]
    );
    app.collapse_all_dirs();
    assert_eq!(app.build_visible_items().len(), 1);
    app.toggle_directory("app/src/main/kotlin");
    assert_eq!(app.build_visible_items().len(), 2);
    app.toggle_directory("app/src/main/kotlin");
    assert_eq!(app.build_visible_items().len(), 1);
    assert_eq!(app.file_list_state.selected(), 0);
}

#[test]
fn compact_folders_stop_at_files_and_branches() {
    let mut app = app_with(&[
        "README.md",
        "a/b/direct.kt",
        "a/b/c/d/one.kt",
        "a/b/e/two.kt",
    ]);
    app.compact_folders = true;
    assert_eq!(
        rendered_tree(&app),
        vec![
            ("README.md".into(), 0),
            ("a/b/".into(), 0),
            ("a/b/direct.kt".into(), 1),
            ("a/b/c/d/".into(), 1),
            ("a/b/c/d/one.kt".into(), 2),
            ("a/b/e/".into(), 1),
            ("a/b/e/two.kt".into(), 2),
        ]
    );
    app.collapse_all_dirs();
    assert_eq!(
        rendered_tree(&app),
        vec![("README.md".into(), 0), ("a/b/".into(), 0)]
    );
    app.expand_all_dirs();
    assert_eq!(app.build_visible_items().len(), 7);
}

#[test]
fn compact_folders_recompute_after_filter_and_search_reveals_full_path() {
    let mut app = app_with(&["a/b/c/one.kt", "a/b/d/two.kt"]);
    app.compact_folders = true;
    assert_eq!(app.build_visible_items().len(), 5);
    app.begin_file_tree_prompt(FileTreePrompt::Include);
    for ch in "one".chars() {
        app.file_tree_prompt_insert_char(ch);
    }
    app.commit_file_tree_prompt();
    assert_eq!(
        rendered_tree(&app),
        vec![("a/b/c/".into(), 0), ("a/b/c/one.kt".into(), 1)]
    );
    app.collapse_all_dirs();
    app.begin_file_tree_prompt(FileTreePrompt::Search);
    for ch in "a/b/c/one".chars() {
        app.file_tree_prompt_insert_char(ch);
    }
    app.commit_file_tree_prompt();
    assert!(matches!(
        app.get_selected_tree_item(),
        Some(FileTreeItem::File {
            file_idx: 0,
            depth: 1
        })
    ));
    app.clear_include_filter();
    assert_eq!(app.build_visible_items().len(), 4);
    app.expand_all_dirs();
    assert_eq!(app.build_visible_items().len(), 5);
}

#[test]
fn compact_folders_are_disabled_by_default() {
    let app = app_with(&["a/b/c/one.kt"]);
    assert!(!app.compact_folders);
    assert_eq!(app.build_visible_items().len(), 4);
}

#[test]
fn compact_folders_startup_keeps_selection_on_current_file() {
    let mut app = app_with(&["a/b/c/one.kt", "z/two.kt"]);
    let current_file_idx = app.diff_state.current_file_idx;
    app.set_compact_folders(true);
    assert!(
        matches!(app.get_selected_tree_item(), Some(FileTreeItem::File { file_idx, .. }) if file_idx == current_file_idx)
    );
}
