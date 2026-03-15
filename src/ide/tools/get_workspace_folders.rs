//! getWorkspaceFolders tool implementation.

use crate::ide::protocol::{ToolContent, ToolsCallResult, WorkspaceFolder};
use crate::ide::state::IdeState;

/// Get the workspace folders (repository root) for the current review.
pub fn get_workspace_folders(state: &IdeState) -> ToolsCallResult {
    let folders = state.get_workspace_folders();

    let result: Vec<WorkspaceFolder> = folders
        .into_iter()
        .map(|f| WorkspaceFolder {
            uri: format!("file://{}", f.path),
            name: f.name,
        })
        .collect();

    let json = serde_json::to_string(&result).unwrap_or_else(|_| "[]".to_string());
    ToolsCallResult {
        content: vec![ToolContent::text(json)],
        is_error: false,
    }
}
