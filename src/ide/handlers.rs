//! MCP method handlers for the IDE server.

use std::collections::HashMap;

use tokio::sync::mpsc;

use super::IdeCommand;
use super::protocol::{
    InitializeParams, InitializeResult, JsonRpcError, JsonRpcId, JsonRpcResponse,
    MCP_PROTOCOL_VERSION, ServerCapabilities, ServerInfo, ToolsCallParams, ToolsCallResult,
    ToolsCapability, ToolsListResult,
};
use super::state::SharedIdeState;
use super::tools;

/// Handle an MCP method call and return the response.
pub async fn handle_method(
    method: &str,
    params: Option<serde_json::Value>,
    id: Option<JsonRpcId>,
    state: &SharedIdeState,
    command_tx: &mpsc::Sender<IdeCommand>,
) -> Option<JsonRpcResponse> {
    match method {
        "initialize" => Some(handle_initialize(params, id)),
        "initialized" => {
            // Notification, no response
            None
        }
        "ping" => Some(handle_ping(id)),
        "tools/list" => Some(handle_tools_list(id)),
        "tools/call" => Some(handle_tools_call(params, id, state, command_tx).await),
        "notifications/cancelled" => {
            // Notification for cancelled requests, no response needed
            None
        }
        _ => Some(JsonRpcResponse::error(
            id,
            JsonRpcError::method_not_found(method),
        )),
    }
}

/// Handle the MCP initialize request.
fn handle_initialize(params: Option<serde_json::Value>, id: Option<JsonRpcId>) -> JsonRpcResponse {
    // Parse and validate params
    let _init_params: InitializeParams = match params {
        Some(p) => match serde_json::from_value(p) {
            Ok(params) => params,
            Err(e) => {
                return JsonRpcResponse::error(
                    id,
                    JsonRpcError::invalid_params(&format!(
                        "Failed to parse initialize params: {e}"
                    )),
                );
            }
        },
        None => {
            return JsonRpcResponse::error(
                id,
                JsonRpcError::invalid_params("Missing initialize params"),
            );
        }
    };

    let result = InitializeResult {
        protocol_version: MCP_PROTOCOL_VERSION.to_string(),
        capabilities: ServerCapabilities {
            tools: Some(ToolsCapability {
                list_changed: false,
            }),
            resources: None,
            prompts: None,
        },
        server_info: ServerInfo {
            name: "tuicr".to_string(),
            version: Some(env!("CARGO_PKG_VERSION").to_string()),
        },
        instructions: Some(
            "tuicr is a TUI code review tool. You can query the current selection, \
             open files, workspace, and review comments (diagnostics). Use openFile \
             to navigate to specific files in the diff viewer."
                .to_string(),
        ),
    };

    JsonRpcResponse::success(id, serde_json::to_value(result).unwrap())
}

/// Handle the ping request.
fn handle_ping(id: Option<JsonRpcId>) -> JsonRpcResponse {
    JsonRpcResponse::success(id, serde_json::json!({}))
}

/// Handle the tools/list request.
fn handle_tools_list(id: Option<JsonRpcId>) -> JsonRpcResponse {
    let result = ToolsListResult {
        tools: tools::all_tools(),
    };
    JsonRpcResponse::success(id, serde_json::to_value(result).unwrap())
}

/// Handle the tools/call request.
async fn handle_tools_call(
    params: Option<serde_json::Value>,
    id: Option<JsonRpcId>,
    state: &SharedIdeState,
    command_tx: &mpsc::Sender<IdeCommand>,
) -> JsonRpcResponse {
    let call_params: ToolsCallParams = match params {
        Some(p) => match serde_json::from_value(p) {
            Ok(params) => params,
            Err(e) => {
                return JsonRpcResponse::error(
                    id,
                    JsonRpcError::invalid_params(&format!(
                        "Failed to parse tools/call params: {e}"
                    )),
                );
            }
        },
        None => {
            return JsonRpcResponse::error(
                id,
                JsonRpcError::invalid_params("Missing tools/call params"),
            );
        }
    };

    let result = dispatch_tool(&call_params.name, &call_params.arguments, state, command_tx).await;
    JsonRpcResponse::success(id, serde_json::to_value(result).unwrap())
}

