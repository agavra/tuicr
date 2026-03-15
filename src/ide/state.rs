//! IDE state interface for providing app state to tools.
//!
//! This module provides a thread-safe view of the app state that can be
//! shared with the async IDE server.

use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Shared IDE state that provides a view of the app state for tools.
pub type SharedIdeState = Arc<RwLock<IdeState>>;

/// IDE state snapshot that tools can query.
#[derive(Debug, Clone, Default)]
pub struct IdeState {
    /// Current selection in the diff viewer
    pub selection: Option<Selection>,
    /// Files in the current review session
    pub open_files: Vec<OpenFileInfo>,
    /// Workspace (repository) root path
    pub workspace_path: Option<String>,
    /// Workspace name (usually the repo directory name)
    pub workspace_name: Option<String>,
    /// Review comments (as diagnostics)
    pub diagnostics: Vec<DiagnosticInfo>,
    /// Index of the currently active/viewed file
    pub active_file_index: usize,
}

/// Selection information from the diff viewer.
#[derive(Debug, Clone)]
pub struct Selection {
    pub file_path: String,
    pub text: String,
    pub start_line: u32,
    pub end_line: u32,
}

/// Information about an open file in the review.
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct OpenFileInfo {
    pub file_path: String,
    pub language_id: String,
    pub is_dirty: bool,
    pub is_active: bool,
    pub status: String,
    pub reviewed: bool,
}

/// Workspace folder information.
#[derive(Debug, Clone)]
pub struct WorkspaceFolderInfo {
    pub path: String,
    pub name: String,
}

/// Diagnostic (comment) information.
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct DiagnosticInfo {
    pub file_path: String,
    pub start_line: u32,
    pub end_line: u32,
    pub message: String,
    pub severity: String,
    pub comment_type: String,
}

impl IdeState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Get the current selection.
    pub fn get_selection(&self) -> Option<&Selection> {
        self.selection.as_ref()
    }

    /// Get the list of open editors/files.
    pub fn get_open_editors(&self) -> Vec<OpenFileInfo> {
        self.open_files.clone()
    }

    /// Get workspace folders.
    pub fn get_workspace_folders(&self) -> Vec<WorkspaceFolderInfo> {
        match (&self.workspace_path, &self.workspace_name) {
            (Some(path), Some(name)) => vec![WorkspaceFolderInfo {
                path: path.clone(),
                name: name.clone(),
            }],
            (Some(path), None) => {
                let name = PathBuf::from(path)
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| "workspace".to_string());
                vec![WorkspaceFolderInfo {
                    path: path.clone(),
                    name,
                }]
            }
            _ => vec![],
        }
    }

    /// Get diagnostics, optionally filtered by file path.
    pub fn get_diagnostics(&self, file_path: Option<&str>) -> Vec<DiagnosticInfo> {
        match file_path {
            Some(path) => self
                .diagnostics
                .iter()
                .filter(|d| d.file_path == path)
                .cloned()
                .collect(),
            None => self.diagnostics.clone(),
        }
    }

    /// Update the selection state.
    pub fn set_selection(&mut self, selection: Option<Selection>) {
        self.selection = selection;
    }

    /// Update the open files list.
    pub fn set_open_files(&mut self, files: Vec<OpenFileInfo>) {
        self.open_files = files;
    }

    /// Set the workspace information.
    pub fn set_workspace(&mut self, path: String, name: Option<String>) {
        self.workspace_path = Some(path);
        self.workspace_name = name;
    }

    /// Set the diagnostics (review comments).
    pub fn set_diagnostics(&mut self, diagnostics: Vec<DiagnosticInfo>) {
        self.diagnostics = diagnostics;
    }

    /// Set the active file index.
    pub fn set_active_file(&mut self, index: usize) {
        self.active_file_index = index;
        // Update is_active flag on files
        for (i, file) in self.open_files.iter_mut().enumerate() {
            file.is_active = i == index;
        }
    }
}

