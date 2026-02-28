//! getCurrentSelection tool implementation.

use crate::ide::protocol::{
    Position, SelectionRange, SelectionResult, ToolContent, ToolsCallResult,
};
use crate::ide::state::IdeState;

/// Get the currently selected text in the diff viewer.
pub fn get_current_selection(state: &IdeState) -> ToolsCallResult {
    let selection = state.get_selection();

    match selection {
        Some(sel) => {
            let result = SelectionResult {
                file_path: sel.file_path.clone(),
                text: sel.text.clone(),
                selection: SelectionRange {
                    start: Position {
                        line: sel.start_line,
                        character: 0,
                    },
                    end: Position {
                        line: sel.end_line,
                        character: 0,
                    },
                },
            };

            let json = serde_json::to_string(&result).unwrap_or_else(|_| "{}".to_string());
            ToolsCallResult {
                content: vec![ToolContent::text(json)],
                is_error: false,
            }
        }
        None => ToolsCallResult {
            content: vec![ToolContent::text(
                r#"{"error": "No selection", "message": "No text is currently selected in the diff viewer"}"#,
            )],
            is_error: false,
        },
    }
}
