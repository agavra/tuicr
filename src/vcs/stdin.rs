use std::path::{Path, PathBuf};

use crate::error::{Result, TuicrError};
use crate::model::{DiffFile, DiffLine, FilePatch, FileStatus};
use crate::syntax::SyntaxHighlighter;

use super::diff_parser::parse_file_patches;
use super::traits::{VcsBackend, VcsInfo, VcsType};

/// Backend for a unified diff piped in on stdin (`pager.diff = tuicr`).
pub struct StdinBackend {
    info: VcsInfo,
    diff_text: String,
}

impl StdinBackend {
    pub fn new(diff_text: String) -> Result<Self> {
        if split_git_diff(&diff_text).is_empty() {
            return Err(TuicrError::NoChanges);
        }
        let hash = crate::hash::fnv1a_64(strip_ansi(&diff_text).as_bytes());
        let info = VcsInfo {
            root_path: repo_root()?,
            head_commit: format!("{hash:016x}"),
            branch_name: None,
            vcs_type: VcsType::File,
        };
        Ok(Self { info, diff_text })
    }
}

/// Repo root, so `git diff | tuicr --stdin` works from a subdirectory.
fn repo_root() -> Result<PathBuf> {
    let cwd = std::env::current_dir()?;
    let output = std::process::Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(&cwd)
        .output();
    match output {
        Ok(o) if o.status.success() => {
            let root = String::from_utf8_lossy(&o.stdout).trim().to_string();
            Ok(if root.is_empty() {
                cwd
            } else {
                PathBuf::from(root)
            })
        }
        _ => Ok(cwd),
    }
}

impl VcsBackend for StdinBackend {
    fn info(&self) -> &VcsInfo {
        &self.info
    }

    fn get_working_tree_diff(&self, highlighter: &SyntaxHighlighter) -> Result<Vec<DiffFile>> {
        parse_file_patches(split_git_diff(&self.diff_text), highlighter)
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

/// Git colours pager output, so drop ANSI CSI sequences before parsing.
fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' && chars.peek() == Some(&'[') {
            chars.next();
            for c in chars.by_ref() {
                if ('\x40'..='\x7e').contains(&c) {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Split `git diff` output into one `FilePatch` per `diff --git` chunk.
pub(crate) fn split_git_diff(text: &str) -> Vec<FilePatch> {
    let text = strip_ansi(text);
    let mut chunks: Vec<String> = Vec::new();
    for line in text.lines() {
        if line.starts_with("diff --git ") {
            chunks.push(String::new());
        }
        if let Some(chunk) = chunks.last_mut() {
            chunk.push_str(line);
            chunk.push('\n');
        }
    }
    chunks.iter().filter_map(|c| parse_chunk(c)).collect()
}

fn parse_chunk(chunk: &str) -> Option<FilePatch> {
    let mut lines = chunk.lines();
    // "diff --git a/<old> b/<new>"
    let rest = lines.next()?.strip_prefix("diff --git a/")?;
    let split = rest.find(" b/")?;
    let mut old = PathBuf::from(&rest[..split]);
    let mut new = PathBuf::from(&rest[split + 3..]);

    let mut status = FileStatus::Modified;
    let mut binary = false;
    for line in lines {
        if line.starts_with("@@") {
            break;
        }
        if line.starts_with("new file mode") {
            status = FileStatus::Added;
        } else if line.starts_with("deleted file mode") {
            status = FileStatus::Deleted;
        } else if let Some(p) = line.strip_prefix("rename from ") {
            status = FileStatus::Renamed;
            old = PathBuf::from(p);
        } else if let Some(p) = line.strip_prefix("rename to ") {
            new = PathBuf::from(p);
        } else if let Some(p) = line.strip_prefix("copy from ") {
            status = FileStatus::Copied;
            old = PathBuf::from(p);
        } else if let Some(p) = line.strip_prefix("copy to ") {
            new = PathBuf::from(p);
        } else if line.starts_with("Binary files ") || line.starts_with("GIT binary patch") {
            binary = true;
        }
    }

    let (old_path, new_path) = match status {
        FileStatus::Added => (None, Some(new)),
        FileStatus::Deleted => (Some(old), None),
        _ => (Some(old), Some(new)),
    };
    let mut patch = FilePatch::new(old_path, new_path, status, chunk);
    patch.is_binary = binary;
    Some(patch)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
diff --git a/src/a.rs b/src/a.rs
index 111..222 100644
--- a/src/a.rs
+++ b/src/a.rs
@@ -1,2 +1,2 @@
 keep
-old
+new
diff --git a/b.txt b/b.txt
new file mode 100644
index 0000000..333
--- /dev/null
+++ b/b.txt
@@ -0,0 +1 @@
+hello
";

    #[test]
    fn splits_modified_and_added_files() {
        let patches = split_git_diff(SAMPLE);
        assert_eq!(patches.len(), 2);
        assert!(matches!(patches[0].status, FileStatus::Modified));
        assert_eq!(patches[0].new_path.as_deref(), Some(Path::new("src/a.rs")));
        assert!(matches!(patches[1].status, FileStatus::Added));
        assert!(patches[1].old_path.is_none());
    }

    #[test]
    fn strips_ansi_colour_codes() {
        let coloured = "\x1b[1mdiff --git a/x b/x\x1b[m\n\x1b[36m@@ -1 +1 @@\x1b[m\n-a\n+b\n";
        assert_eq!(split_git_diff(coloured).len(), 1);
    }

    #[test]
    fn sample_parses_into_diff_files() {
        let backend = StdinBackend::new(SAMPLE.to_string()).unwrap();
        let files = backend
            .get_working_tree_diff(&SyntaxHighlighter::default())
            .unwrap();
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].hunks[0].lines.len(), 3);
    }

    #[test]
    fn detects_deleted_and_renamed_files() {
        let text = "\
diff --git a/gone.txt b/gone.txt
deleted file mode 100644
--- a/gone.txt
+++ /dev/null
@@ -1 +0,0 @@
-bye
diff --git a/old.rs b/new.rs
similarity index 90%
rename from old.rs
rename to new.rs
";
        let patches = split_git_diff(text);
        assert!(matches!(patches[0].status, FileStatus::Deleted));
        assert!(patches[0].new_path.is_none());
        assert!(matches!(patches[1].status, FileStatus::Renamed));
        assert_eq!(patches[1].old_path.as_deref(), Some(Path::new("old.rs")));
        assert_eq!(patches[1].new_path.as_deref(), Some(Path::new("new.rs")));
    }

    #[test]
    fn colour_codes_do_not_change_session_identity() {
        let plain = StdinBackend::new(SAMPLE.to_string()).unwrap();
        let coloured = StdinBackend::new(format!("\x1b[1m{SAMPLE}\x1b[m")).unwrap();
        assert_eq!(plain.info().head_commit, coloured.info().head_commit);
    }
}