/// Dispatch a tool call to the appropriate handler.
async fn dispatch_tool(
    name: &str,
    arguments: &HashMap<String, serde_json::Value>,
    state: &SharedIdeState,
    command_tx: &mpsc::Sender<IdeCommand>,
) -> ToolsCallResult {
    let state_guard = state.read().await;

    match name {
        "getCurrentSelection" => tools::get_current_selection(&state_guard),
        "getOpenEditors" => tools::get_open_editors(&state_guard),
        "getWorkspaceFolders" => tools::get_workspace_folders(&state_guard),
        "getDiagnostics" => tools::get_diagnostics(&state_guard, arguments),
        "openFile" => {
            // Release the read lock before sending command
            drop(state_guard);
            tools::open_file(arguments, command_tx).await
        }
        _ => ToolsCallResult {
            content: vec![super::protocol::ToolContent::text(format!(
                r#"{{"error": "Unknown tool: {name}"}}"#
            ))],
            is_error: true,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ide::state::new_shared_state;

    fn create_test_state() -> (
        SharedIdeState,
        mpsc::Sender<IdeCommand>,
        mpsc::Receiver<IdeCommand>,
    ) {
        let state = new_shared_state();
        let (tx, rx) = mpsc::channel(32);
        (state, tx, rx)
    }

    #[tokio::test]
    async fn handle_initialize_returns_server_info() {
        let params = serde_json::json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": {"name": "test-client"}
        });

        let response = handle_initialize(Some(params), Some(JsonRpcId::Number(1)));

        assert!(response.error.is_none());
        assert!(response.result.is_some());

        let result = response.result.unwrap();
        assert_eq!(result["protocolVersion"], "2024-11-05");
        assert_eq!(result["serverInfo"]["name"], "tuicr");
    }

    #[tokio::test]
    async fn handle_initialize_without_params_returns_error() {
        let response = handle_initialize(None, Some(JsonRpcId::Number(1)));

        assert!(response.error.is_some());
        assert_eq!(response.error.unwrap().code, -32602); // Invalid params
    }

    #[test]
    fn handle_ping_returns_empty_object() {
        let response = handle_ping(Some(JsonRpcId::Number(1)));

        assert!(response.error.is_none());
        assert_eq!(response.result, Some(serde_json::json!({})));
    }

    #[test]
    fn handle_tools_list_returns_all_tools() {
        let response = handle_tools_list(Some(JsonRpcId::Number(1)));

        assert!(response.error.is_none());
        let result = response.result.unwrap();
        let tools = result["tools"].as_array().unwrap();

        // Should have 5 tools
        assert_eq!(tools.len(), 5);

        // Check tool names exist
        let tool_names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert!(tool_names.contains(&"getCurrentSelection"));
        assert!(tool_names.contains(&"getOpenEditors"));
        assert!(tool_names.contains(&"getWorkspaceFolders"));
        assert!(tool_names.contains(&"getDiagnostics"));
        assert!(tool_names.contains(&"openFile"));
    }

    #[tokio::test]
    async fn handle_method_returns_none_for_notifications() {
        let (state, tx, _rx) = create_test_state();

        // initialized is a notification, should return None
        let response = handle_method("initialized", None, None, &state, &tx).await;
        assert!(response.is_none());

        // notifications/cancelled should also return None
        let response = handle_method("notifications/cancelled", None, None, &state, &tx).await;
        assert!(response.is_none());
    }

    #[tokio::test]
    async fn handle_method_returns_error_for_unknown_method() {
        let (state, tx, _rx) = create_test_state();

        let response = handle_method(
            "unknown/method",
            None,
            Some(JsonRpcId::Number(1)),
            &state,
            &tx,
        )
        .await;

        assert!(response.is_some());
        let resp = response.unwrap();
        assert!(resp.error.is_some());
        assert_eq!(resp.error.unwrap().code, -32601); // Method not found
    }

    #[tokio::test]
    async fn handle_tools_call_with_missing_params_returns_error() {
        let (state, tx, _rx) = create_test_state();

        let response =
            handle_method("tools/call", None, Some(JsonRpcId::Number(1)), &state, &tx).await;

        assert!(response.is_some());
        let resp = response.unwrap();
        assert!(resp.error.is_some());
        assert_eq!(resp.error.unwrap().code, -32602); // Invalid params
    }

    #[tokio::test]
    async fn dispatch_unknown_tool_returns_error() {
        let (state, tx, _rx) = create_test_state();

        let result = dispatch_tool("unknownTool", &HashMap::new(), &state, &tx).await;

        assert!(result.is_error);
        let content = &result.content[0];
        if let super::super::protocol::ToolContent::Text { text } = content {
            assert!(text.contains("Unknown tool"));
        }
    }

    #[tokio::test]
    async fn dispatch_get_current_selection_when_no_selection() {
        let (state, tx, _rx) = create_test_state();

        let result = dispatch_tool("getCurrentSelection", &HashMap::new(), &state, &tx).await;

        assert!(!result.is_error);
        let content = &result.content[0];
        if let super::super::protocol::ToolContent::Text { text } = content {
            assert!(text.contains("No selection"));
        }
    }

    #[tokio::test]
    async fn dispatch_get_open_editors_returns_empty_list() {
        let (state, tx, _rx) = create_test_state();

        let result = dispatch_tool("getOpenEditors", &HashMap::new(), &state, &tx).await;

        assert!(!result.is_error);
        let content = &result.content[0];
        if let super::super::protocol::ToolContent::Text { text } = content {
            assert_eq!(text, "[]");
        }
    }

    #[tokio::test]
    async fn dispatch_get_workspace_folders_returns_empty_list() {
        let (state, tx, _rx) = create_test_state();

        let result = dispatch_tool("getWorkspaceFolders", &HashMap::new(), &state, &tx).await;

        assert!(!result.is_error);
        let content = &result.content[0];
        if let super::super::protocol::ToolContent::Text { text } = content {
            assert_eq!(text, "[]");
        }
    }

    #[tokio::test]
    async fn dispatch_open_file_without_path_returns_error() {
        let (state, tx, _rx) = create_test_state();

        let result = dispatch_tool("openFile", &HashMap::new(), &state, &tx).await;

        assert!(result.is_error);
        let content = &result.content[0];
        if let super::super::protocol::ToolContent::Text { text } = content {
            assert!(text.contains("Missing required parameter"));
        }
    }

    #[tokio::test]
    async fn dispatch_open_file_sends_command() {
        let (state, tx, mut rx) = create_test_state();

        let mut args = HashMap::new();
        args.insert("filePath".to_string(), serde_json::json!("test.rs"));
        args.insert("line".to_string(), serde_json::json!(42));

        let result = dispatch_tool("openFile", &args, &state, &tx).await;

        assert!(!result.is_error);

        // Verify command was sent
        let cmd = rx.try_recv().unwrap();
        match cmd {
            IdeCommand::OpenFile { path, line } => {
                assert_eq!(path, "test.rs");
                assert_eq!(line, Some(42));
            }
        }
    }
}
