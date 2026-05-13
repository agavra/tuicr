use std::borrow::Cow;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use chrono::Utc;
use rmcp::model::{
    CallToolRequestParams, CallToolResult, Content, Implementation, JsonObject, ListToolsResult,
    PaginatedRequestParams, ServerCapabilities, ServerInfo, Tool,
};
use rmcp::service::{MaybeSendFuture, RequestContext};
use rmcp::transport::stdio;
use rmcp::{ErrorData as McpError, RoleServer, ServerHandler, ServiceExt};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::app::{App, CommentTypeDefinition, DiffSource};
use crate::config::{self, CommentTypeConfig, ConfigLoadOutcome};
use crate::error::{Result, TuicrError};
use crate::model::{
    ClearScope, Comment, CommentType, DiffFile, DiffLine, FileStatus, LineOrigin, LineRange,
    LineSide, ReviewSession, SessionDiffSource,
};
use crate::output::generate_export_content;
use crate::persistence::{load_latest_session_for_context, save_session};
use crate::theme::resolve_theme_with_config;
use crate::tuicrignore;
use crate::vcs::{VcsInfo, detect_vcs};

const SERVER_NAME: &str = "tuicr-mcp";
const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");

static CWD_LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum McpDiffSource {
    #[default]
    WorkingTree,
    Staged,
    Unstaged,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OpenReviewArgs {
    repo_path: Option<PathBuf>,
    diff_source: Option<McpDiffSource>,
    revisions: Option<String>,
    include_working_tree: Option<bool>,
    path: Option<String>,
    file: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SessionIdArgs {
    session_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GetReviewArgs {
    session_id: Option<String>,
    repo_path: Option<PathBuf>,
    diff_source: Option<McpDiffSource>,
    revisions: Option<String>,
    include_working_tree: Option<bool>,
    path: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FileDiffArgs {
    session_id: String,
    path: PathBuf,
    max_lines: Option<usize>,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum CommentScope {
    Review,
    File,
    Line,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AddCommentArgs {
    session_id: String,
    scope: CommentScope,
    path: Option<PathBuf>,
    line: Option<u32>,
    end_line: Option<u32>,
    side: Option<LineSide>,
    #[serde(rename = "type")]
    comment_type: Option<String>,
    body: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SetReviewedArgs {
    session_id: String,
    path: PathBuf,
    reviewed: bool,
}

#[derive(Clone)]
pub struct TuicrMcpServer {
    default_repo_path: PathBuf,
    sessions: Arc<Mutex<HashMap<String, ReviewState>>>,
    persist_sessions: bool,
}

#[derive(Clone)]
struct ReviewState {
    session: ReviewSession,
    diff_files: Vec<DiffFile>,
    diff_source: DiffSource,
    comment_types: Vec<CommentTypeDefinition>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AgentReviewSession {
    id: String,
    repo_path: PathBuf,
    branch_name: Option<String>,
    base_commit: String,
    diff_source: String,
    created_at: String,
    updated_at: String,
    review_comments: Vec<AgentReviewComment>,
    files: Vec<AgentReviewFile>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AgentReviewFile {
    path: PathBuf,
    old_path: Option<PathBuf>,
    new_path: Option<PathBuf>,
    status: &'static str,
    reviewed: bool,
    is_binary: bool,
    is_too_large: bool,
    added_lines: usize,
    removed_lines: usize,
    hunk_count: usize,
    file_comments: Vec<AgentReviewComment>,
    line_comments: Vec<AgentReviewComment>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AgentReviewComment {
    id: String,
    #[serde(rename = "type")]
    comment_type: String,
    body: String,
    created_at: String,
    scope: CommentScope,
    path: Option<PathBuf>,
    line: Option<u32>,
    end_line: Option<u32>,
    side: LineSide,
}

struct CwdGuard {
    original: PathBuf,
}

impl CwdGuard {
    fn enter(path: &Path) -> Result<Self> {
        let original = std::env::current_dir()?;
        std::env::set_current_dir(path)?;
        Ok(Self { original })
    }
}

impl Drop for CwdGuard {
    fn drop(&mut self) {
        let _ = std::env::set_current_dir(&self.original);
    }
}

impl TuicrMcpServer {
    pub fn new(default_repo_path: PathBuf) -> Self {
        Self {
            default_repo_path,
            sessions: Arc::new(Mutex::new(HashMap::new())),
            persist_sessions: true,
        }
    }

    #[cfg(test)]
    fn new_ephemeral(default_repo_path: PathBuf) -> Self {
        Self {
            default_repo_path,
            sessions: Arc::new(Mutex::new(HashMap::new())),
            persist_sessions: false,
        }
    }

    fn remember(&self, state: ReviewState) -> Result<AgentReviewSession> {
        let payload = state.payload();
        self.sessions
            .lock()
            .map_err(|_| TuicrError::Io(std::io::Error::other("MCP session store poisoned")))?
            .insert(state.session.id.clone(), state);
        Ok(payload)
    }

    fn with_state<R>(
        &self,
        session_id: &str,
        f: impl FnOnce(&mut ReviewState) -> Result<R>,
    ) -> Result<R> {
        let mut sessions = self
            .sessions
            .lock()
            .map_err(|_| TuicrError::Io(std::io::Error::other("MCP session store poisoned")))?;
        let state = sessions
            .get_mut(session_id)
            .ok_or_else(|| TuicrError::Io(std::io::Error::other("Unknown review session")))?;
        let result = f(state)?;
        if self.persist_sessions {
            save_session(&state.session)?;
        }
        Ok(result)
    }

    fn open_review(&self, args: OpenReviewArgs) -> Result<AgentReviewSession> {
        let repo_path = args
            .repo_path
            .clone()
            .unwrap_or_else(|| self.default_repo_path.clone());
        let state = load_review_state(&repo_path, args)?;
        self.remember(state)
    }

    fn get_review(&self, args: GetReviewArgs) -> Result<AgentReviewSession> {
        match (args.session_id, args.repo_path) {
            (Some(session_id), None) => self.with_state(&session_id, |state| Ok(state.payload())),
            (None, Some(repo_path)) => self.open_review(OpenReviewArgs {
                repo_path: Some(repo_path),
                diff_source: args.diff_source,
                revisions: args.revisions,
                include_working_tree: args.include_working_tree,
                path: args.path,
                file: None,
            }),
            (Some(_), Some(_)) => Err(TuicrError::Io(std::io::Error::other(
                "get_review expects either sessionId or repoPath, not both",
            ))),
            (None, None) => Err(TuicrError::Io(std::io::Error::other(
                "get_review expects either sessionId or repoPath",
            ))),
        }
    }

    fn get_file_diff(&self, args: FileDiffArgs) -> Result<Value> {
        self.with_state(&args.session_id, |state| {
            let file = state
                .diff_files
                .iter()
                .find(|file| file.display_path() == &args.path)
                .ok_or_else(|| {
                    TuicrError::Io(std::io::Error::other(format!(
                        "Unknown review file: {}",
                        args.path.display()
                    )))
                })?;
            let (diff, truncated) = render_file_diff(file, args.max_lines.unwrap_or(500));
            Ok(json!({
                "session": state.payload(),
                "path": args.path,
                "diff": diff,
                "truncated": truncated,
            }))
        })
    }

    fn add_comment(&self, args: AddCommentArgs) -> Result<AgentReviewSession> {
        if args.body.trim().is_empty() {
            return Err(TuicrError::Io(std::io::Error::other(
                "Comment body is required",
            )));
        }
        self.with_state(&args.session_id, |state| {
            let comment_type = CommentType::from_id(args.comment_type.as_deref().unwrap_or("note"));
            match args.scope {
                CommentScope::Review => {
                    state.session.review_comments.push(Comment::new(
                        args.body.trim().to_string(),
                        comment_type,
                        None,
                    ));
                }
                CommentScope::File => {
                    let path = args.path.ok_or_else(|| {
                        TuicrError::Io(std::io::Error::other("File comments require path"))
                    })?;
                    let review = state.session.get_file_mut(&path).ok_or_else(|| {
                        TuicrError::Io(std::io::Error::other(format!(
                            "Unknown review file: {}",
                            path.display()
                        )))
                    })?;
                    review.add_file_comment(Comment::new(
                        args.body.trim().to_string(),
                        comment_type,
                        None,
                    ));
                }
                CommentScope::Line => {
                    let path = args.path.ok_or_else(|| {
                        TuicrError::Io(std::io::Error::other("Line comments require path"))
                    })?;
                    let line = args.line.ok_or_else(|| {
                        TuicrError::Io(std::io::Error::other("Line comments require line"))
                    })?;
                    let side = args.side.unwrap_or(LineSide::New);
                    let review = state.session.get_file_mut(&path).ok_or_else(|| {
                        TuicrError::Io(std::io::Error::other(format!(
                            "Unknown review file: {}",
                            path.display()
                        )))
                    })?;
                    if let Some(end_line) = args.end_line.filter(|end| *end != line) {
                        let range = LineRange::new(line, end_line);
                        review.add_line_comment(
                            range.end,
                            Comment::new_with_range(
                                args.body.trim().to_string(),
                                comment_type,
                                Some(side),
                                range,
                            ),
                        );
                    } else {
                        review.add_line_comment(
                            line,
                            Comment::new(args.body.trim().to_string(), comment_type, Some(side)),
                        );
                    }
                }
            }
            state.session.updated_at = Utc::now();
            Ok(state.payload())
        })
    }

    fn set_file_reviewed(&self, args: SetReviewedArgs) -> Result<AgentReviewSession> {
        self.with_state(&args.session_id, |state| {
            let review = state.session.get_file_mut(&args.path).ok_or_else(|| {
                TuicrError::Io(std::io::Error::other(format!(
                    "Unknown review file: {}",
                    args.path.display()
                )))
            })?;
            review.reviewed = args.reviewed;
            state.session.updated_at = Utc::now();
            Ok(state.payload())
        })
    }

    fn clear_review(&self, args: SessionIdArgs) -> Result<AgentReviewSession> {
        self.with_state(&args.session_id, |state| {
            state
                .session
                .clear_comments(ClearScope::CommentsAndReviewed);
            state.session.updated_at = Utc::now();
            Ok(state.payload())
        })
    }

    fn export_review(&self, args: SessionIdArgs) -> Result<String> {
        self.with_state(&args.session_id, |state| {
            generate_export_content(
                &state.session,
                &state.diff_source,
                &state.comment_types,
                true,
            )
        })
    }
}

impl ServerHandler for TuicrMcpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new(SERVER_NAME, SERVER_VERSION))
            .with_instructions("Use tuicr tools to review local repository changes, add comments, mark files reviewed, and export agent-ready Markdown.")
    }

    fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl std::future::Future<Output = std::result::Result<ListToolsResult, McpError>>
    + MaybeSendFuture
    + '_ {
        std::future::ready(Ok(ListToolsResult::with_all_items(tools())))
    }

    fn get_tool(&self, name: &str) -> Option<Tool> {
        tools().into_iter().find(|tool| tool.name == name)
    }

    fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> impl std::future::Future<Output = std::result::Result<CallToolResult, McpError>>
    + MaybeSendFuture
    + '_ {
        let server = self.clone();
        async move {
            let name = request.name.as_ref();
            let args = request.arguments.unwrap_or_default();
            match name {
                "open_review" => {
                    let session = server.open_review(parse_args(args)?).map_err(mcp_error)?;
                    Ok(session_result(session, "Opened tuicr review."))
                }
                "get_review" => {
                    let session = server.get_review(parse_args(args)?).map_err(mcp_error)?;
                    Ok(session_result(session, "Loaded tuicr review."))
                }
                "get_file_diff" => {
                    let payload = server.get_file_diff(parse_args(args)?).map_err(mcp_error)?;
                    let diff = payload
                        .get("diff")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                    Ok(tool_result(vec![Content::text(diff)], payload))
                }
                "add_comment" => {
                    let session = server.add_comment(parse_args(args)?).map_err(mcp_error)?;
                    Ok(session_result(session, "Comment added."))
                }
                "set_file_reviewed" => {
                    let session = server
                        .set_file_reviewed(parse_args(args)?)
                        .map_err(mcp_error)?;
                    Ok(session_result(session, "Review state updated."))
                }
                "clear_review" => {
                    let session = server.clear_review(parse_args(args)?).map_err(mcp_error)?;
                    Ok(session_result(session, "Review cleared."))
                }
                "export_review" => {
                    let markdown = server.export_review(parse_args(args)?).map_err(mcp_error)?;
                    Ok(tool_result(
                        vec![Content::text(markdown.clone())],
                        json!({ "markdown": markdown }),
                    ))
                }
                _ => Err(McpError::invalid_params(
                    format!("Unknown tuicr tool: {name}"),
                    None,
                )),
            }
        }
    }
}

pub fn run_stdio_blocking() -> anyhow::Result<()> {
    let default_repo_path = std::env::current_dir()?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_io()
        .enable_time()
        .build()?;
    runtime.block_on(async move {
        let service = TuicrMcpServer::new(default_repo_path)
            .serve(stdio())
            .await?;
        service.waiting().await?;
        anyhow::Ok(())
    })
}

fn parse_args<T: for<'de> Deserialize<'de>>(args: JsonObject) -> std::result::Result<T, McpError> {
    serde_json::from_value(Value::Object(args))
        .map_err(|error| McpError::invalid_params(format!("Invalid tool arguments: {error}"), None))
}

fn mcp_error(error: TuicrError) -> McpError {
    McpError::internal_error(error.to_string(), None)
}

fn load_review_state(repo_path: &Path, args: OpenReviewArgs) -> Result<ReviewState> {
    let _lock = CWD_LOCK
        .lock()
        .map_err(|_| TuicrError::Io(std::io::Error::other("cwd lock poisoned")))?;
    let _guard = CwdGuard::enter(repo_path)?;

    let config_outcome = config::load_config().unwrap_or_else(|_| ConfigLoadOutcome::default());
    let (theme, _) = resolve_theme_with_config(
        None,
        None,
        config_outcome
            .config
            .as_ref()
            .and_then(|cfg| cfg.theme.as_deref()),
        config_outcome
            .config
            .as_ref()
            .and_then(|cfg| cfg.theme_dark.as_deref()),
        config_outcome
            .config
            .as_ref()
            .and_then(|cfg| cfg.theme_light.as_deref()),
        config_outcome
            .config
            .as_ref()
            .and_then(|cfg| cfg.appearance.as_deref()),
    );
    let comment_types = config_outcome
        .config
        .as_ref()
        .and_then(|cfg| cfg.comment_types.clone());

    if args.diff_source.unwrap_or_default() == McpDiffSource::WorkingTree
        || args.revisions.is_some()
    {
        let app = App::new(
            theme,
            comment_types,
            false,
            args.revisions.as_deref(),
            args.revisions.is_none() || args.include_working_tree.unwrap_or(false),
            args.path.as_deref(),
            args.file.as_deref(),
        )?;
        return Ok(ReviewState {
            session: app.session,
            diff_files: app.diff_files,
            diff_source: app.diff_source,
            comment_types: app.comment_types,
        });
    }

    let vcs = detect_vcs()?;
    let vcs_info = vcs.info().clone();
    let highlighter = theme.syntax_highlighter();
    let mut diff_files = match args.diff_source.unwrap_or_default() {
        McpDiffSource::WorkingTree => vcs.get_working_tree_diff(highlighter)?,
        McpDiffSource::Staged => vcs.get_staged_diff(highlighter)?,
        McpDiffSource::Unstaged => vcs.get_unstaged_diff(highlighter)?,
    };
    diff_files = tuicrignore::filter_diff_files(&vcs_info.root_path, diff_files);
    if let Some(path_filter) = args.path.as_deref() {
        diff_files = filter_by_path(diff_files, path_filter);
    }
    if diff_files.is_empty() {
        return Err(TuicrError::NoChanges);
    }

    let session_source = match args.diff_source.unwrap_or_default() {
        McpDiffSource::WorkingTree => SessionDiffSource::StagedAndUnstaged,
        McpDiffSource::Staged => SessionDiffSource::Staged,
        McpDiffSource::Unstaged => SessionDiffSource::Unstaged,
    };
    let session = load_or_create_session(&vcs_info, session_source, None, &diff_files);
    let comment_types = comment_types_from_config(
        config_outcome
            .config
            .as_ref()
            .and_then(|cfg| cfg.comment_types.clone()),
    );

    Ok(ReviewState {
        session,
        diff_files,
        diff_source: match args.diff_source.unwrap_or_default() {
            McpDiffSource::WorkingTree => DiffSource::StagedAndUnstaged,
            McpDiffSource::Staged => DiffSource::Staged,
            McpDiffSource::Unstaged => DiffSource::Unstaged,
        },
        comment_types,
    })
}

fn load_or_create_session(
    vcs_info: &VcsInfo,
    diff_source: SessionDiffSource,
    commit_range: Option<Vec<String>>,
    diff_files: &[DiffFile],
) -> ReviewSession {
    let mut session = load_latest_session_for_context(
        &vcs_info.root_path,
        vcs_info.branch_name.as_deref(),
        &vcs_info.head_commit,
        diff_source,
        commit_range.as_deref(),
    )
    .ok()
    .flatten()
    .map(|(_, session)| session)
    .unwrap_or_else(|| {
        ReviewSession::new(
            vcs_info.root_path.clone(),
            vcs_info.head_commit.clone(),
            vcs_info.branch_name.clone(),
            diff_source,
        )
    });
    session.commit_range = commit_range;
    for file in diff_files {
        session.add_file(file.display_path().clone(), file.status, file.content_hash);
    }
    session.updated_at = Utc::now();
    session
}

fn filter_by_path(diff_files: Vec<DiffFile>, path_filter: &str) -> Vec<DiffFile> {
    let normalized = Path::new(path_filter);
    diff_files
        .into_iter()
        .filter(|file| {
            file.display_path() == normalized
                || file.display_path().starts_with(normalized)
                || file
                    .old_path
                    .as_ref()
                    .is_some_and(|path| path == normalized || path.starts_with(normalized))
        })
        .collect()
}

impl ReviewState {
    fn payload(&self) -> AgentReviewSession {
        let mut files: Vec<_> = self
            .diff_files
            .iter()
            .map(|file| self.file_payload(file))
            .collect();
        files.sort_by(|a, b| a.path.cmp(&b.path));
        AgentReviewSession {
            id: self.session.id.clone(),
            repo_path: self.session.repo_path.clone(),
            branch_name: self.session.branch_name.clone(),
            base_commit: self.session.base_commit.clone(),
            diff_source: diff_source_label(&self.diff_source).to_string(),
            created_at: self.session.created_at.to_rfc3339(),
            updated_at: self.session.updated_at.to_rfc3339(),
            review_comments: self
                .session
                .review_comments
                .iter()
                .map(|comment| comment_payload(comment, CommentScope::Review, None, None))
                .collect(),
            files,
        }
    }

    fn file_payload(&self, file: &DiffFile) -> AgentReviewFile {
        let path = file.display_path().clone();
        let review = self.session.files.get(&path);
        let (added_lines, removed_lines) = file.stat();
        let line_comments = review
            .map(|review| {
                review
                    .line_comments
                    .iter()
                    .flat_map(|(line, comments)| {
                        comments.iter().map(|comment| {
                            comment_payload(
                                comment,
                                CommentScope::Line,
                                Some(path.clone()),
                                Some(*line),
                            )
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        AgentReviewFile {
            path: path.clone(),
            old_path: file.old_path.clone(),
            new_path: file.new_path.clone(),
            status: file_status_label(file.status),
            reviewed: review.map(|review| review.reviewed).unwrap_or(false),
            is_binary: file.is_binary,
            is_too_large: file.is_too_large,
            added_lines,
            removed_lines,
            hunk_count: file.hunks.len(),
            file_comments: review
                .map(|review| {
                    review
                        .file_comments
                        .iter()
                        .map(|comment| {
                            comment_payload(comment, CommentScope::File, Some(path.clone()), None)
                        })
                        .collect()
                })
                .unwrap_or_default(),
            line_comments,
        }
    }
}

fn comment_payload(
    comment: &Comment,
    scope: CommentScope,
    path: Option<PathBuf>,
    fallback_line: Option<u32>,
) -> AgentReviewComment {
    let range = comment.line_range;
    AgentReviewComment {
        id: comment.id.clone(),
        comment_type: comment.comment_type.id().to_string(),
        body: comment.content.clone(),
        created_at: comment.created_at.to_rfc3339(),
        scope,
        path,
        line: range.map(|range| range.start).or(fallback_line),
        end_line: range.map(|range| range.end),
        side: comment.side.unwrap_or_default(),
    }
}

fn file_status_label(status: FileStatus) -> &'static str {
    match status {
        FileStatus::Added => "added",
        FileStatus::Modified => "modified",
        FileStatus::Deleted => "deleted",
        FileStatus::Renamed => "renamed",
        FileStatus::Copied => "copied",
    }
}

fn diff_source_label(diff_source: &DiffSource) -> &'static str {
    match diff_source {
        DiffSource::WorkingTree => "working_tree",
        DiffSource::Staged => "staged",
        DiffSource::Unstaged => "unstaged",
        DiffSource::StagedAndUnstaged => "staged_and_unstaged",
        DiffSource::CommitRange(_) => "commit_range",
        DiffSource::StagedUnstagedAndCommits(_) => "staged_unstaged_and_commits",
    }
}

fn render_diff_line(line: &DiffLine) -> String {
    let marker = match line.origin {
        LineOrigin::Addition => "+",
        LineOrigin::Deletion => "-",
        LineOrigin::Context => " ",
    };
    let old_line = line
        .old_lineno
        .map(|line| line.to_string())
        .unwrap_or_else(|| "-".to_string());
    let new_line = line
        .new_lineno
        .map(|line| line.to_string())
        .unwrap_or_else(|| "-".to_string());
    format!("{marker} old:{old_line} new:{new_line} {}", line.content)
}

fn render_file_diff(file: &DiffFile, max_lines: usize) -> (String, bool) {
    let (added, removed) = file.stat();
    let mut lines = vec![
        format!("File: {}", file.display_path().display()),
        format!("Status: {}", file_status_label(file.status)),
        format!("Changed lines: +{added}/-{removed}"),
        String::new(),
    ];
    if file.is_binary {
        lines.push("Binary file; no text hunks available.".to_string());
        return (lines.join("\n"), false);
    }
    if file.is_too_large {
        lines.push("File is too large; text hunks were intentionally skipped.".to_string());
        return (lines.join("\n"), false);
    }
    let mut truncated = false;
    for hunk in &file.hunks {
        if lines.len() >= max_lines {
            truncated = true;
            break;
        }
        lines.push(hunk.header.clone());
        for line in &hunk.lines {
            if lines.len() >= max_lines {
                truncated = true;
                break;
            }
            lines.push(render_diff_line(line));
        }
    }
    if truncated {
        lines.push(format!(
            "...truncated after {max_lines} lines. Call get_file_diff again with a higher maxLines value if needed."
        ));
    }
    (lines.join("\n"), truncated)
}

fn session_summary(session: &AgentReviewSession) -> String {
    let mut lines = vec![
        format!(
            "Review session {} for {}",
            session.id,
            session.repo_path.display()
        ),
        format!(
            "Branch: {}",
            session
                .branch_name
                .as_deref()
                .unwrap_or("(detached or unknown)")
        ),
        format!("Diff source: {}", session.diff_source),
        format!("Files: {}", session.files.len()),
    ];
    if session.files.is_empty() {
        lines.push("Loaded files: none".to_string());
    } else {
        lines.push("Loaded files:".to_string());
        for file in session.files.iter().take(20) {
            lines.push(format!(
                "- {} ({}, +{}/-{})",
                file.path.display(),
                file.status,
                file.added_lines,
                file.removed_lines
            ));
        }
        if session.files.len() > 20 {
            lines.push(format!("- ...and {} more", session.files.len() - 20));
        }
    }
    lines.push("Use get_file_diff with this sessionId and file path to inspect line-numbered diff content. Use add_comment and export_review to produce agent-ready Markdown.".to_string());
    lines.join("\n")
}

fn session_result(session: AgentReviewSession, message: &str) -> CallToolResult {
    let summary = session_summary(&session);
    tool_result(
        vec![Content::text(message.to_string()), Content::text(summary)],
        json!({ "session": session }),
    )
}

fn tool_result(content: Vec<Content>, structured_content: Value) -> CallToolResult {
    let mut result = CallToolResult::success(content);
    result.structured_content = Some(structured_content);
    result
}

fn tools() -> Vec<Tool> {
    vec![
        tool(
            "open_review",
            "Open tuicr review",
            "Open or refresh an agentic code review session for local changes or a revision range.",
            json_schema(json!({
                "type": "object",
                "properties": {
                    "repoPath": { "type": "string" },
                    "diffSource": { "type": "string", "enum": ["working_tree", "staged", "unstaged"] },
                    "revisions": { "type": "string" },
                    "includeWorkingTree": { "type": "boolean" },
                    "path": { "type": "string" },
                    "file": { "type": "string" }
                }
            })),
        ),
        tool(
            "get_review",
            "Get tuicr review",
            "Return an in-memory review session by sessionId, or open/refresh one by repoPath.",
            json_schema(json!({
                "type": "object",
                "properties": {
                    "sessionId": { "type": "string" },
                    "repoPath": { "type": "string" },
                    "diffSource": { "type": "string", "enum": ["working_tree", "staged", "unstaged"] },
                    "revisions": { "type": "string" },
                    "includeWorkingTree": { "type": "boolean" },
                    "path": { "type": "string" }
                }
            })),
        ),
        tool(
            "get_file_diff",
            "Get file diff",
            "Return line-numbered diff text for one file in a review session.",
            json_schema(json!({
                "type": "object",
                "required": ["sessionId", "path"],
                "properties": {
                    "sessionId": { "type": "string" },
                    "path": { "type": "string" },
                    "maxLines": { "type": "integer", "minimum": 1, "maximum": 5000 }
                }
            })),
        ),
        tool(
            "add_comment",
            "Add review comment",
            "Add a review-level, file-level, or line-level tuicr comment.",
            json_schema(json!({
                "type": "object",
                "required": ["sessionId", "scope", "body"],
                "properties": {
                    "sessionId": { "type": "string" },
                    "scope": { "type": "string", "enum": ["review", "file", "line"] },
                    "path": { "type": "string" },
                    "line": { "type": "integer", "minimum": 1 },
                    "endLine": { "type": "integer", "minimum": 1 },
                    "side": { "type": "string", "enum": ["old", "new"] },
                    "type": { "type": "string" },
                    "body": { "type": "string" }
                }
            })),
        ),
        tool(
            "set_file_reviewed",
            "Set file reviewed",
            "Mark a file as reviewed or unreviewed in a tuicr review session.",
            json_schema(json!({
                "type": "object",
                "required": ["sessionId", "path", "reviewed"],
                "properties": {
                    "sessionId": { "type": "string" },
                    "path": { "type": "string" },
                    "reviewed": { "type": "boolean" }
                }
            })),
        ),
        tool(
            "clear_review",
            "Clear review",
            "Clear all comments and reviewed marks in a review session.",
            json_schema(json!({
                "type": "object",
                "required": ["sessionId"],
                "properties": { "sessionId": { "type": "string" } }
            })),
        ),
        tool(
            "export_review",
            "Export review",
            "Export the review as agent-consumable Markdown using tuicr's CLI/TUI export format.",
            json_schema(json!({
                "type": "object",
                "required": ["sessionId"],
                "properties": { "sessionId": { "type": "string" } }
            })),
        ),
    ]
}

fn tool(
    name: &'static str,
    title: &'static str,
    description: &'static str,
    input_schema: JsonObject,
) -> Tool {
    Tool::new(
        Cow::Borrowed(name),
        Cow::Borrowed(description),
        input_schema,
    )
    .with_title(title)
}

fn json_schema(value: Value) -> JsonObject {
    match value {
        Value::Object(object) => object,
        _ => JsonObject::new(),
    }
}

fn default_comment_types() -> Vec<CommentTypeDefinition> {
    vec![
        CommentTypeDefinition {
            id: "note".to_string(),
            label: "note".to_string(),
            definition: Some("observations".to_string()),
            color: None,
        },
        CommentTypeDefinition {
            id: "suggestion".to_string(),
            label: "suggestion".to_string(),
            definition: Some("improvements".to_string()),
            color: None,
        },
        CommentTypeDefinition {
            id: "issue".to_string(),
            label: "issue".to_string(),
            definition: Some("problems to fix".to_string()),
            color: None,
        },
        CommentTypeDefinition {
            id: "praise".to_string(),
            label: "praise".to_string(),
            definition: Some("positive feedback".to_string()),
            color: None,
        },
    ]
}

fn comment_types_from_config(
    configs: Option<Vec<CommentTypeConfig>>,
) -> Vec<CommentTypeDefinition> {
    let Some(configs) = configs else {
        return default_comment_types();
    };
    let resolved: Vec<_> = configs
        .into_iter()
        .filter(|config| !config.id.trim().is_empty())
        .map(|config| CommentTypeDefinition {
            label: config.label.unwrap_or_else(|| config.id.clone()),
            id: config.id,
            definition: config.definition,
            color: None,
        })
        .collect();
    if resolved.is_empty() {
        default_comment_types()
    } else {
        resolved
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::DiffHunk;
    use tempfile::tempdir;

    fn diff_file(path: &str) -> DiffFile {
        DiffFile {
            old_path: Some(PathBuf::from(path)),
            new_path: Some(PathBuf::from(path)),
            status: FileStatus::Modified,
            hunks: vec![DiffHunk {
                header: "@@ -1,1 +1,1 @@".to_string(),
                old_start: 1,
                old_count: 1,
                new_start: 1,
                new_count: 1,
                lines: vec![
                    DiffLine {
                        origin: LineOrigin::Deletion,
                        content: "old".to_string(),
                        old_lineno: Some(1),
                        new_lineno: None,
                        highlighted_spans: None,
                    },
                    DiffLine {
                        origin: LineOrigin::Addition,
                        content: "new".to_string(),
                        old_lineno: None,
                        new_lineno: Some(1),
                        highlighted_spans: None,
                    },
                ],
            }],
            is_binary: false,
            is_too_large: false,
            is_commit_message: false,
            content_hash: 42,
        }
    }

    #[test]
    fn file_diff_renders_agent_readable_line_numbers() {
        let (rendered, truncated) = render_file_diff(&diff_file("src/main.rs"), 100);

        assert!(!truncated);
        assert!(rendered.contains("File: src/main.rs"));
        assert!(rendered.contains("- old:1 new:- old"));
        assert!(rendered.contains("+ old:- new:1 new"));
    }

    #[test]
    fn tool_list_has_no_app_resource_metadata() {
        let tool_list = tools();

        assert!(tool_list.iter().any(|tool| tool.name == "open_review"));
        assert!(tool_list.iter().all(|tool| tool.meta.is_none()));
        assert!(tool_list.iter().all(|tool| tool.name != "resource_link"));
    }

    #[test]
    fn comments_export_through_mcp_server_state() {
        let dir = tempdir().expect("tempdir");
        let server = TuicrMcpServer::new_ephemeral(dir.path().to_path_buf());
        let mut session = ReviewSession::new(
            dir.path().to_path_buf(),
            "HEAD".to_string(),
            Some("main".to_string()),
            SessionDiffSource::StagedAndUnstaged,
        );
        let file = diff_file("src/main.rs");
        session.add_file(file.display_path().clone(), file.status, file.content_hash);
        let session_id = session.id.clone();
        server
            .remember(ReviewState {
                session,
                diff_files: vec![file],
                diff_source: DiffSource::StagedAndUnstaged,
                comment_types: default_comment_types(),
            })
            .expect("remember");

        server
            .add_comment(AddCommentArgs {
                session_id: session_id.clone(),
                scope: CommentScope::Line,
                path: Some(PathBuf::from("src/main.rs")),
                line: Some(1),
                end_line: None,
                side: Some(LineSide::New),
                comment_type: Some("issue".to_string()),
                body: "Fix this line".to_string(),
            })
            .expect("add comment");

        let markdown = server
            .export_review(SessionIdArgs { session_id })
            .expect("export");

        assert!(markdown.contains("I reviewed your code"));
        assert!(markdown.contains("**[ISSUE]** `src/main.rs:1` - Fix this line"));
    }
}
