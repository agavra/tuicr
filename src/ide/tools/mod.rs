//! IDE tool implementations for Claude Code MCP protocol.
//!
//! Each tool maps tuicr concepts to IDE semantics:
//! - getCurrentSelection: Selected lines in diff view
//! - getOpenEditors: Files in the current review session
//! - getWorkspaceFolders: Repository root being reviewed
//! - getDiagnostics: Review comments (Issue/Suggestion types)
//! - openFile: Navigate to a file in the diff view

mod get_current_selection;
mod get_diagnostics;
mod get_open_editors;
mod get_workspace_folders;
mod open_file;

pub use get_current_selection::get_current_selection;
pub use get_diagnostics::get_diagnostics;
pub use get_open_editors::get_open_editors;
pub use get_workspace_folders::get_workspace_folders;
pub use open_file::open_file;

use crate::ide::protocol::Tool;

/// Returns the list of all available IDE tools with their schemas.
pub fn all_tools() -> Vec<Tool> {
    vec![
        Tool {
            name: "getCurrentSelection".to_string(),
            description: Some(
                "Get the currently selected text in the diff viewer. Returns the file path, \
                 selected text, and line range of the selection."
                    .to_string(),
            ),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {},
                "required": []
            }),
        },
        Tool {
            name: "getOpenEditors".to_string(),
            description: Some(
                "Get the list of files open in the current code review session. Each file \
                 includes its path, status (added/modified/deleted), and review state."
                    .to_string(),
            ),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {},
                "required": []
            }),
        },
        Tool {
            name: "getWorkspaceFolders".to_string(),
            description: Some(
                "Get the workspace folders (repository roots) for the current review session."
                    .to_string(),
            ),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {},
                "required": []
            }),
        },
        Tool {
            name: "getDiagnostics".to_string(),
            description: Some(
                "Get review comments as diagnostics. Issue-type comments are returned as errors, \
                 Suggestion-type as warnings, and Note/Praise as information."
                    .to_string(),
            ),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "filePath": {
                        "type": "string",
                        "description": "Optional file path to filter diagnostics. If not provided, returns all diagnostics."
                    }
                },
                "required": []
            }),
        },
        Tool {
            name: "openFile".to_string(),
            description: Some(
                "Navigate to a specific file in the diff viewer. Optionally jump to a specific line."
                    .to_string(),
            ),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "filePath": {
                        "type": "string",
                        "description": "Path to the file to open"
                    },
                    "line": {
                        "type": "integer",
                        "description": "Optional line number to jump to"
                    }
                },
                "required": ["filePath"]
            }),
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ide::state::{DiagnosticInfo, IdeState, OpenFileInfo, Selection};
    use std::collections::HashMap;

    #[test]
    fn all_tools_returns_five_tools() {
        let tools = all_tools();
        assert_eq!(tools.len(), 5);
    }

    #[test]
    fn all_tools_have_descriptions() {
        let tools = all_tools();
        for tool in &tools {
            assert!(tool.description.is_some());
            assert!(!tool.description.as_ref().unwrap().is_empty());
        }
    }

    #[test]
    fn all_tools_have_valid_schemas() {
        let tools = all_tools();
        for tool in &tools {
            assert_eq!(tool.input_schema["type"], "object");
        }
    }

    #[test]
    fn open_file_tool_requires_file_path() {
        let tools = all_tools();
        let open_file = tools.iter().find(|t| t.name == "openFile").unwrap();
        let required = open_file.input_schema["required"].as_array().unwrap();
        assert!(required.contains(&serde_json::json!("filePath")));
    }

    #[test]
    fn get_current_selection_with_selection() {
        let mut state = IdeState::new();
        state.set_selection(Some(Selection {
            file_path: "test.rs".to_string(),
            text: "fn main()".to_string(),
            start_line: 10,
            end_line: 15,
        }));

        let result = get_current_selection(&state);
        assert!(!result.is_error);

        let content = &result.content[0];
        if let crate::ide::protocol::ToolContent::Text { text } = content {
            let parsed: serde_json::Value = serde_json::from_str(text).unwrap();
            assert_eq!(parsed["filePath"], "test.rs");
            assert_eq!(parsed["selection"]["start"]["line"], 10);
            assert_eq!(parsed["selection"]["end"]["line"], 15);
        }
    }

    #[test]
    fn get_current_selection_without_selection() {
        let state = IdeState::new();
        let result = get_current_selection(&state);

        assert!(!result.is_error);
        let content = &result.content[0];
        if let crate::ide::protocol::ToolContent::Text { text } = content {
            assert!(text.contains("No selection"));
        }
    }

    #[test]
    fn get_open_editors_with_files() {
        let mut state = IdeState::new();
        state.set_open_files(vec![
            OpenFileInfo {
                file_path: "src/main.rs".to_string(),
                language_id: "rust".to_string(),
                is_dirty: false,
                is_active: true,
                status: "Modified".to_string(),
                reviewed: true,
            },
            OpenFileInfo {
                file_path: "src/lib.rs".to_string(),
                language_id: "rust".to_string(),
                is_dirty: true,
                is_active: false,
                status: "Added".to_string(),
                reviewed: false,
            },
        ]);

        let result = get_open_editors(&state);
        assert!(!result.is_error);

        let content = &result.content[0];
        if let crate::ide::protocol::ToolContent::Text { text } = content {
            let parsed: Vec<serde_json::Value> = serde_json::from_str(text).unwrap();
            assert_eq!(parsed.len(), 2);
            assert_eq!(parsed[0]["filePath"], "src/main.rs");
            assert_eq!(parsed[0]["languageId"], "rust");
            assert_eq!(parsed[0]["isActive"], true);
            assert_eq!(parsed[1]["filePath"], "src/lib.rs");
            assert_eq!(parsed[1]["isDirty"], true);
        }
    }

    #[test]
    fn get_workspace_folders_with_workspace() {
        let mut state = IdeState::new();
        state.set_workspace("/path/to/repo".to_string(), Some("repo".to_string()));

        let result = get_workspace_folders(&state);
        assert!(!result.is_error);

        let content = &result.content[0];
        if let crate::ide::protocol::ToolContent::Text { text } = content {
            let parsed: Vec<serde_json::Value> = serde_json::from_str(text).unwrap();
            assert_eq!(parsed.len(), 1);
            assert_eq!(parsed[0]["uri"], "file:///path/to/repo");
            assert_eq!(parsed[0]["name"], "repo");
        }
    }

    #[test]
    fn get_diagnostics_returns_all() {
        let mut state = IdeState::new();
        state.set_diagnostics(vec![
            DiagnosticInfo {
                file_path: "file1.rs".to_string(),
                start_line: 10,
                end_line: 10,
                message: "Issue here".to_string(),
                severity: "error".to_string(),
                comment_type: "Issue".to_string(),
            },
            DiagnosticInfo {
                file_path: "file2.rs".to_string(),
                start_line: 20,
                end_line: 25,
                message: "Suggestion here".to_string(),
                severity: "warning".to_string(),
                comment_type: "Suggestion".to_string(),
            },
        ]);

        let result = get_diagnostics(&state, &HashMap::new());
        assert!(!result.is_error);

        let content = &result.content[0];
        if let crate::ide::protocol::ToolContent::Text { text } = content {
            let parsed: Vec<serde_json::Value> = serde_json::from_str(text).unwrap();
            assert_eq!(parsed.len(), 2);
        }
    }

    #[test]
    fn get_diagnostics_filters_by_file() {
        let mut state = IdeState::new();
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
                end_line: 20,
                message: "Issue 2".to_string(),
                severity: "error".to_string(),
                comment_type: "Issue".to_string(),
            },
        ]);

        let mut args = HashMap::new();
        args.insert("filePath".to_string(), serde_json::json!("file1.rs"));

        let result = get_diagnostics(&state, &args);
        assert!(!result.is_error);

        let content = &result.content[0];
        if let crate::ide::protocol::ToolContent::Text { text } = content {
            let parsed: Vec<serde_json::Value> = serde_json::from_str(text).unwrap();
            assert_eq!(parsed.len(), 1);
            assert_eq!(parsed[0]["message"], "Issue 1");
        }
    }

    #[test]
    fn get_diagnostics_maps_severity_correctly() {
        let mut state = IdeState::new();
        state.set_diagnostics(vec![
            DiagnosticInfo {
                file_path: "test.rs".to_string(),
                start_line: 1,
                end_line: 1,
                message: "error".to_string(),
                severity: "error".to_string(),
                comment_type: "Issue".to_string(),
            },
            DiagnosticInfo {
                file_path: "test.rs".to_string(),
                start_line: 2,
                end_line: 2,
                message: "warning".to_string(),
                severity: "warning".to_string(),
                comment_type: "Suggestion".to_string(),
            },
            DiagnosticInfo {
                file_path: "test.rs".to_string(),
                start_line: 3,
                end_line: 3,
                message: "info".to_string(),
                severity: "information".to_string(),
                comment_type: "Note".to_string(),
            },
            DiagnosticInfo {
                file_path: "test.rs".to_string(),
                start_line: 4,
                end_line: 4,
                message: "hint".to_string(),
                severity: "hint".to_string(),
                comment_type: "Praise".to_string(),
            },
        ]);

        let result = get_diagnostics(&state, &HashMap::new());
        let content = &result.content[0];
        if let crate::ide::protocol::ToolContent::Text { text } = content {
            let parsed: Vec<serde_json::Value> = serde_json::from_str(text).unwrap();
            assert_eq!(parsed[0]["severity"], "error");
            assert_eq!(parsed[1]["severity"], "warning");
            assert_eq!(parsed[2]["severity"], "information");
            assert_eq!(parsed[3]["severity"], "hint");
        }
    }

    #[tokio::test]
    async fn open_file_missing_path_returns_error() {
        let (tx, _rx) = tokio::sync::mpsc::channel(32);
        let result = open_file(&HashMap::new(), &tx).await;

        assert!(result.is_error);
        let content = &result.content[0];
        if let crate::ide::protocol::ToolContent::Text { text } = content {
            assert!(text.contains("Missing required parameter"));
        }
    }

    #[tokio::test]
    async fn open_file_sends_command() {
        let (tx, mut rx) = tokio::sync::mpsc::channel(32);

        let mut args = HashMap::new();
        args.insert("filePath".to_string(), serde_json::json!("src/main.rs"));
        args.insert("line".to_string(), serde_json::json!(42));

        let result = open_file(&args, &tx).await;
        assert!(!result.is_error);

        // Verify the command was sent
        let cmd = rx.recv().await.unwrap();
        match cmd {
            crate::ide::IdeCommand::OpenFile { path, line } => {
                assert_eq!(path, "src/main.rs");
                assert_eq!(line, Some(42));
            }
        }
    }

    #[tokio::test]
    async fn open_file_without_line() {
        let (tx, mut rx) = tokio::sync::mpsc::channel(32);

        let mut args = HashMap::new();
        args.insert("filePath".to_string(), serde_json::json!("src/main.rs"));

        let result = open_file(&args, &tx).await;
        assert!(!result.is_error);

        let cmd = rx.recv().await.unwrap();
        match cmd {
            crate::ide::IdeCommand::OpenFile { path, line } => {
                assert_eq!(path, "src/main.rs");
                assert_eq!(line, None);
            }
        }
    }
}
