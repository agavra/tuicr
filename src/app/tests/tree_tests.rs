use crate::app::*;
use crate::model::{DiffFile, FileStatus};

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
