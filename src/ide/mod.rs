//! Claude Code IDE integration module.
//!
//! This module implements the Model Context Protocol (MCP) server that allows
//! Claude Code to connect to tuicr and interact with the code review session.
//!
//! ## Architecture
//!
//! ```text
//! ┌─────────────────┐     WebSocket      ┌─────────────────┐
//! │   Claude Code   │ ◄────────────────► │    IdeServer    │
//! └─────────────────┘                    └────────┬────────┘
//!                                                 │
//!                                                 │ channels
//!                                                 ▼
//!                                        ┌─────────────────┐
//!                                        │   Main Event    │
//!                                        │      Loop       │
//!                                        └─────────────────┘
//! ```
//!
//! ## Protocol
//!
//! - Transport: WebSocket on localhost (127.0.0.1)
//! - Protocol: JSON-RPC 2.0
//! - Standard: MCP (Model Context Protocol) version 2024-11-05
//! - Discovery: Lock file at ~/.claude/ide/{port}.lock
//!
//! ## Tools
//!
//! - `getCurrentSelection`: Get selected text in diff view
//! - `getOpenEditors`: List files in review session
//! - `getWorkspaceFolders`: Get repository root
//! - `getDiagnostics`: Get review comments
//! - `openFile`: Navigate to file in diff view

pub mod handlers;
pub mod lockfile;
pub mod protocol;
pub mod server;
pub mod state;
pub mod tools;

pub use server::{IdeServer, ServerError};
pub use state::{
    DiagnosticInfo, IdeState, OpenFileInfo, Selection, SharedIdeState, WorkspaceFolderInfo,
    new_shared_state,
};

/// Commands that can be sent from the IDE server to the main event loop.
#[derive(Debug, Clone)]
pub enum IdeCommand {
    /// Navigate to a file, optionally at a specific line.
    OpenFile { path: String, line: Option<u32> },
}

/// Events that can be sent from the main event loop to the IDE server.
/// Reserved for future use (selection change notifications, etc.)
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub enum IdeEvent {
    /// Selection changed in the diff viewer.
    SelectionChanged(Option<Selection>),
    /// Active file changed.
    ActiveFileChanged { file_index: usize },
    /// Files in the review session changed.
    FilesChanged,
    /// Diagnostics (comments) changed.
    DiagnosticsChanged,
}
