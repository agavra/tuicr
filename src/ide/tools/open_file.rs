//! openFile tool implementation.

use std::collections::HashMap;

use crate::ide::protocol::{OpenFileResult, ToolContent, ToolsCallResult};
use crate::ide::IdeCommand;
use tokio::sync::mpsc;

/// Navigate to a specific file in the diff viewer.
pub async fn open_file(
    arguments: &HashMap<String, serde_json::Value>,
    command_tx: &mpsc::Sender<IdeCommand>,
) -> ToolsCallResult {
    let file_path = match arguments.get("filePath").and_then(|v| v.as_str()) {
        Some(path) => path.to_string(),
        None => {
            let result = OpenFileResult {
                success: false,
                error: Some("Missing required parameter: filePath".to_string()),
            };
            let json = serde_json::to_string(&result).unwrap_or_else(|_| "{}".to_string());
            return ToolsCallResult {
                content: vec![ToolContent::text(json)],
                is_error: true,
            };
        }
    };

    let line = arguments
        .get("line")
        .and_then(|v| v.as_i64())
        .map(|l| l as u32);

    // Send command to the main event loop
    let cmd = IdeCommand::OpenFile {
        path: file_path.clone(),
        line,
    };

    match command_tx.send(cmd).await {
        Ok(()) => {
            let result = OpenFileResult {
                success: true,
                error: None,
            };
            let json = serde_json::to_string(&result).unwrap_or_else(|_| "{}".to_string());
            ToolsCallResult {
                content: vec![ToolContent::text(json)],
                is_error: false,
            }
        }
        Err(e) => {
            let result = OpenFileResult {
                success: false,
                error: Some(format!("Failed to send command: {e}")),
            };
            let json = serde_json::to_string(&result).unwrap_or_else(|_| "{}".to_string());
            ToolsCallResult {
                content: vec![ToolContent::text(json)],
                is_error: true,
            }
        }
    }
}
