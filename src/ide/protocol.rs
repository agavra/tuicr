//! JSON-RPC 2.0 and MCP protocol types for Claude Code IDE integration.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// JSON-RPC 2.0 request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub id: Option<JsonRpcId>,
    pub method: String,
    #[serde(default)]
    pub params: Option<serde_json::Value>,
}

/// JSON-RPC 2.0 response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<JsonRpcId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

impl JsonRpcResponse {
    pub fn success(id: Option<JsonRpcId>, result: serde_json::Value) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id,
            result: Some(result),
            error: None,
        }
    }

    pub fn error(id: Option<JsonRpcId>, error: JsonRpcError) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id,
            result: None,
            error: Some(error),
        }
    }
}

/// JSON-RPC 2.0 notification (no id, no response expected)
/// Reserved for future use (selection change notifications, etc.)
#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcNotification {
    pub jsonrpc: String,
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<serde_json::Value>,
}

impl JsonRpcNotification {
    pub fn new(method: &str, params: Option<serde_json::Value>) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            method: method.to_string(),
            params,
        }
    }
}

/// JSON-RPC ID can be a number or string
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(untagged)]
pub enum JsonRpcId {
    Number(i64),
    String(String),
}

/// JSON-RPC 2.0 error object
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcError {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

impl JsonRpcError {
    pub fn parse_error() -> Self {
        Self {
            code: -32700,
            message: "Parse error".to_string(),
            data: None,
        }
    }

    pub fn invalid_request(msg: &str) -> Self {
        Self {
            code: -32600,
            message: format!("Invalid request: {msg}"),
            data: None,
        }
    }

    pub fn method_not_found(method: &str) -> Self {
        Self {
            code: -32601,
            message: format!("Method not found: {method}"),
            data: None,
        }
    }

    pub fn invalid_params(msg: &str) -> Self {
        Self {
            code: -32602,
            message: format!("Invalid params: {msg}"),
            data: None,
        }
    }

    #[allow(dead_code)]
    pub fn internal_error(msg: &str) -> Self {
        Self {
            code: -32603,
            message: format!("Internal error: {msg}"),
            data: None,
        }
    }
}

// ============================================================================
// MCP Protocol Types
// ============================================================================

/// MCP protocol version
pub const MCP_PROTOCOL_VERSION: &str = "2024-11-05";

/// MCP initialize request params
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InitializeParams {
    pub protocol_version: String,
    pub capabilities: ClientCapabilities,
    pub client_info: ClientInfo,
}

/// Client capabilities
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ClientCapabilities {
    #[serde(default)]
    pub roots: Option<RootsCapability>,
    #[serde(default)]
    pub sampling: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RootsCapability {
    #[serde(default)]
    pub list_changed: bool,
}

/// Client info
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientInfo {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

/// MCP initialize result
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InitializeResult {
    pub protocol_version: String,
    pub capabilities: ServerCapabilities,
    pub server_info: ServerInfo,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
}

/// Server capabilities
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ServerCapabilities {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<ToolsCapability>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resources: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompts: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolsCapability {
    #[serde(default)]
    pub list_changed: bool,
}

/// Server info
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerInfo {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

/// MCP tool definition
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Tool {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub input_schema: serde_json::Value,
}

/// MCP tools/list result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolsListResult {
    pub tools: Vec<Tool>,
}

/// MCP tools/call params
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolsCallParams {
    pub name: String,
    #[serde(default)]
    pub arguments: HashMap<String, serde_json::Value>,
}

/// MCP tools/call result
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolsCallResult {
    pub content: Vec<ToolContent>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub is_error: bool,
}

fn is_false(b: &bool) -> bool {
    !*b
}

/// Tool content item
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ToolContent {
    #[serde(rename = "text")]
    Text { text: String },
}

impl ToolContent {
    pub fn text(s: impl Into<String>) -> Self {
        Self::Text { text: s.into() }
    }
}

// ============================================================================
// IDE-specific tool result types
// ============================================================================

/// Result for getCurrentSelection tool
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SelectionResult {
    pub file_path: String,
    pub text: String,
    pub selection: SelectionRange,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SelectionRange {
    pub start: Position,
    pub end: Position,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Position {
    pub line: u32,
    pub character: u32,
}

/// Result for getOpenEditors tool
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenEditor {
    pub file_path: String,
    pub language_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_dirty: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_active: Option<bool>,
}

/// Result for getWorkspaceFolders tool
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceFolder {
    pub uri: String,
    pub name: String,
}

/// Result for getDiagnostics tool
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Diagnostic {
    pub file_path: String,
    pub range: SelectionRange,
    pub message: String,
    pub severity: DiagnosticSeverity,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DiagnosticSeverity {
    Error,
    Warning,
    Information,
    Hint,
}

/// Result for openFile tool
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenFileResult {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

// ============================================================================
// Lock file types
// ============================================================================

/// Lock file content written to ~/.claude/ide/{port}.lock
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LockFileContent {
    pub pid: u32,
    pub workspace_path: String,
    pub transport: String,
    pub ide_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ide_version: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_rpc_request_deserializes() {
        let json = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#;
        let req: JsonRpcRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.jsonrpc, "2.0");
        assert_eq!(req.method, "initialize");
        assert_eq!(req.id, Some(JsonRpcId::Number(1)));
    }

    #[test]
    fn json_rpc_request_with_string_id() {
        let json = r#"{"jsonrpc":"2.0","id":"abc-123","method":"test"}"#;
        let req: JsonRpcRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.id, Some(JsonRpcId::String("abc-123".to_string())));
    }

    #[test]
    fn json_rpc_request_without_id() {
        let json = r#"{"jsonrpc":"2.0","method":"initialized"}"#;
        let req: JsonRpcRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.id, None);
    }

    #[test]
    fn json_rpc_response_success_serializes() {
        let response = JsonRpcResponse::success(
            Some(JsonRpcId::Number(1)),
            serde_json::json!({"result": "ok"}),
        );
        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains(r#""jsonrpc":"2.0""#));
        assert!(json.contains(r#""id":1"#));
        assert!(json.contains(r#""result""#));
        assert!(!json.contains(r#""error""#));
    }

    #[test]
    fn json_rpc_response_error_serializes() {
        let response = JsonRpcResponse::error(
            Some(JsonRpcId::Number(1)),
            JsonRpcError::method_not_found("unknown"),
        );
        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains(r#""error""#));
        assert!(json.contains(r#""code":-32601"#));
        assert!(!json.contains(r#""result""#));
    }

    #[test]
    fn json_rpc_error_codes_are_correct() {
        assert_eq!(JsonRpcError::parse_error().code, -32700);
        assert_eq!(JsonRpcError::invalid_request("test").code, -32600);
        assert_eq!(JsonRpcError::method_not_found("test").code, -32601);
        assert_eq!(JsonRpcError::invalid_params("test").code, -32602);
        assert_eq!(JsonRpcError::internal_error("test").code, -32603);
    }

    #[test]
    fn initialize_params_deserializes() {
        let json = r#"{
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": {"name": "claude-code", "version": "1.0"}
        }"#;
        let params: InitializeParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.protocol_version, "2024-11-05");
        assert_eq!(params.client_info.name, "claude-code");
    }

    #[test]
    fn initialize_result_serializes() {
        let result = InitializeResult {
            protocol_version: MCP_PROTOCOL_VERSION.to_string(),
            capabilities: ServerCapabilities {
                tools: Some(ToolsCapability { list_changed: false }),
                resources: None,
                prompts: None,
            },
            server_info: ServerInfo {
                name: "tuicr".to_string(),
                version: Some("1.0".to_string()),
            },
            instructions: Some("test".to_string()),
        };
        let json = serde_json::to_string(&result).unwrap();
        assert!(json.contains(r#""protocolVersion":"2024-11-05""#));
        assert!(json.contains(r#""name":"tuicr""#));
    }

    #[test]
    fn tools_call_params_deserializes() {
        let json = r#"{"name": "getCurrentSelection", "arguments": {"key": "value"}}"#;
        let params: ToolsCallParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.name, "getCurrentSelection");
        assert_eq!(params.arguments.get("key").unwrap(), "value");
    }

    #[test]
    fn tools_call_params_with_empty_arguments() {
        let json = r#"{"name": "getOpenEditors"}"#;
        let params: ToolsCallParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.name, "getOpenEditors");
        assert!(params.arguments.is_empty());
    }

    #[test]
    fn tool_content_text_serializes() {
        let content = ToolContent::text("hello");
        let json = serde_json::to_string(&content).unwrap();
        assert!(json.contains(r#""type":"text""#));
        assert!(json.contains(r#""text":"hello""#));
    }

    #[test]
    fn tools_call_result_serializes() {
        let result = ToolsCallResult {
            content: vec![ToolContent::text("result")],
            is_error: false,
        };
        let json = serde_json::to_string(&result).unwrap();
        assert!(json.contains(r#""content""#));
        // is_error should be omitted when false
        assert!(!json.contains(r#""isError""#));
    }

    #[test]
    fn tools_call_result_with_error_serializes() {
        let result = ToolsCallResult {
            content: vec![ToolContent::text("error message")],
            is_error: true,
        };
        let json = serde_json::to_string(&result).unwrap();
        assert!(json.contains(r#""isError":true"#));
    }

    #[test]
    fn selection_result_serializes() {
        let result = SelectionResult {
            file_path: "/path/to/file.rs".to_string(),
            text: "selected text".to_string(),
            selection: SelectionRange {
                start: Position { line: 10, character: 0 },
                end: Position { line: 15, character: 0 },
            },
        };
        let json = serde_json::to_string(&result).unwrap();
        assert!(json.contains(r#""filePath":"/path/to/file.rs""#));
        assert!(json.contains(r#""line":10"#));
    }

    #[test]
    fn diagnostic_severity_serializes() {
        let diag = Diagnostic {
            file_path: "test.rs".to_string(),
            range: SelectionRange {
                start: Position { line: 1, character: 0 },
                end: Position { line: 1, character: 0 },
            },
            message: "test".to_string(),
            severity: DiagnosticSeverity::Error,
            source: Some("tuicr".to_string()),
        };
        let json = serde_json::to_string(&diag).unwrap();
        assert!(json.contains(r#""severity":"error""#));
    }

    #[test]
    fn lock_file_content_serializes() {
        let content = LockFileContent {
            pid: 12345,
            workspace_path: "/path/to/repo".to_string(),
            transport: "ws://127.0.0.1:8080".to_string(),
            ide_name: "tuicr".to_string(),
            ide_version: Some("0.7.2".to_string()),
        };
        let json = serde_json::to_string(&content).unwrap();
        assert!(json.contains(r#""pid":12345"#));
        assert!(json.contains(r#""workspacePath":"/path/to/repo""#));
        assert!(json.contains(r#""ideName":"tuicr""#));
    }
}
