use std::borrow::Cow;
use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
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

use crate::app::{App, AppStartupOptions, CommentTypeDefinition, DiffSource};
use crate::config::{self, CommentTypeConfig, ConfigLoadOutcome};
use crate::error::{Result, TuicrError};
use crate::model::review::FileReview;
use crate::model::{
    ClearScope, Comment, CommentType, DiffFile, DiffLine, FileStatus, LineOrigin, LineRange,
    LineSide, ReviewSession, SessionDiffSource,
};
use crate::output::generate_export_content;
use crate::persistence::{load_latest_session_for_context, save_session};
use crate::review_api::ReviewService;
use crate::theme::resolve_theme_with_config;
use crate::tuicrignore;
use crate::vcs::{GitBackendPreference, VcsInfo, detect_vcs};

const SERVER_NAME: &str = "tuicr-mcp";
const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");
const DEFAULT_MAX_DIFF_LINES: usize = 500;
const MAX_DIFF_LINES: usize = 5_000;

static CWD_LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum McpDiffSource {
    #[default]
    WorkingTree,
    Staged,
    Unstaged,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenReviewArgs {
    pub repo_path: Option<PathBuf>,
    pub diff_source: Option<McpDiffSource>,
    pub revisions: Option<String>,
    pub include_working_tree: Option<bool>,
    pub path: Option<String>,
    pub file: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionIdArgs {
    pub session_id: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GetReviewArgs {
    pub session_id: Option<String>,
    pub repo_path: Option<PathBuf>,
    pub diff_source: Option<McpDiffSource>,
    pub revisions: Option<String>,
    pub include_working_tree: Option<bool>,
    pub path: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileDiffArgs {
    pub session_id: String,
    pub path: PathBuf,
    pub max_lines: Option<usize>,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CommentScope {
    Review,
    File,
    Line,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AddCommentArgs {
    pub session_id: String,
    pub scope: CommentScope,
    pub path: Option<PathBuf>,
    pub line: Option<u32>,
    pub end_line: Option<u32>,
    pub side: Option<LineSide>,
    #[serde(rename = "type")]
    pub comment_type: Option<String>,
    pub body: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SetReviewedArgs {
    pub session_id: String,
    pub path: PathBuf,
    pub reviewed: bool,
}

#[derive(Clone)]
pub struct TuicrMcpServer {
    service: ReviewService,
}

#[derive(Clone)]
pub(crate) struct ReviewServiceInner {
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

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentReviewSession {
    pub id: String,
    pub repo_path: PathBuf,
    pub branch_name: Option<String>,
    pub base_commit: String,
    pub diff_source: String,
    pub created_at: String,
    pub updated_at: String,
    pub comment_types: Vec<AgentCommentType>,
    pub review_comments: Vec<AgentReviewComment>,
    pub files: Vec<AgentReviewFile>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentCommentType {
    pub id: String,
    pub label: String,
    pub definition: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentReviewFile {
    pub path: PathBuf,
    pub old_path: Option<PathBuf>,
    pub new_path: Option<PathBuf>,
    pub status: &'static str,
    pub reviewed: bool,
    pub is_binary: bool,
    pub is_too_large: bool,
    pub added_lines: usize,
    pub removed_lines: usize,
    pub hunk_count: usize,
    pub file_comments: Vec<AgentReviewComment>,
    pub line_comments: Vec<AgentReviewComment>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentReviewComment {
    pub id: String,
    #[serde(rename = "type")]
    pub comment_type: String,
    pub body: String,
    pub created_at: String,
    pub scope: CommentScope,
    pub path: Option<PathBuf>,
    pub line: Option<u32>,
    pub end_line: Option<u32>,
    pub side: LineSide,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileDiffView {
    pub session: AgentReviewSession,
    pub path: PathBuf,
    pub diff: String,
    pub truncated: bool,
    pub max_lines: usize,
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
        Self::from_review_service(ReviewService::new(default_repo_path))
    }

    fn from_review_service(service: ReviewService) -> Self {
        Self { service }
    }

    #[cfg(test)]
    fn new_ephemeral(default_repo_path: PathBuf) -> Self {
        Self {
            service: ReviewService::new_ephemeral(default_repo_path),
        }
    }

    #[cfg(test)]
    fn get_review(&self, args: GetReviewArgs) -> Result<AgentReviewSession> {
        self.service.get_review(args)
    }

    #[cfg(test)]
    fn get_file_diff(&self, args: FileDiffArgs) -> Result<FileDiffView> {
        self.service.get_file_diff(args)
    }

    #[cfg(test)]
    fn add_comment(&self, args: AddCommentArgs) -> Result<AgentReviewSession> {
        self.service.add_comment(args)
    }

    #[cfg(test)]
    fn export_review(&self, args: SessionIdArgs) -> Result<String> {
        self.service.export_review(args)
    }

    #[cfg(test)]
    fn remember(&self, state: ReviewState) -> Result<AgentReviewSession> {
        self.service.inner.remember(state)
    }
}

impl ReviewServiceInner {
    pub(crate) fn new(default_repo_path: PathBuf) -> Self {
        Self {
            default_repo_path,
            sessions: Arc::new(Mutex::new(HashMap::new())),
            persist_sessions: true,
        }
    }

    #[cfg(test)]
    pub(crate) fn new_ephemeral(default_repo_path: PathBuf) -> Self {
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

    fn with_state_read<R>(
        &self,
        session_id: &str,
        f: impl FnOnce(&ReviewState) -> Result<R>,
    ) -> Result<R> {
        let sessions = self
            .sessions
            .lock()
            .map_err(|_| TuicrError::Io(std::io::Error::other("MCP session store poisoned")))?;
        let state = sessions
            .get(session_id)
            .ok_or_else(|| TuicrError::Io(std::io::Error::other("Unknown review session")))?;
        f(state)
    }

    fn with_state_mut<R>(
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

    pub(crate) fn open_review(&self, args: OpenReviewArgs) -> Result<AgentReviewSession> {
        let repo_path = args
            .repo_path
            .clone()
            .unwrap_or_else(|| self.default_repo_path.clone());
        let state = load_review_state(&repo_path, args)?;
        self.remember(state)
    }

    pub(crate) fn get_review(&self, args: GetReviewArgs) -> Result<AgentReviewSession> {
        match (args.session_id, args.repo_path) {
            (Some(session_id), None) => {
                self.with_state_read(&session_id, |state| Ok(state.payload()))
            }
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

    pub(crate) fn get_file_diff(&self, args: FileDiffArgs) -> Result<FileDiffView> {
        self.with_state_read(&args.session_id, |state| {
            let path = state.resolve_file_path(&args.path)?;
            let file = state.diff_file(&path)?;
            let max_lines = validate_max_lines(args.max_lines)?;
            let (diff, truncated) = render_file_diff(file, max_lines);
            Ok(FileDiffView {
                session: state.payload(),
                path,
                diff,
                truncated,
                max_lines,
            })
        })
    }

    pub(crate) fn add_comment(&self, args: AddCommentArgs) -> Result<AgentReviewSession> {
        if args.body.trim().is_empty() {
            return Err(TuicrError::Io(std::io::Error::other(
                "Comment body is required",
            )));
        }
        self.with_state_mut(&args.session_id, |state| {
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
                    let raw_path = args.path.ok_or_else(|| {
                        TuicrError::Io(std::io::Error::other("File comments require path"))
                    })?;
                    let path = state.resolve_file_path(&raw_path)?;
                    let review = state.file_review_mut(&path)?;
                    review.add_file_comment(Comment::new(
                        args.body.trim().to_string(),
                        comment_type,
                        None,
                    ));
                }
                CommentScope::Line => {
                    let raw_path = args.path.ok_or_else(|| {
                        TuicrError::Io(std::io::Error::other("Line comments require path"))
                    })?;
                    let line = args.line.ok_or_else(|| {
                        TuicrError::Io(std::io::Error::other("Line comments require line"))
                    })?;
                    let side = args.side.unwrap_or(LineSide::New);
                    let path = state.resolve_file_path(&raw_path)?;
                    let review = state.file_review_mut(&path)?;
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

    pub(crate) fn set_file_reviewed(&self, args: SetReviewedArgs) -> Result<AgentReviewSession> {
        self.with_state_mut(&args.session_id, |state| {
            let path = state.resolve_file_path(&args.path)?;
            let review = state.file_review_mut(&path)?;
            review.reviewed = args.reviewed;
            state.session.updated_at = Utc::now();
            Ok(state.payload())
        })
    }

    pub(crate) fn clear_review(&self, args: SessionIdArgs) -> Result<AgentReviewSession> {
        self.with_state_mut(&args.session_id, |state| {
            state
                .session
                .clear_comments(ClearScope::CommentsAndReviewed);
            state.session.updated_at = Utc::now();
            Ok(state.payload())
        })
    }

    pub(crate) fn export_review(&self, args: SessionIdArgs) -> Result<String> {
        self.with_state_read(&args.session_id, |state| {
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
                    let args = parse_args(args)?;
                    Ok(tool_result_from(
                        server.service.open_review(args),
                        |session| session_result(session, "Opened tuicr review."),
                    ))
                }
                "get_review" => {
                    let args = parse_args(args)?;
                    Ok(tool_result_from(
                        server.service.get_review(args),
                        |session| session_result(session, "Loaded tuicr review."),
                    ))
                }
                "get_file_diff" => {
                    let args = parse_args(args)?;
                    Ok(tool_result_from(
                        server.service.get_file_diff(args),
                        |payload| {
                            tool_result(vec![Content::text(payload.diff.clone())], json!(payload))
                        },
                    ))
                }
                "add_comment" => {
                    let args = parse_args(args)?;
                    Ok(tool_result_from(
                        server.service.add_comment(args),
                        |session| session_result(session, "Comment added."),
                    ))
                }
                "set_file_reviewed" => {
                    let args = parse_args(args)?;
                    Ok(tool_result_from(
                        server.service.set_file_reviewed(args),
                        |session| session_result(session, "Review state updated."),
                    ))
                }
                "clear_review" => {
                    let args = parse_args(args)?;
                    Ok(tool_result_from(
                        server.service.clear_review(args),
                        |session| session_result(session, "Review cleared."),
                    ))
                }
                "export_review" => {
                    let args = parse_args(args)?;
                    Ok(tool_result_from(
                        server.service.export_review(args),
                        |markdown| {
                            tool_result(
                                vec![Content::text(markdown.clone())],
                                json!({ "markdown": markdown }),
                            )
                        },
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
        let review_service = ReviewService::new(default_repo_path);
        let service = TuicrMcpServer::from_review_service(review_service)
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
    let git_backend_preference = GitBackendPreference::from_config(
        config_outcome
            .config
            .as_ref()
            .and_then(|cfg| cfg.backend.as_deref()),
    );

    if args.diff_source.unwrap_or_default() == McpDiffSource::WorkingTree
        || args.revisions.is_some()
    {
        let app = App::new(
            theme,
            comment_types,
            false,
            AppStartupOptions {
                revisions: args.revisions.as_deref(),
                working_tree: args.revisions.is_none()
                    || args.include_working_tree.unwrap_or(false),
                path_filter: args.path.as_deref(),
                file_path: args.file.as_deref(),
                git_backend_preference,
            },
        )?;
        return Ok(ReviewState {
            session: app.session,
            diff_files: app.diff_files,
            diff_source: app.diff_source,
            comment_types: app.comment_types,
        });
    }

    let vcs = detect_vcs(git_backend_preference)?;
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
            comment_types: self
                .comment_types
                .iter()
                .map(comment_type_payload)
                .collect(),
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

    fn resolve_file_path(&self, path: &Path) -> Result<PathBuf> {
        let normalized = normalize_review_path(path, &self.session.repo_path);
        if self.session.files.contains_key(&normalized) {
            return Ok(normalized);
        }

        if let Some(file) = self
            .diff_files
            .iter()
            .find(|file| path_matches_file(&normalized, file))
        {
            return Ok(file.display_path().clone());
        }

        if let Some(file) = self
            .diff_files
            .iter()
            .find(|file| path_matches_file(path, file))
        {
            return Ok(file.display_path().clone());
        }

        Err(unknown_file_error(path, &self.diff_files))
    }

    fn diff_file(&self, path: &Path) -> Result<&DiffFile> {
        self.diff_files
            .iter()
            .find(|file| path_matches_file(path, file))
            .ok_or_else(|| unknown_file_error(path, &self.diff_files))
    }

    fn file_review_mut(&mut self, path: &Path) -> Result<&mut FileReview> {
        self.session
            .get_file_mut(&path.to_path_buf())
            .ok_or_else(|| unknown_file_error(path, &self.diff_files))
    }
}

fn comment_type_payload(comment_type: &CommentTypeDefinition) -> AgentCommentType {
    AgentCommentType {
        id: comment_type.id.clone(),
        label: comment_type.label.clone(),
        definition: comment_type.definition.clone(),
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

fn normalize_review_path(path: &Path, repo_path: &Path) -> PathBuf {
    let stripped = path.strip_prefix(repo_path).unwrap_or(path);
    let mut normalized = PathBuf::new();
    for component in stripped.components() {
        match component {
            Component::CurDir => {}
            Component::Normal(segment) => normalized.push(segment),
            Component::ParentDir => {
                normalized.pop();
            }
            Component::RootDir | Component::Prefix(_) => {}
        }
    }
    normalized
}

fn path_matches_file(path: &Path, file: &DiffFile) -> bool {
    file.display_path() == path
        || file.new_path.as_deref() == Some(path)
        || file.old_path.as_deref() == Some(path)
}

fn unknown_file_error(path: &Path, diff_files: &[DiffFile]) -> TuicrError {
    let mut known: Vec<_> = diff_files
        .iter()
        .take(8)
        .map(|file| file.display_path().display().to_string())
        .collect();
    if diff_files.len() > known.len() {
        known.push(format!("...and {} more", diff_files.len() - known.len()));
    }
    let hint = if known.is_empty() {
        "No files are loaded in this review.".to_string()
    } else {
        format!("Known files: {}", known.join(", "))
    };
    TuicrError::Io(std::io::Error::other(format!(
        "Unknown review file: {}. {hint}",
        path.display()
    )))
}

fn validate_max_lines(max_lines: Option<usize>) -> Result<usize> {
    let max_lines = max_lines.unwrap_or(DEFAULT_MAX_DIFF_LINES);
    if max_lines == 0 {
        return Err(TuicrError::Io(std::io::Error::other(
            "maxLines must be at least 1",
        )));
    }
    if max_lines > MAX_DIFF_LINES {
        return Err(TuicrError::Io(std::io::Error::other(format!(
            "maxLines must be at most {MAX_DIFF_LINES}"
        ))));
    }
    Ok(max_lines)
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
        format!(
            "Comment types: {}",
            session
                .comment_types
                .iter()
                .map(|comment_type| comment_type.id.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ),
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

fn tool_result_from<T>(
    result: Result<T>,
    success: impl FnOnce(T) -> CallToolResult,
) -> CallToolResult {
    match result {
        Ok(value) => success(value),
        Err(error) => tool_error(error),
    }
}

fn tool_error(error: TuicrError) -> CallToolResult {
    let message = error.to_string();
    let mut result = CallToolResult::error(vec![Content::text(message.clone())]);
    result.structured_content = Some(json!({ "error": message }));
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
                    "repoPath": { "type": "string", "description": "Local repository path. Defaults to the server process working directory." },
                    "diffSource": { "type": "string", "enum": ["working_tree", "staged", "unstaged"], "description": "Local diff source. Ignored when revisions is provided except includeWorkingTree can add local changes." },
                    "revisions": { "type": "string", "description": "Revision range or revset to review, using the active VCS syntax." },
                    "includeWorkingTree": { "type": "boolean", "description": "When revisions is set, also include local working tree changes." },
                    "path": { "type": "string", "description": "Optional repository-relative path filter." },
                    "file": { "type": "string", "description": "Optional single file path for no-VCS annotation mode." }
                },
                "additionalProperties": false
            })),
        ),
        tool(
            "get_review",
            "Get tuicr review",
            "Return an in-memory review session by sessionId, or open/refresh one by repoPath. Provide exactly one of sessionId or repoPath.",
            json_schema(json!({
                "type": "object",
                "properties": {
                    "sessionId": { "type": "string" },
                    "repoPath": { "type": "string" },
                    "diffSource": { "type": "string", "enum": ["working_tree", "staged", "unstaged"] },
                    "revisions": { "type": "string" },
                    "includeWorkingTree": { "type": "boolean" },
                    "path": { "type": "string" }
                },
                "oneOf": [
                    { "required": ["sessionId"], "not": { "required": ["repoPath"] } },
                    { "required": ["repoPath"], "not": { "required": ["sessionId"] } }
                ],
                "additionalProperties": false
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
                    "path": { "type": "string", "description": "Repository-relative or absolute path for a file loaded in the review. Old paths for renames are accepted." },
                    "maxLines": { "type": "integer", "minimum": 1, "maximum": MAX_DIFF_LINES, "description": "Maximum rendered diff lines to return. Defaults to 500." }
                },
                "additionalProperties": false
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
                    "type": { "type": "string", "description": "Comment type id. Use one of session.commentTypes; defaults to note." },
                    "body": { "type": "string" }
                },
                "additionalProperties": false
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
                    "path": { "type": "string", "description": "Repository-relative or absolute path for a file loaded in the review." },
                    "reviewed": { "type": "boolean" }
                },
                "additionalProperties": false
            })),
        ),
        tool(
            "clear_review",
            "Clear review",
            "Clear all comments and reviewed marks in a review session.",
            json_schema(json!({
                "type": "object",
                "required": ["sessionId"],
                "properties": { "sessionId": { "type": "string" } },
                "additionalProperties": false
            })),
        ),
        tool(
            "export_review",
            "Export review",
            "Export the review as agent-consumable Markdown using tuicr's CLI/TUI export format.",
            json_schema(json!({
                "type": "object",
                "required": ["sessionId"],
                "properties": { "sessionId": { "type": "string" } },
                "additionalProperties": false
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

    fn remembered_server_with_file(path: &str) -> (TuicrMcpServer, String) {
        let dir = tempdir().expect("tempdir");
        let server = TuicrMcpServer::new_ephemeral(dir.path().to_path_buf());
        let mut session = ReviewSession::new(
            dir.path().to_path_buf(),
            "HEAD".to_string(),
            Some("main".to_string()),
            SessionDiffSource::StagedAndUnstaged,
        );
        let file = diff_file(path);
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
        (server, session_id)
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
    fn tool_errors_are_reported_as_tool_results() {
        let result = tool_error(TuicrError::NoComments);

        assert_eq!(result.is_error, Some(true));
        assert!(
            result.content[0]
                .as_text()
                .is_some_and(|text| text.text.contains("No comments"))
        );
        assert!(result.structured_content.is_some());
    }

    #[test]
    fn session_payload_includes_comment_types() {
        let (server, session_id) = remembered_server_with_file("src/main.rs");

        let session = server
            .get_review(GetReviewArgs {
                session_id: Some(session_id),
                repo_path: None,
                diff_source: None,
                revisions: None,
                include_working_tree: None,
                path: None,
            })
            .expect("get review");

        assert!(session.comment_types.iter().any(|kind| kind.id == "note"));
        assert!(session.comment_types.iter().any(|kind| kind.id == "issue"));
        assert_eq!(session.files[0].added_lines, 1);
        assert_eq!(session.files[0].removed_lines, 1);
    }

    #[test]
    fn file_diff_accepts_absolute_path_under_repo() {
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

        let payload = server
            .get_file_diff(FileDiffArgs {
                session_id,
                path: dir.path().join("src/main.rs"),
                max_lines: Some(100),
            })
            .expect("get file diff");

        assert_eq!(payload.path, PathBuf::from("src/main.rs"));
        assert!(payload.diff.contains("File: src/main.rs"));
    }

    #[test]
    fn max_lines_above_limit_is_rejected() {
        let (server, session_id) = remembered_server_with_file("src/main.rs");

        let error = server
            .get_file_diff(FileDiffArgs {
                session_id,
                path: PathBuf::from("src/main.rs"),
                max_lines: Some(MAX_DIFF_LINES + 1),
            })
            .expect_err("max lines should be rejected");

        assert!(error.to_string().contains("maxLines must be at most"));
    }

    #[test]
    fn file_comment_accepts_old_path_for_renamed_file() {
        let dir = tempdir().expect("tempdir");
        let server = TuicrMcpServer::new_ephemeral(dir.path().to_path_buf());
        let mut session = ReviewSession::new(
            dir.path().to_path_buf(),
            "HEAD".to_string(),
            Some("main".to_string()),
            SessionDiffSource::StagedAndUnstaged,
        );
        let mut file = diff_file("src/new.rs");
        file.status = FileStatus::Renamed;
        file.old_path = Some(PathBuf::from("src/old.rs"));
        file.new_path = Some(PathBuf::from("src/new.rs"));
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

        let session = server
            .add_comment(AddCommentArgs {
                session_id,
                scope: CommentScope::File,
                path: Some(PathBuf::from("src/old.rs")),
                line: None,
                end_line: None,
                side: None,
                comment_type: Some("suggestion".to_string()),
                body: "Keep the rename comment attached to the reviewed file".to_string(),
            })
            .expect("add comment with old path");

        assert_eq!(session.files[0].path, PathBuf::from("src/new.rs"));
        assert_eq!(
            session.files[0].file_comments[0].path,
            Some(PathBuf::from("src/new.rs"))
        );
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
