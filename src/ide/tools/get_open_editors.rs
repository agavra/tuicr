//! getOpenEditors tool implementation.

use crate::ide::protocol::{OpenEditor, ToolContent, ToolsCallResult};
use crate::ide::state::IdeState;

/// Get the list of files in the current review session.
pub fn get_open_editors(state: &IdeState) -> ToolsCallResult {
    let editors = state.get_open_editors();

    let result: Vec<OpenEditor> = editors
        .into_iter()
        .map(|e| OpenEditor {
            file_path: e.file_path,
            language_id: e.language_id,
            is_dirty: Some(e.is_dirty),
            is_active: Some(e.is_active),
        })
        .collect();

    let json = serde_json::to_string(&result).unwrap_or_else(|_| "[]".to_string());
    ToolsCallResult {
        content: vec![ToolContent::text(json)],
        is_error: false,
    }
}