/// Create a new shared IDE state.
pub fn new_shared_state() -> SharedIdeState {
    Arc::new(RwLock::new(IdeState::new()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ide_state_default_is_empty() {
        let state = IdeState::new();
        assert!(state.selection.is_none());
        assert!(state.open_files.is_empty());
        assert!(state.workspace_path.is_none());
        assert!(state.diagnostics.is_empty());
    }

    #[test]
    fn set_and_get_selection() {
        let mut state = IdeState::new();
        assert!(state.get_selection().is_none());

        state.set_selection(Some(Selection {
            file_path: "test.rs".to_string(),
            text: "selected".to_string(),
            start_line: 10,
            end_line: 15,
        }));

        let sel = state.get_selection().unwrap();
        assert_eq!(sel.file_path, "test.rs");
        assert_eq!(sel.start_line, 10);
        assert_eq!(sel.end_line, 15);
    }

    #[test]
    fn set_and_get_open_files() {
        let mut state = IdeState::new();
        assert!(state.get_open_editors().is_empty());

        state.set_open_files(vec![
            OpenFileInfo {
                file_path: "file1.rs".to_string(),
                language_id: "rust".to_string(),
                is_dirty: false,
                is_active: true,
                status: "Modified".to_string(),
                reviewed: false,
            },
            OpenFileInfo {
                file_path: "file2.rs".to_string(),
                language_id: "rust".to_string(),
                is_dirty: true,
                is_active: false,
                status: "Added".to_string(),
                reviewed: true,
            },
        ]);

        let editors = state.get_open_editors();
        assert_eq!(editors.len(), 2);
        assert_eq!(editors[0].file_path, "file1.rs");
        assert!(editors[0].is_active);
        assert_eq!(editors[1].file_path, "file2.rs");
        assert!(!editors[1].is_active);
    }

    #[test]
    fn set_and_get_workspace() {
        let mut state = IdeState::new();
        assert!(state.get_workspace_folders().is_empty());

        state.set_workspace("/path/to/repo".to_string(), Some("repo".to_string()));

        let folders = state.get_workspace_folders();
        assert_eq!(folders.len(), 1);
        assert_eq!(folders[0].path, "/path/to/repo");
        assert_eq!(folders[0].name, "repo");
    }

    #[test]
    fn workspace_derives_name_from_path() {
        let mut state = IdeState::new();
        state.set_workspace("/path/to/myproject".to_string(), None);

        let folders = state.get_workspace_folders();
        assert_eq!(folders.len(), 1);
        assert_eq!(folders[0].name, "myproject");
    }

    #[test]
    fn set_and_get_diagnostics() {
        let mut state = IdeState::new();
        assert!(state.get_diagnostics(None).is_empty());

        state.set_diagnostics(vec![
            DiagnosticInfo {
                file_path: "file1.rs".to_string(),
                start_line: 10,
                end_line: 10,
                message: "Issue 1".to_string(),
                severity: "error".to_string(),
                comment_type: "Issue".to_string(),
            },
            DiagnosticInfo {
                file_path: "file2.rs".to_string(),
                start_line: 20,
                end_line: 25,
                message: "Suggestion 1".to_string(),
                severity: "warning".to_string(),
                comment_type: "Suggestion".to_string(),
            },
        ]);

        // Get all diagnostics
        let all = state.get_diagnostics(None);
        assert_eq!(all.len(), 2);

        // Filter by file
        let file1_diags = state.get_diagnostics(Some("file1.rs"));
        assert_eq!(file1_diags.len(), 1);
        assert_eq!(file1_diags[0].message, "Issue 1");

        let file2_diags = state.get_diagnostics(Some("file2.rs"));
        assert_eq!(file2_diags.len(), 1);
        assert_eq!(file2_diags[0].message, "Suggestion 1");

        // Non-existent file
        let no_diags = state.get_diagnostics(Some("nonexistent.rs"));
        assert!(no_diags.is_empty());
    }

    #[test]
    fn set_active_file_updates_is_active() {
        let mut state = IdeState::new();
        state.set_open_files(vec![
            OpenFileInfo {
                file_path: "file1.rs".to_string(),
                language_id: "rust".to_string(),
                is_dirty: false,
                is_active: true,
                status: "Modified".to_string(),
                reviewed: false,
            },
            OpenFileInfo {
                file_path: "file2.rs".to_string(),
                language_id: "rust".to_string(),
                is_dirty: false,
                is_active: false,
                status: "Added".to_string(),
                reviewed: false,
            },
        ]);

        // Initially file1 is active
        assert!(state.open_files[0].is_active);
        assert!(!state.open_files[1].is_active);

        // Switch to file2
        state.set_active_file(1);
        assert!(!state.open_files[0].is_active);
        assert!(state.open_files[1].is_active);
        assert_eq!(state.active_file_index, 1);
    }

    #[tokio::test]
    async fn shared_state_is_thread_safe() {
        let state = new_shared_state();

        // Write from one task
        {
            let mut guard = state.write().await;
            guard.set_workspace("/test".to_string(), Some("test".to_string()));
        }

        // Read from another
        {
            let guard = state.read().await;
            let folders = guard.get_workspace_folders();
            assert_eq!(folders.len(), 1);
            assert_eq!(folders[0].path, "/test");
        }
    }
}
