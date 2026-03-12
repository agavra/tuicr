//! getDiagnostics tool implementation.

use std::collections::HashMap;

use crate::ide::protocol::{
    Diagnostic, DiagnosticSeverity, Position, SelectionRange, ToolContent, ToolsCallResult,
};
use crate::ide::state::IdeState;

/// Get review comments as diagnostics.
pub fn get_diagnostics(
    state: &IdeState,
    arguments: &HashMap<String, serde_json::Value>,
) -> ToolsCallResult {
    let file_path_filter = arguments
        .get("filePath")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let diagnostics_data = state.get_diagnostics(file_path_filter.as_deref());

    let result: Vec<Diagnostic> = diagnostics_data
        .into_iter()
        .map(|d| Diagnostic {
            file_path: d.file_path,
            range: SelectionRange {
                start: Position {
                    line: d.start_line,
                    character: 0,
                },
                end: Position {
                    line: d.end_line,
                    character: 0,
                },
            },
            message: d.message,
            severity: match d.severity.as_str() {
                "error" => DiagnosticSeverity::Error,
                "warning" => DiagnosticSeverity::Warning,
                "hint" => DiagnosticSeverity::Hint,
                _ => DiagnosticSeverity::Information,
            },
            source: Some("tuicr".to_string()),
        })
        .collect();

    let json = serde_json::to_string(&result).unwrap_or_else(|_| "[]".to_string());
    ToolsCallResult {
        content: vec![ToolContent::text(json)],
        is_error: false,
    }
}
