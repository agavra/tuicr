//! Public review service API shared by the headless MCP server and external integrations.
//!
//! This module intentionally exposes a narrow service surface instead of the
//! TUI `App` state. MCP remains one adapter over the same underlying review
//! logic; other tools can depend on this module without reimplementing tuicr
//! review semantics.

use std::path::PathBuf;

use crate::error::Result;
use crate::mcp::TuicrMcpServer;
pub use crate::mcp::{
    AddCommentArgs as AddCommentRequest, AgentCommentType as ReviewCommentTypeView,
    AgentReviewComment as ReviewCommentView, AgentReviewFile as ReviewFileView,
    AgentReviewSession as ReviewSessionView, CommentScope, FileDiffArgs as FileDiffRequest,
    FileDiffView, GetReviewArgs as GetReviewRequest, McpDiffSource as ReviewDiffSource,
    OpenReviewArgs as OpenReviewRequest, SessionIdArgs as SessionIdRequest,
    SetReviewedArgs as SetReviewedRequest,
};

#[derive(Clone)]
pub struct ReviewService {
    inner: TuicrMcpServer,
}

impl ReviewService {
    pub fn new(default_repo_path: impl Into<PathBuf>) -> Self {
        Self {
            inner: TuicrMcpServer::new(default_repo_path.into()),
        }
    }

    pub fn open_review(&self, request: OpenReviewRequest) -> Result<ReviewSessionView> {
        self.inner.open_review(request)
    }

    pub fn get_review(&self, request: GetReviewRequest) -> Result<ReviewSessionView> {
        self.inner.get_review(request)
    }

    pub fn get_file_diff(&self, request: FileDiffRequest) -> Result<FileDiffView> {
        self.inner.get_file_diff(request)
    }

    pub fn add_comment(&self, request: AddCommentRequest) -> Result<ReviewSessionView> {
        self.inner.add_comment(request)
    }

    pub fn set_file_reviewed(&self, request: SetReviewedRequest) -> Result<ReviewSessionView> {
        self.inner.set_file_reviewed(request)
    }

    pub fn clear_review(&self, request: SessionIdRequest) -> Result<ReviewSessionView> {
        self.inner.clear_review(request)
    }

    pub fn export_review(&self, request: SessionIdRequest) -> Result<String> {
        self.inner.export_review(request)
    }
}
