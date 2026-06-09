use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use chrono::Utc;
use ratatui::style::Color;

use crate::config::CommentTypeConfig;
use crate::editor::EditorTarget;
use crate::error::{Result, TuicrError};
use crate::forge::context::{ContextProvider, ForgeContextProvider, VcsContextProvider};
use crate::forge::selector::PullRequestsTab;
use crate::forge::traits::{ForgeBackend, ForgeRepository};
use crate::model::{
    ClearScope, Comment, CommentType, DiffFile, DiffHunk, DiffLine, FileStatus, LineOrigin,
    LineRange, LineSide, ReviewSession, SessionDiffSource,
};
use crate::persistence::load_latest_session_for_context;
use crate::review_store::{AddCommentRequest, CommentTarget, add_comment_to_session};
use crate::syntax::SyntaxHighlighter;
use crate::theme::Theme;
use crate::update::UpdateInfo;
use crate::vcs::git::calculate_gap;
use crate::vcs::traits::VcsType;
use crate::vcs::{
    ChangeKind, CommitInfo, DiffWhitespaceMode, FileBackend, GitBackendPreference, PrNoopVcs,
    ResolvedRevisionRange, RevisionDiffTarget, VcsBackend, VcsChangeStatus, VcsInfo, detect_vcs,
};

const VISIBLE_COMMIT_COUNT: usize = 10;
const COMMIT_PAGE_SIZE: usize = 10;
pub const DEFAULT_REVIEW_WATCH_INTERVAL_MS: u64 = 1000;
pub const STAGED_SELECTION_ID: &str = "__tuicr_staged__";
pub const UNSTAGED_SELECTION_ID: &str = "__tuicr_unstaged__";
pub const GAP_EXPAND_BATCH: usize = 20;

/// Create a forge backend for the given repository.
/// Routes to the GitHub backend (via `gh`) or the GitLab backend (via `glab`)
/// based on `repo.kind`.
fn create_forge_backend(
    repo: &ForgeRepository,
    local_checkout: Option<PathBuf>,
) -> Box<dyn ForgeBackend> {
    use crate::forge::traits::ForgeKind;
    match repo.kind {
        ForgeKind::GitHub => {
            use crate::forge::github::gh::GitHubGhBackend;
            Box::new(GitHubGhBackend::new(Some(repo.clone())).with_local_checkout(local_checkout))
        }
        ForgeKind::GitLab => {
            use crate::forge::gitlab::GitLabGlabBackend;
            Box::new(GitLabGlabBackend::new(Some(repo.clone())).with_local_checkout(local_checkout))
        }
    }
}

fn char_slice(s: &str, lo_char: usize, hi_char: Option<usize>) -> &str {
    let mut indices = s.char_indices();
    let lo_byte = indices
        .by_ref()
        .nth(lo_char)
        .map(|(b, _)| b)
        .unwrap_or(s.len());
    let hi_byte = match hi_char {
        None => s.len(),
        Some(hi) if hi <= lo_char => return "",
        Some(hi) => indices
            .nth(hi - lo_char - 1)
            .map(|(b, _)| b)
            .unwrap_or(s.len()),
    };
    &s[lo_byte..hi_byte]
}

fn gap_annotation_line_count(
    is_top_of_file: bool,
    is_end_of_file: bool,
    remaining: usize,
) -> usize {
    if remaining == 0 {
        0
    } else if is_top_of_file {
        // ↑ expander, plus a HiddenLines line when remaining > batch
        if remaining > GAP_EXPAND_BATCH { 2 } else { 1 }
    } else if is_end_of_file {
        // ↓ expander, plus a HiddenLines line when remaining > batch
        if remaining > GAP_EXPAND_BATCH { 2 } else { 1 }
    } else {
        // Between hunks: ↓ + HiddenLines + ↑ when >= batch, else single ↕
        if remaining >= GAP_EXPAND_BATCH { 3 } else { 1 }
    }
}

fn profile_diff_result(result: &Result<Vec<DiffFile>>) -> String {
    match result {
        Ok(files) => format!("files={}", files.len()),
        Err(e) => format!("error={e}"),
    }
}

fn profile_commit_result(result: &Result<Vec<CommitInfo>>) -> String {
    match result {
        Ok(commits) => format!("commits={}", commits.len()),
        Err(e) => format!("error={e}"),
    }
}

fn profile_unit_result(result: &Result<()>) -> String {
    match result {
        Ok(()) => "result=ok".to_string(),
        Err(e) => format!("error={e}"),
    }
}

#[derive(Debug, Clone)]
pub enum FileTreeItem {
    Directory {
        path: String,
        depth: usize,
        expanded: bool,
    },
    File {
        file_idx: usize,
        depth: usize,
    },
}

/// Identifies a gap between hunks in a file (for context expansion)
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct GapId {
    pub file_idx: usize,
    /// Index of the hunk that this gap precedes (0 = gap before first hunk)
    pub hunk_idx: usize,
}

/// Direction of gap expansion
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ExpandDirection {
    /// ↓ Expand downward from upper boundary
    Down,
    /// ↑ Expand upward from lower boundary
    Up,
    /// ↕ Expand all remaining lines in both directions (merged expander)
    Both,
}

/// Minimum line-number column width (covers files up to 9 999 lines).
const MIN_LINENO_WIDTH: usize = 4;

/// Number of characters needed to display `n` in decimal, minimum `MIN_LINENO_WIDTH`.
pub fn lineno_width(max_lineno: u32) -> usize {
    if max_lineno == 0 {
        return MIN_LINENO_WIDTH;
    }
    let mut digits = 0;
    let mut n = max_lineno;
    while n > 0 {
        digits += 1;
        n /= 10;
    }
    digits.max(MIN_LINENO_WIDTH)
}

/// Unified diff gutter: indicator(1) + lineno(w) + space(1) + prefix(1) + space(1).
pub fn unified_gutter(w: usize) -> u16 {
    (w + 4) as u16
}

/// Side-by-side leading width before Old content: indicator(1) + lineno(w) + space(1) + prefix(1).
pub fn sbs_left_gutter(w: usize) -> u16 {
    (w + 3) as u16
}

/// Side-by-side fixed overhead (both gutters + " │ " divider).
/// Left: indicator(1) + lineno(w) + space(1) + prefix(1)
/// Right: lineno(w) + space(1) + prefix(1)
/// Divider: 3
pub fn sbs_overhead(w: usize) -> u16 {
    (2 * w + 8) as u16
}

/// X-coords of one diff content pane. SBS has Old and New; Unified has one.
#[derive(Debug, Clone, Copy)]
pub struct PaneGeom {
    pub content_x_start: u16,
    pub content_x_end: u16,
    pub content_width: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SelPoint {
    pub annotation_idx: usize,
    pub char_offset: usize,
    pub side: LineSide,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VisualSelection {
    pub anchor: SelPoint,
    pub head: SelPoint,
}

impl VisualSelection {
    pub fn collapsed(point: SelPoint) -> Self {
        Self {
            anchor: point,
            head: point,
        }
    }

    pub fn ordered(&self) -> (SelPoint, SelPoint) {
        if (self.anchor.annotation_idx, self.anchor.char_offset)
            <= (self.head.annotation_idx, self.head.char_offset)
        {
            (self.anchor, self.head)
        } else {
            (self.head, self.anchor)
        }
    }

    /// Char range `[lo, hi)` of `total_chars` covered by this selection on the
    /// annotation `ann_idx`. Returns `(0, total_chars)` for annotations
    /// strictly between start and end.
    pub fn char_range(&self, ann_idx: usize, total_chars: usize) -> (usize, usize) {
        let (start, end) = self.ordered();
        let lo = if ann_idx == start.annotation_idx {
            start.char_offset.min(total_chars)
        } else {
            0
        };
        let hi = if ann_idx == end.annotation_idx {
            end.char_offset.min(total_chars)
        } else {
            total_chars
        };
        (lo, hi)
    }
}

/// Result of checking what the cursor is on in a gap region
pub enum GapCursorHit {
    /// Cursor is on a directional expander
    Expander(GapId, ExpandDirection),
    /// Cursor is on the "N lines hidden" info line
    HiddenLines(GapId),
    /// Cursor is on already-expanded context
    ExpandedContent(GapId),
}

/// Describes what a rendered line represents - built once and used for O(1) cursor queries
#[derive(Debug, Clone)]
pub enum AnnotatedLine {
    /// Review comments section header line
    ReviewCommentsHeader,
    /// A review-level comment line (part of a multi-line comment box)
    ReviewComment { comment_idx: usize },
    /// A read-only line of a rendered remote review summary (PR review body).
    /// Renders at review scope, parallel to `ReviewComment` for local drafts.
    RemoteReviewSummaryLine { summary_idx: usize },
    /// File header line
    FileHeader { file_idx: usize },
    /// A file-level comment line (part of a multi-line comment box)
    FileComment { file_idx: usize, comment_idx: usize },
    /// Expander line showing hidden context with direction arrow
    Expander {
        gap_id: GapId,
        direction: ExpandDirection,
    },
    /// Informational line showing count of hidden lines between expanders
    HiddenLines { gap_id: GapId, count: usize },
    /// Expanded context line (muted text)
    ExpandedContext { gap_id: GapId, line_idx: usize },
    /// Hunk header (@@...@@)
    HunkHeader { file_idx: usize, hunk_idx: usize },
    /// Actual diff line with line numbers
    DiffLine {
        file_idx: usize,
        hunk_idx: usize,
        line_idx: usize,
        old_lineno: Option<u32>,
        new_lineno: Option<u32>,
    },
    /// Side-by-side paired diff line
    SideBySideLine {
        file_idx: usize,
        hunk_idx: usize,
        del_line_idx: Option<usize>,
        add_line_idx: Option<usize>,
        old_lineno: Option<u32>,
        new_lineno: Option<u32>,
    },
    /// A line comment (part of a multi-line comment box)
    LineComment {
        file_idx: usize,
        line: u32,
        side: LineSide,
        comment_idx: usize,
    },
    /// A read-only line of a rendered remote review thread. Cursor cannot
    /// edit or reply to these in v1; the annotation is informational so
    /// hit-testing and scroll math stay correct.
    RemoteThreadLine { thread_idx: usize },
    /// Binary or empty file indicator
    BinaryOrEmpty { file_idx: usize },
    /// Spacing between files
    Spacing,
}

/// Per-file index of remote threads keyed by `(line, side)` so the
/// renderer / annotation builder can place threads inline at the right
/// anchor without scanning all threads on every diff line.
#[derive(Debug, Default, Clone)]
pub struct RemoteThreadIndex {
    /// Outer key = file path (display form). Inner key =
    /// `(line, side)` where `side` is the *local* `LineSide` mapping.
    pub by_file:
        std::collections::HashMap<String, std::collections::HashMap<(u32, LineSide), Vec<usize>>>,
}

impl RemoteThreadIndex {
    #[allow(dead_code)]
    pub fn threads_at(
        &self,
        path: &std::path::Path,
        line: u32,
        side: LineSide,
    ) -> Option<&Vec<usize>> {
        self.by_file
            .get(path.to_string_lossy().as_ref())
            .and_then(|m| m.get(&(line, side)))
    }
}

/// Result of searching for a source line number in annotations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FindSourceLineResult {
    /// Exact match found at the given annotation index.
    Exact(usize),
    /// No exact match; nearest line found at the given annotation index.
    Nearest(usize),
    /// No matching lines found in the current file at all.
    NotFound,
}

/// Best-guess side for an annotation: New for everything except a Side-by-Side
/// line that only has an Old number (a deletion). Mouse cells outside content
/// annotations get New as a harmless default; range-comment line resolution
/// later filters non-diff annotations anyway.
pub fn annotation_side_default(annotation: &AnnotatedLine) -> LineSide {
    match annotation {
        AnnotatedLine::SideBySideLine {
            new_lineno: None,
            old_lineno: Some(_),
            ..
        } => LineSide::Old,
        AnnotatedLine::DiffLine {
            new_lineno: None,
            old_lineno: Some(_),
            ..
        } => LineSide::Old,
        _ => LineSide::New,
    }
}

/// Map a forge-side `PullRequestCommit` into the VCS-shaped `CommitInfo`
/// the inline commit selector renders against. We keep two arrays — the
/// forge truth in `App::pr_commits` and the rendered form in
/// `App::review_commits` — so the selector renderer stays agnostic of
/// whether the source is local or remote.
pub fn pr_commit_to_commit_info(commit: &crate::forge::traits::PullRequestCommit) -> CommitInfo {
    CommitInfo {
        id: commit.oid.clone(),
        short_id: commit.short_oid.clone(),
        branch_name: None,
        summary: commit.summary.clone(),
        body: None,
        author: commit.author.clone(),
        time: commit.timestamp.unwrap_or_else(chrono::Utc::now),
    }
}

pub fn annotation_file_idx(annotation: &AnnotatedLine) -> Option<usize> {
    match annotation {
        AnnotatedLine::FileHeader { file_idx }
        | AnnotatedLine::FileComment { file_idx, .. }
        | AnnotatedLine::HunkHeader { file_idx, .. }
        | AnnotatedLine::DiffLine { file_idx, .. }
        | AnnotatedLine::SideBySideLine { file_idx, .. }
        | AnnotatedLine::LineComment { file_idx, .. }
        | AnnotatedLine::BinaryOrEmpty { file_idx } => Some(*file_idx),
        AnnotatedLine::ReviewCommentsHeader
        | AnnotatedLine::ReviewComment { .. }
        | AnnotatedLine::RemoteReviewSummaryLine { .. }
        | AnnotatedLine::Expander { .. }
        | AnnotatedLine::HiddenLines { .. }
        | AnnotatedLine::ExpandedContext { .. }
        | AnnotatedLine::RemoteThreadLine { .. }
        | AnnotatedLine::Spacing => None,
    }
}

/// Search `line_annotations` for the annotation whose line number on the given
/// `side` best matches `target_lineno` within the file identified by
/// `current_file`. `side` selects whether to compare against `new_lineno`
/// (post-change) or `old_lineno` (pre-change).
///
/// Test-only entry point that exercises the core matching algorithm against
/// `DiffLine` / `SideBySideLine` annotations. Production code goes through
/// `App::find_source_line_in_diff`, which also resolves `ExpandedContext`
/// lines through `get_expanded_line`.
#[cfg(test)]
pub fn find_source_line(
    annotations: &[AnnotatedLine],
    current_file: usize,
    target_lineno: u32,
    side: LineSide,
) -> FindSourceLineResult {
    let mut best: Option<(usize, u32)> = None; // (index, distance)

    for (idx, annotation) in annotations.iter().enumerate() {
        let (file_idx, old_lineno, new_lineno) = match annotation {
            AnnotatedLine::DiffLine {
                file_idx,
                old_lineno,
                new_lineno,
                ..
            } => (*file_idx, *old_lineno, *new_lineno),
            AnnotatedLine::SideBySideLine {
                file_idx,
                old_lineno,
                new_lineno,
                ..
            } => (*file_idx, *old_lineno, *new_lineno),
            _ => continue,
        };
        if file_idx != current_file {
            continue;
        }
        let candidate = match side {
            LineSide::New => new_lineno,
            LineSide::Old => old_lineno,
        };
        if let Some(ln) = candidate {
            let dist = ln.abs_diff(target_lineno);
            if dist == 0 {
                return FindSourceLineResult::Exact(idx);
            }
            if best.is_none() || dist < best.unwrap().1 {
                best = Some((idx, dist));
            }
        }
    }

    match best {
        Some((idx, _)) => FindSourceLineResult::Nearest(idx),
        None => FindSourceLineResult::NotFound,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputMode {
    Normal,
    Comment,
    Command,
    Search,
    Help,
    Confirm,
    CommitSelect,
    VisualSelect,
    /// Modal listing comments that cannot be mapped to GitHub inline review
    /// comments. The user toggles between "move to summary" / "omit" for
    /// each row, then `s` advances to `SubmitConfirm`.
    SubmitResolver,
    /// Final confirmation modal before the network create-review call.
    SubmitConfirm,
    /// Event picker opened by bare `:submit` — the user chooses
    /// Comment/Approve/Request changes/Draft. Picking IS the confirmation;
    /// no `SubmitConfirm` follows (resolver still runs if any comment is
    /// unmappable).
    SubmitActionPicker,
}

/// CommandCompletionState keeps one Tab-completion run anchored to the text
/// the user typed before cycling began.
///
/// Without this state, repeated Tab presses would re-scan from the currently
/// displayed candidate and narrow the cycle to a different match set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CommandCompletionState {
    /// Prefix used to build `matches`.
    pub(crate) prefix: String,
    /// Matching command strings in the order they should cycle.
    pub(crate) matches: Vec<&'static str>,
    /// Index of the command currently displayed in the command buffer.
    pub(crate) selected: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffSource {
    WorkingTree,
    Staged,
    Unstaged,
    StagedAndUnstaged,
    CommitRange(Vec<String>),
    StagedUnstagedAndCommits(Vec<String>),
    /// Remote PR review. Carries identity + base/head SHAs needed for
    /// context expansion and status bar labels.
    ///
    /// Boxed because `PullRequestDiffSource` is much larger than the other
    /// variants; keeping it inline would balloon `DiffSource` for every
    /// local-review caller.
    PullRequest(Box<PullRequestDiffSource>),
}

impl DiffSource {
    /// Returns true when the active review target includes live worktree changes.
    ///
    /// This marks diff sources where reloading after an external editor exits
    /// can surface newly written worktree edits. Pure staged, commit-range,
    /// and pull-request reviews intentionally return false because editing the
    /// local file does not update the selected review target.
    pub fn includes_worktree_changes(&self) -> bool {
        matches!(
            self,
            Self::WorkingTree
                | Self::Unstaged
                | Self::StagedAndUnstaged
                | Self::StagedUnstagedAndCommits(_)
        )
    }
}

#[cfg(test)]
mod diff_source_tests {
    use super::*;

    #[test]
    fn unstaged_source_includes_worktree_changes() {
        assert!(DiffSource::Unstaged.includes_worktree_changes());
    }

    #[test]
    fn commit_range_source_does_not_include_worktree_changes() {
        let source = DiffSource::CommitRange(vec!["abc123".to_string()]);

        assert!(!source.includes_worktree_changes());
    }
}

/// Runtime PR identity for `DiffSource::PullRequest`.
///
/// The `PrSessionKey` portion is what scopes persistence; the additional
/// fields are display state derived once at open time so the status bar and
/// context expansion don't have to call back into the forge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PullRequestDiffSource {
    pub key: crate::forge::traits::PrSessionKey,
    pub base_sha: String,
    pub title: String,
    pub url: String,
    pub head_ref_name: String,
    pub base_ref_name: String,
    pub state: String,
    pub closed: bool,
    pub merged: bool,
}

impl PullRequestDiffSource {
    pub fn from_details(details: &crate::forge::traits::PullRequestDetails) -> Self {
        Self {
            key: crate::forge::traits::PrSessionKey::from_details(details),
            base_sha: details.base_sha.clone(),
            title: details.title.clone(),
            url: details.url.clone(),
            head_ref_name: details.head_ref_name.clone(),
            base_ref_name: details.base_ref_name.clone(),
            state: details.state.clone(),
            closed: details.closed,
            merged: details.merged_at.is_some(),
        }
    }

    pub fn read_only_reason(&self) -> Option<&'static str> {
        if self.merged {
            Some("merged")
        } else if self.closed {
            Some("closed")
        } else {
            None
        }
    }

    pub fn is_read_only(&self) -> bool {
        self.read_only_reason().is_some()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfirmAction {
    CopyAndQuit,
}

/// Push a `MappedComment` onto the appropriate bucket. Free function so the
/// preflight walk doesn't need to keep `self` borrowed mutably.
fn bucket_mapping(
    mapped: crate::forge::submit::MappedComment,
    mappable: &mut Vec<crate::forge::submit::InlineComment>,
    unmappable: &mut Vec<crate::forge::submit::UnmappableItem>,
) {
    use crate::forge::submit::{MappedComment, UnmappableItem};
    match mapped {
        MappedComment::Inline(inline) => mappable.push(inline),
        MappedComment::Unmappable {
            comment,
            file,
            reason,
        } => unmappable.push(UnmappableItem {
            comment,
            file,
            reason,
        }),
    }
}

/// In-flight `:submit*` state, populated by preflight and consumed by the
/// resolver + confirmation modals. Lives on `App::submit_state` so the same
/// preflight output can flow from resolver to confirmation without recomputing.
#[derive(Debug, Clone)]
pub struct SubmitState {
    pub event: crate::forge::submit::SubmitEvent,
    /// Comments that mapped cleanly to inline GitHub review comments.
    pub mappable: Vec<crate::forge::submit::InlineComment>,
    /// Comments that did not map, paired with the resolver action the user
    /// has chosen for each (defaults to `MoveToSummary` per spec).
    pub unmappable: Vec<crate::forge::submit::UnmappableItem>,
    pub resolver_choices: Vec<crate::forge::submit::ResolverAction>,
    /// Cursor row inside the resolver modal.
    pub resolver_cursor: usize,
    /// Originally-reviewed head SHA — used as `commit_id` in the payload.
    pub commit_id: String,
    /// When `true`, the resolver advances directly to the network call
    /// instead of routing through `SubmitConfirm`. Set by the action-picker
    /// path; left `false` for explicit `:submit <event>` invocations.
    pub skip_confirm: bool,
}

/// Event options shown in the bare-`:submit` action picker, in display
/// order. Each row pairs the user-facing label with the `SubmitEvent` it
/// dispatches.
pub const SUBMIT_PICKER_EVENTS: &[(&str, crate::forge::submit::SubmitEvent)] = &[
    ("Comment", crate::forge::submit::SubmitEvent::Comment),
    ("Approve", crate::forge::submit::SubmitEvent::Approve),
    (
        "Request changes",
        crate::forge::submit::SubmitEvent::RequestChanges,
    ),
    ("Draft", crate::forge::submit::SubmitEvent::Draft),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusedPanel {
    FileList,
    Comments,
    Diff,
    CommitSelector,
}

/// Active tab in the review target selector.
///
/// The selector internally still goes through `InputMode::CommitSelect`,
/// but it shows two tabs to the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetTab {
    Local,
    PullRequests,
}

/// Background-thread events that the PR tab consumes through `pr_load_rx`.
#[derive(Debug)]
pub enum PrLoadEvent {
    /// Result of the initial PR list fetch. `canonical` carries the
    /// repository the fetch was actually executed against — i.e. the result
    /// of the canonical (fork-parent) resolver — so the main thread can
    /// promote `App.forge_repository` to it before applying the rows.
    Initial {
        canonical: crate::forge::traits::ForgeRepository,
        result: std::result::Result<(Vec<crate::forge::traits::PullRequestSummary>, bool), String>,
    },
    /// Result of a "load more" fetch.
    LoadMore(std::result::Result<(Vec<crate::forge::traits::PullRequestSummary>, bool), String>),
}

/// In-flight PR open. Stored on `App::pr_open_state` so the selector
/// renderer can paint a spinner glyph on the matching row and the
/// handler can gate further input until the load resolves.
#[derive(Debug, Clone)]
pub struct PrOpenRequest {
    pub repository: crate::forge::traits::ForgeRepository,
    pub pr_number: u64,
    /// Wall-clock origin for the spinner animation. Derived in the App
    /// (not the renderer) so the spinner phase is stable across redraws.
    pub started_at: Instant,
}

impl PrOpenRequest {
    /// True when this in-flight open is for the given PR row.
    pub fn matches(&self, repo: &crate::forge::traits::ForgeRepository, number: u64) -> bool {
        self.pr_number == number && &self.repository == repo
    }
}

/// Result delivered from the PR-open background thread.
#[derive(Debug)]
pub enum PrOpenEvent {
    Done {
        request: PrOpenRequest,
        /// Network-only outcome. Parsing + session build runs on the main
        /// thread after this lands so `SyntaxHighlighter` does not need to
        /// cross thread boundaries.
        result: std::result::Result<
            (
                crate::forge::traits::PullRequestDetails,
                String,
                Vec<crate::forge::traits::PullRequestCommit>,
            ),
            String,
        >,
    },
}

/// A semantic anchor for the cursor — what we captured before kicking
/// off `:e`, and what we try to re-locate after the new diff lands.
/// Identifies the cursor's last-known position in terms of file path
/// and line numbers rather than the volatile annotation index.
#[derive(Debug, Clone)]
pub struct PrCursorAnchor {
    pub path: std::path::PathBuf,
    pub new_lineno: Option<u32>,
    pub old_lineno: Option<u32>,
}

/// In-flight `:e` reload of the current PR. Drives the status-bar
/// spinner and carries the cursor anchor we want to restore after the
/// reload lands.
#[derive(Debug, Clone)]
pub struct PrReloadRequest {
    pub repository: crate::forge::traits::ForgeRepository,
    pub pr_number: u64,
    pub head_sha: String,
    pub started_at: Instant,
    pub anchor: Option<PrCursorAnchor>,
}

/// Result delivered from the PR-reload background thread.
#[derive(Debug)]
pub enum PrReloadEvent {
    Done {
        request: PrReloadRequest,
        result: std::result::Result<
            (
                crate::forge::traits::PullRequestDetails,
                String,
                Vec<crate::forge::traits::PullRequestCommit>,
            ),
            String,
        >,
    },
}

/// In-flight commit-range re-fetch (PR mode). Drives a status-bar spinner
/// and carries the cursor anchor we want to restore once the range diff
/// lands.
#[derive(Debug, Clone)]
pub struct PrRangeReloadRequest {
    pub repository: crate::forge::traits::ForgeRepository,
    pub pr_number: u64,
    pub head_sha: String,
    pub start_sha: String,
    pub end_sha: String,
    pub range: (usize, usize),
    pub started_at: Instant,
    pub anchor: Option<PrCursorAnchor>,
}

/// Result delivered from the PR range re-fetch background thread.
#[derive(Debug)]
pub enum PrRangeReloadEvent {
    Done {
        request: PrRangeReloadRequest,
        result: std::result::Result<String, String>,
    },
}

/// Snapshot of the submit state needed to lock the matching local comments
/// after the background `gh api .../reviews` call returns. Captured at
/// time and stashed on `App::pr_submit_state` so the in-flight spinner has
/// something to render and the result handler can flip lifecycle state on
/// every comment that was sent.
///
/// Locked comments stay visible after submit so the user keeps seeing their
/// just-submitted work; they're pruned out of the session the next time
/// `forge_review_threads` is refreshed (via `:e` or reopen) so the freshly
/// fetched remote copies don't render alongside stale locals.
#[derive(Debug, Clone)]
pub struct SubmitInFlightState {
    pub event: crate::forge::submit::SubmitEvent,
    /// The mappable inline comments that were sent in the payload. Each
    /// carries the source `Comment.id` so we can locate it post-success.
    pub mappable: Vec<crate::forge::submit::InlineComment>,
    /// Source `Comment.id`s of unmappable items the user chose to move into
    /// the review body's "Unplaced comments" section.
    pub summary_comment_ids: Vec<String>,
    /// Source `Comment.id`s of review-level comments that were rendered into
    /// the review body.
    pub review_comment_ids: Vec<String>,
    /// Display count of moved-to-summary items, used only by the success
    /// message (kept separate from `summary_comment_ids` so message wording
    /// doesn't accidentally drift if the id list is empty).
    pub moved_to_summary_count: usize,
    /// Head SHA at preflight — used as `commit_id` in the GitHub payload and
    /// to discard stale results if the user reloaded the PR mid-submit.
    pub head_sha_snapshot: String,
    /// Repository + PR identity. Lets the stale-result guard verify the
    /// result still applies to the same PR session.
    pub repository: crate::forge::traits::ForgeRepository,
    pub pr_number: u64,
    pub started_at: Instant,
}

/// Result delivered from the create-review background thread.
#[derive(Debug)]
pub enum PrSubmitEvent {
    Done {
        repository: crate::forge::traits::ForgeRepository,
        pr_number: u64,
        head_sha: String,
        result: std::result::Result<crate::forge::traits::GhCreateReviewResponse, String>,
    },
}

/// Result delivered from the remote-thread fetch background thread. The PR
/// diff is rendered as soon as it parses; threads land asynchronously and
/// trigger a repaint via `poll_pr_threads_events`.
#[derive(Debug)]
pub enum PrThreadsEvent {
    Done {
        /// Forge identity for the request; used to discard stale results if
        /// the user has since opened a different PR.
        repository: crate::forge::traits::ForgeRepository,
        pr_number: u64,
        head_sha: String,
        threads:
            std::result::Result<Vec<crate::forge::remote_comments::RemoteReviewThread>, String>,
        summaries:
            std::result::Result<Vec<crate::forge::remote_comments::RemoteReviewSummary>, String>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffViewMode {
    Unified,
    SideBySide,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MessageType {
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub content: String,
    pub message_type: MessageType,
    /// When this message should be auto-cleared. `None` means sticky.
    pub expires_at: Option<Instant>,
}

const MESSAGE_TTL_INFO: Duration = Duration::from_secs(3);
const MESSAGE_TTL_WARNING: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SessionFileState {
    modified: Option<SystemTime>,
    len: u64,
}

impl SessionFileState {
    fn from_path(path: &Path) -> Result<Self> {
        let metadata = std::fs::metadata(path)?;
        Ok(Self {
            modified: metadata.modified().ok(),
            len: metadata.len(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct StoredComment {
    location: StoredCommentLocation,
    comment: Comment,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum StoredCommentLocation {
    Review,
    File { path: PathBuf },
    Line { path: PathBuf, line: u32 },
}

pub struct App {
    pub theme: Theme,
    pub vcs: Box<dyn VcsBackend>,
    pub vcs_info: VcsInfo,
    pub session: ReviewSession,
    pub(crate) persisted_session_snapshot: ReviewSession,
    pub(crate) session_path: Option<PathBuf>,
    pub(crate) session_file_state: Option<SessionFileState>,
    pub review_watch_interval: Option<Duration>,
    pub next_review_watch_at: Instant,
    pub(crate) ephemeral_session_paths: HashSet<PathBuf>,
    pub diff_files: Vec<DiffFile>,
    pub diff_source: DiffSource,
    pub pending_editor_target: Option<EditorTarget>,

    pub input_mode: InputMode,
    pub focused_panel: FocusedPanel,
    pub diff_view_mode: DiffViewMode,

    pub file_list_state: FileListState,
    pub comment_navigator_state: CommentNavigatorState,
    pub diff_state: DiffState,
    pub help_state: HelpState,
    pub command_buffer: String,
    pub(crate) command_completion: Option<CommandCompletionState>,
    pub search_buffer: String,
    pub last_search_pattern: Option<String>,
    pub comment_buffer: String,
    pub comment_cursor: usize,
    pub comment_type: CommentType,
    pub comment_types: Vec<CommentTypeDefinition>,
    pub comment_is_review_level: bool,
    pub comment_is_file_level: bool,
    pub comment_line: Option<(u32, LineSide)>,
    pub editing_comment_id: Option<String>,

    pub visual_selection: Option<VisualSelection>,
    /// True once the active mouse drag has actually moved off the press cell.
    /// Lets Up distinguish click from drag-back-to-anchor.
    pub mouse_drag_active: bool,
    /// Line range for range comments (used when creating comments from visual selection)
    pub comment_line_range: Option<(LineRange, LineSide)>,

    // Commit selection state
    pub commit_list: Vec<CommitInfo>,
    pub commit_list_cursor: usize,
    pub commit_list_scroll_offset: usize,
    pub commit_list_viewport_height: usize,
    /// Selected commit range as (start_idx, end_idx) inclusive, where start <= end.
    /// Indices refer to positions in commit_list.
    pub commit_selection_range: Option<(usize, usize)>,
    /// State describing how many commits are currently shown and how pagination behaves.
    pub visible_commit_count: usize,
    pub commit_page_size: usize,
    pub has_more_commit: bool,

    // Review target selector tab state. The selector reuses InputMode::CommitSelect
    // but is conceptually a "target" picker with Local and Pull Requests tabs.
    pub target_tab: TargetTab,
    /// GitHub forge repository used for all PR operations. Initially set
    /// from the local `origin` remote; replaced with the canonical (parent)
    /// repository on first PR-tab entry, or pre-empted by `--repo-url`.
    pub forge_repository: Option<ForgeRepository>,
    /// Explicit `--repo-url` override. When `Some`, the canonical resolver
    /// skips the `gh api` parent lookup and uses this value directly.
    pub repo_url_override: Option<ForgeRepository>,
    /// True once the canonical resolver has run for this session — avoids
    /// repeating the `gh api` parent lookup on every PR-tab visit.
    pub canonical_resolved: bool,
    /// State machine for the Pull Requests tab.
    pub pr_tab: PullRequestsTab,
    /// Viewport height of the PR list (set during render).
    pub pr_list_viewport_height: usize,
    /// Inner content rect of the PR list panel (set during render).
    pub pr_list_inner_area: Option<ratatui::layout::Rect>,
    /// When `Some`, the user is editing the local PR filter. Captured keys
    /// update this draft; pressing Enter commits it to the tab state.
    pub pr_filter_draft: Option<String>,
    /// Background-thread channel that delivers PR list fetch results.
    /// `Receiver` is only present while a fetch is in flight.
    pub pr_load_rx: Option<std::sync::mpsc::Receiver<PrLoadEvent>>,
    /// In-flight PR open. Drives the inline row spinner and gates further
    /// Enter presses while the network calls run on a background thread.
    pub pr_open_state: Option<PrOpenRequest>,
    /// Background-thread channel that delivers the result of a PR open.
    /// `Receiver` is only present while an open is in flight.
    pub pr_open_rx: Option<std::sync::mpsc::Receiver<PrOpenEvent>>,
    /// In-flight `:e` reload. Drives the status-bar spinner and stores
    /// the cursor anchor we want to restore after the new diff lands.
    pub pr_reload_state: Option<PrReloadRequest>,
    /// Background-thread channel that delivers the result of a PR reload.
    pub pr_reload_rx: Option<std::sync::mpsc::Receiver<PrReloadEvent>>,
    /// Forge backend instance live while in PR diff mode. Used by the
    /// context provider for gap expansion against base/head SHAs and (in a
    /// future PR) for remote comment fetch/submit.
    pub forge_backend: Option<Box<dyn ForgeBackend>>,
    /// Remote review threads fetched from the forge for the active PR.
    /// Populated asynchronously after the diff renders (see
    /// `poll_pr_threads_events`). Empty in non-PR modes and during the
    /// loading window.
    pub forge_review_threads: Vec<crate::forge::remote_comments::RemoteReviewThread>,
    /// Review-level summary comments (body text on each `PullRequestReview`).
    /// Populated alongside `forge_review_threads`. These render at the top
    /// of the diff, parallel to local `session.review_comments` drafts.
    pub forge_review_summaries: Vec<crate::forge::remote_comments::RemoteReviewSummary>,
    /// True while a background remote-thread fetch is in flight for the
    /// currently open PR. Drives the footer "Loading remote comments…"
    /// hint and skips re-fetch if `:e` is hit again before the first
    /// fetch lands.
    pub forge_review_threads_loading: bool,
    /// Background-thread channel that delivers remote-thread fetch results.
    /// `Receiver` is only present while a fetch is in flight.
    pub pr_threads_rx: Option<std::sync::mpsc::Receiver<PrThreadsEvent>>,

    /// `[forge]` section settings resolved at startup. Drives the body/footer
    /// formatting on submit. Defaults to `ForgeConfig::default()` when the
    /// section is missing.
    pub forge_config: crate::config::ForgeConfig,
    /// Local viewer identity. Stamped on new comments authored in the TUI,
    /// and compared against existing comment authors so the comment pane can
    /// distinguish "your" comments from others. Resolved from the config
    /// `username` field; defaults to `Comment::DEFAULT_AUTHOR`.
    pub username: String,
    /// In-flight `:submit*` state. `None` outside the resolver + confirmation
    /// modal flow; preflight populates it.
    pub submit_state: Option<SubmitState>,
    /// Cursor row inside the bare-`:submit` action picker modal. Only
    /// meaningful while `input_mode == SubmitActionPicker`.
    pub submit_picker_cursor: usize,
    /// In-flight `gh api .../reviews` call. `Some` while a background submit
    /// is running; cleared by `poll_pr_submit_events` once the result lands.
    /// Drives the status-bar spinner.
    pub pr_submit_state: Option<SubmitInFlightState>,
    /// Background-thread channel that delivers the create-review result.
    /// `Receiver` is only present while a submit is in flight.
    pub pr_submit_rx: Option<std::sync::mpsc::Receiver<PrSubmitEvent>>,
    /// Latest known PR head SHA from the remote. PR 5 leaves this as the
    /// open-time head so the stale-head warning never fires; PR 6 may refresh
    /// it via a pre-submit `gh pr view` to power the warning.
    pub current_pr_head: Option<String>,

    pub should_quit: bool,
    pub dirty: bool,
    pub quit_warned: bool,
    pub message: Option<Message>,
    pub pending_confirm: Option<ConfirmAction>,
    pub supports_keyboard_enhancement: bool,
    pub show_file_list: bool,
    /// `true` when the session was opened via `--all-files`. Drives the
    /// `PRISTINE · N files` chip in the status bar and prevents that chip
    /// from showing in the regular `--file <dir>` directory mode.
    pub is_pristine_mode: bool,
    /// `true` when single-file view is active. Renders only the currently
    /// focused file in the diff panel instead of the continuous-scroll
    /// concatenation. Toggled via `:focus` or `<leader>f`.
    pub is_single_file_view: bool,
    /// Set when `j` (or down arrow) tries to overflow past the last line
    /// of the current file in single-file view. The first overflow press
    /// arms the flag and parks the cursor on max; a deliberate second
    /// press then walks to the next file. Reset by any cursor move that
    /// isn't a continuing overflow attempt.
    pub primed_walk_next: bool,
    /// Symmetric inverse of [`primed_walk_next`]: armed by an underflow
    /// `k` press at the first line of the current file in single-file
    /// view; consumed by a second underflow press to walk to the
    /// previous file.
    pub primed_walk_prev: bool,
    /// Set when the Down arrow / `j` key is released after
    /// `primed_walk_next` was armed. The walk consumes only when both
    /// flags are true so held-key auto-repeat (Press, Repeat, Repeat...)
    /// never satisfies the gate. Only meaningful on terminals that
    /// support kitty `REPORT_EVENT_TYPES`; on others Release events are
    /// never emitted and the gate is bypassed via
    /// `supports_keyboard_enhancement`.
    pub down_released_since_arm: bool,
    /// Symmetric inverse of [`down_released_since_arm`] for the prev-file
    /// walk gate.
    pub up_released_since_arm: bool,
    pub cursor_line_highlight: bool,
    pub leader_key: char,
    pub scroll_offset: usize,
    pub file_list_area: Option<ratatui::layout::Rect>,
    pub comment_navigator_area: Option<ratatui::layout::Rect>,
    pub diff_area: Option<ratatui::layout::Rect>,
    /// Inner content rect of the file list panel; populated during render.
    pub file_list_inner_area: Option<ratatui::layout::Rect>,
    /// Inner content rect of the comment navigator panel; populated during render.
    pub comment_navigator_inner_area: Option<ratatui::layout::Rect>,
    /// Inner content rect of the diff panel; populated during render.
    pub diff_inner_area: Option<ratatui::layout::Rect>,
    /// Inner content rect of the commit list panel (full-screen picker or inline selector);
    /// populated during render.
    pub commit_list_inner_area: Option<ratatui::layout::Rect>,
    /// Visual-row -> annotation-index map for the diff viewport. Wrapped
    /// logical lines repeat their annotation index across multiple rows.
    pub diff_row_to_annotation: Vec<usize>,
    pub expanded_dirs: HashSet<String>,
    /// Stores lines expanded downward from the upper boundary of each gap
    pub expanded_top: HashMap<GapId, Vec<DiffLine>>,
    /// Stores lines expanded upward from the lower boundary of each gap (in ascending line order)
    pub expanded_bottom: HashMap<GapId, Vec<DiffLine>>,
    /// Cached file line counts (keyed by file_idx) to avoid repeated disk reads
    pub file_line_count_cache: HashMap<usize, u32>,
    /// Cached annotations describing what each rendered line represents
    pub line_annotations: Vec<AnnotatedLine>,
    /// Output to stdout instead of clipboard when exporting
    pub output_to_stdout: bool,
    /// Pending output to print to stdout after TUI exits
    pub pending_stdout_output: Option<String>,
    /// Calculated screen position for comment input cursor (col, row) for IME positioning.
    /// Set during render when in Comment mode, None otherwise.
    pub comment_cursor_screen_pos: Option<(u16, u16)>,
    /// During render, the comment input box may introduce lines that have no corresponding
    /// entry in `line_annotations`. This field stores `(box_start, box_len, annotations_replaced)`
    /// where `box_start` is the absolute rendered line index where the input box begins,
    /// `box_len` is the number of rendered lines the input box occupies, and
    /// `annotations_replaced` is how many annotation entries exist for the comment being
    /// edited (0 for a new comment). Used by `is_line_highlighted` to adjust annotation lookups.
    pub comment_input_annotation_offset: Option<(usize, usize, usize)>,
    /// Information about available updates (set by background check)
    pub update_info: Option<UpdateInfo>,
    /// Accumulated digit count for {N}G jump-to-line
    pub pending_count: Option<usize>,

    // Inline commit selector state (shown at top of diff view for multi-commit reviews)
    /// CommitInfo for commits in the current review (display order: newest first)
    pub review_commits: Vec<CommitInfo>,
    /// Forge-side commit list for the active PR (display order: newest first).
    /// Empty outside PR mode. Used as the source of truth for resolving a
    /// `commit_selection_range` back to (start_sha, end_sha) when toggling.
    pub pr_commits: Vec<crate::forge::traits::PullRequestCommit>,
    /// In-flight range re-fetch driven by toggling commits in the inline
    /// selector while in PR mode. Drives a spinner in the status bar.
    pub pr_range_reload_state: Option<PrRangeReloadRequest>,
    /// Background-thread channel for the active range re-fetch.
    pub pr_range_reload_rx: Option<std::sync::mpsc::Receiver<PrRangeReloadEvent>>,
    /// Whether the inline commit selector panel is visible
    pub show_commit_selector: bool,
    /// Cached individual/subrange diffs keyed by (start_idx, end_idx) into review_commits
    pub commit_diff_cache: HashMap<(usize, usize), Vec<DiffFile>>,
    /// The combined "all selected" diff, cached for quick restoration
    pub range_diff_files: Option<Vec<DiffFile>>,
    /// Saved inline selection range when entering full commit select mode via :commits
    pub saved_inline_selection: Option<(usize, usize)>,
    /// Path filter for scoping diff to a specific file or directory
    pub path_filter: Option<String>,
    /// Whether to include the "Comment types:" legend line in export
    pub export_legend: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommentTypeDefinition {
    pub id: String,
    pub label: String,
    pub definition: Option<String>,
    pub color: Option<Color>,
}

#[derive(Default)]
pub struct FileListState {
    pub list_state: ratatui::widgets::ListState,
    pub scroll_x: usize,
    pub viewport_width: usize,    // Set during render
    pub viewport_height: usize,   // Set during render
    pub max_content_width: usize, // Set during render
}

impl FileListState {
    pub fn selected(&self) -> usize {
        self.list_state.selected().unwrap_or(0)
    }

    pub fn select(&mut self, index: usize) {
        self.list_state.select(Some(index));
    }

    pub fn scroll_left(&mut self, cols: usize) {
        self.scroll_x = self.scroll_x.saturating_sub(cols);
    }

    pub fn scroll_right(&mut self, cols: usize) {
        let max_scroll_x = self.max_content_width.saturating_sub(self.viewport_width);
        self.scroll_x = (self.scroll_x.saturating_add(cols)).min(max_scroll_x);
    }
}

#[derive(Default)]
pub struct CommentNavigatorState {
    pub list_state: ratatui::widgets::ListState,
    pub scroll_x: usize,
    pub viewport_width: usize,    // Set during render
    pub viewport_height: usize,   // Set during render
    pub max_content_width: usize, // Set during render
}

impl CommentNavigatorState {
    pub fn selected(&self) -> usize {
        self.list_state.selected().unwrap_or(0)
    }

    pub fn select(&mut self, index: usize) {
        self.list_state.select(Some(index));
    }

    pub fn scroll_left(&mut self, cols: usize) {
        self.scroll_x = self.scroll_x.saturating_sub(cols);
    }

    pub fn scroll_right(&mut self, cols: usize) {
        let max_scroll_x = self.max_content_width.saturating_sub(self.viewport_width);
        self.scroll_x = (self.scroll_x.saturating_add(cols)).min(max_scroll_x);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommentNavigatorKey {
    Review {
        comment_idx: usize,
    },
    File {
        file_idx: usize,
        comment_idx: usize,
    },
    Line {
        file_idx: usize,
        line: u32,
        side: LineSide,
        comment_idx: usize,
    },
    Remote {
        thread_idx: usize,
    },
    RemoteReview {
        summary_idx: usize,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommentNavigatorKind {
    Local(CommentType),
    Remote { muted: bool },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommentNavigatorItem {
    pub key: CommentNavigatorKey,
    pub kind: CommentNavigatorKind,
    pub target_annotation: usize,
    pub path: Option<String>,
    pub line: Option<u32>,
    pub side: Option<LineSide>,
    /// Author of the underlying comment — the local commenter for local items,
    /// or the root comment's author for remote threads. `None` only when the
    /// forge did not attach an author (e.g. deleted user).
    pub author: Option<String>,
}

#[derive(Debug)]
pub struct DiffState {
    pub scroll_offset: usize,
    pub scroll_x: usize,
    pub cursor_line: usize,
    pub current_file_idx: usize,
    pub viewport_height: usize,
    pub viewport_width: usize,
    pub max_content_width: usize,
    pub wrap_lines: bool,
    /// Number of logical lines that fit in the viewport (set during render).
    /// When wrapping is enabled, this accounts for lines expanding to multiple visual rows.
    pub visible_line_count: usize,
}

impl DiffState {
    /// Number of logical lines that fit in the viewport. Uses the render-computed
    /// `visible_line_count` (which accounts for line wrapping), falling back to
    /// `viewport_height` before the first render.
    pub fn effective_visible_lines(&self) -> usize {
        if self.visible_line_count > 0 {
            self.visible_line_count
        } else {
            self.viewport_height.max(1)
        }
    }

    /// Minimum number of lines kept between the cursor and the viewport edge
    /// (equivalent to vim's `scrolloff`). Must be strictly less than half the
    /// viewport to guarantee a stable free zone after centering (zz).
    pub fn effective_scroll_margin(&self, scroll_offset: usize) -> usize {
        scroll_offset.min((self.effective_visible_lines() / 2).saturating_sub(1))
    }
}

impl Default for DiffState {
    fn default() -> Self {
        Self {
            scroll_offset: 0,
            scroll_x: 0,
            cursor_line: 0,
            current_file_idx: 0,
            viewport_height: 0,
            viewport_width: 0,
            max_content_width: 0,
            wrap_lines: true,
            visible_line_count: 0,
        }
    }
}

#[derive(Debug, Default)]
pub struct HelpState {
    pub scroll_offset: usize,
    pub viewport_height: usize,
    pub total_lines: usize, // Set during render
}

/// Represents a comment location for deletion
enum CommentLocation {
    Review {
        index: usize,
    },
    File {
        path: std::path::PathBuf,
        index: usize,
    },
    Line {
        path: std::path::PathBuf,
        line: u32,
        side: LineSide,
        index: usize,
    },
}

pub struct AppStartupOptions<'a> {
    pub revisions: Option<&'a str>,
    pub working_tree: bool,
    pub path_filter: Option<&'a str>,
    pub file_path: Option<&'a str>,
    /// Whole-repo annotation mode (`--all-files`). Mutually exclusive with
    /// the other selectors; the binary validates that before reaching here.
    pub all_files: bool,
    pub git_backend_preference: GitBackendPreference,
    pub diff_whitespace_mode: DiffWhitespaceMode,
    /// Direct PR target (`tuicr pr <target>`). Mutually exclusive with the
    /// other selectors above; the binary validates that before reaching here.
    pub pr_target: Option<&'a str>,
    /// `--repo-url` override for PR operations, already parsed into a
    /// `ForgeRepository`. When `Some`, the canonical resolver short-circuits
    /// the `gh api` parent lookup and uses this value directly.
    pub repo_url_override: Option<ForgeRepository>,
}

impl App {
    pub fn new(
        theme: Theme,
        comment_type_configs: Option<Vec<CommentTypeConfig>>,
        output_to_stdout: bool,
        options: AppStartupOptions<'_>,
    ) -> Result<Self> {
        // `tuicr pr <target>` mode: enter PR review directly, skipping the
        // selector. Errors here surface before TUI startup like other
        // startup failures.
        if let Some(target) = options.pr_target {
            return Self::new_from_pr_target(
                theme,
                comment_type_configs,
                output_to_stdout,
                target,
                options.repo_url_override.clone(),
            );
        }

        // --file mode: open a single file for annotation without VCS
        if let Some(file_path) = options.file_path {
            let vcs = Box::new(FileBackend::new(file_path)?);
            let vcs_info = vcs.info().clone();
            let highlighter = theme.syntax_highlighter();
            let diff_files = vcs.get_working_tree_diff(highlighter)?;
            let session = Self::load_or_create_session(&vcs_info, SessionDiffSource::WorkingTree);

            let mut app = Self::build(
                vcs,
                vcs_info,
                theme,
                comment_type_configs,
                output_to_stdout,
                diff_files,
                session,
                DiffSource::WorkingTree,
                InputMode::Normal,
                Vec::new(),
                None, // no path_filter
                options.repo_url_override.clone(),
            )?;

            // Hide the file list only when reviewing a single file; in
            // directory mode the user needs the list to navigate.
            if app.diff_files.len() == 1 {
                app.show_file_list = false;
            }
            app.focused_panel = FocusedPanel::Diff;

            return Ok(app);
        }

        // --all-files mode: enumerate every tracked file via `git ls-files`
        // and render in context-only mode for whole-repo annotation. Git-only
        // for MVP; non-git invocation surfaces as `NotARepository`.
        if options.all_files {
            let cwd = std::env::current_dir()
                .map_err(|_| TuicrError::NotARepository)?
                .canonicalize()
                .map_err(|_| TuicrError::NotARepository)?;
            let paths = crate::vcs::pristine::collect_tracked_paths(&cwd)?;

            let mut joined = Vec::new();
            for path in &paths {
                joined.extend_from_slice(path.as_os_str().as_encoded_bytes());
                joined.push(b'\n');
            }
            let path_hash = crate::hash::fnv1a_64(&joined);
            let head_or_none = crate::vcs::pristine::head_short_sha(&cwd);
            let base_commit = format!("pristine:{head_or_none}:{path_hash:016x}");

            let vcs = Box::new(FileBackend::new_pristine(paths, cwd.clone())?);
            let mut vcs_info = vcs.info().clone();
            vcs_info.head_commit = base_commit;
            let highlighter = theme.syntax_highlighter();
            let diff_files = vcs.get_working_tree_diff(highlighter)?;
            // `git ls-files` already honors `.gitignore`, but `.tuicrignore`
            // is tuicr-specific and not known to git. Run the same post-VCS
            // filter every other mode uses so users can elide tracked-but-
            // boring files (lockfiles, generated docs) from the review surface.
            let diff_files = Self::filter_ignored_diff_files(&cwd, diff_files);
            if diff_files.is_empty() {
                return Err(TuicrError::NoChanges);
            }
            let session = Self::load_or_create_session(&vcs_info, SessionDiffSource::Pristine);

            let mut app = Self::build(
                vcs,
                vcs_info,
                theme,
                comment_type_configs,
                output_to_stdout,
                diff_files,
                session,
                DiffSource::WorkingTree,
                InputMode::Normal,
                Vec::new(),
                None, // no path_filter
                options.repo_url_override.clone(),
            )?;

            app.is_pristine_mode = true;
            app.focused_panel = FocusedPanel::Diff;
            // Force unified view: pristine mode has no diff, so side-by-side
            // would render two identical panes. The `:diff` command is gated
            // separately so the user cannot toggle back.
            app.diff_view_mode = DiffViewMode::Unified;
            // Default `--all-files` to single-file view: every tracked file
            // in one continuous scroll is overwhelming on large repos -- both
            // visually and at startup, since pristine still loads the whole
            // tree before render. `:focus` / `<leader>f` toggles back.
            app.is_single_file_view = true;
            // Snap the viewport to the first file's start.
            let start = app.calculate_file_scroll_offset(app.diff_state.current_file_idx);
            app.diff_state.scroll_offset = start;
            app.diff_state.cursor_line = start;

            return Ok(app);
        }

        let vcs = crate::profile::time("startup.detect_vcs", || {
            detect_vcs(options.git_backend_preference, options.diff_whitespace_mode)
        })?;
        let vcs_info = vcs.info().clone();
        let highlighter =
            crate::profile::time("startup.syntax_highlighter", || theme.syntax_highlighter());
        // Determine the diff source, files, and session based on input.
        // Four paths:
        //   1. -r + -w: combined commit range and uncommitted changes
        //   2. -r only: commit range
        //   3. -w only: working tree directly (skip commit selector)
        //   4. neither: commit selection UI
        if let Some(revisions) = options.revisions {
            let revision_range = crate::profile::time_with(
                "startup.resolve_revision_range",
                || vcs.resolve_revision_range(revisions),
                |result| match result {
                    Ok(range) => format!("commits={}", range.commit_ids.len()),
                    Err(e) => format!("error={e}"),
                },
            )?;
            let commit_ids = revision_range.commit_ids.to_vec();

            if options.working_tree {
                // Combined: commit range + staged/unstaged changes
                let diff_files = Self::get_working_tree_with_commits_diff_with_ignore(
                    vcs.as_ref(),
                    &vcs_info.root_path,
                    &commit_ids,
                    highlighter,
                    options.path_filter,
                )?;
                let session = Self::load_or_create_staged_unstaged_and_commits_session(
                    &vcs_info,
                    &commit_ids,
                );
                let review_commits: Vec<CommitInfo> = crate::profile::time_with(
                    "startup.selected_commit_info",
                    || vcs.get_commits_info(&commit_ids),
                    profile_commit_result,
                )?
                .into_iter()
                .rev()
                .collect();
                // Prepend staged/unstaged entries only when the backend supports them
                let change_status = Self::get_change_status_with_ignore(
                    vcs.as_ref(),
                    &vcs_info.root_path,
                    highlighter,
                    options.path_filter,
                )?;
                let mut all_commits = Vec::new();
                if change_status.staged {
                    all_commits.push(Self::staged_commit_entry());
                }
                if change_status.unstaged {
                    all_commits.push(Self::unstaged_commit_entry());
                }
                all_commits.extend(review_commits);

                let mut app = Self::build(
                    vcs,
                    vcs_info,
                    theme,
                    comment_type_configs.clone(),
                    output_to_stdout,
                    diff_files,
                    session,
                    DiffSource::StagedUnstagedAndCommits(commit_ids),
                    InputMode::Normal,
                    Vec::new(),
                    options.path_filter,
                    options.repo_url_override.clone(),
                )?;

                app.range_diff_files = Some(app.diff_files.clone());
                app.commit_list = all_commits.clone();
                app.commit_list_cursor = 0;
                app.commit_selection_range = if all_commits.is_empty() {
                    None
                } else {
                    Some((0, all_commits.len() - 1))
                };
                app.commit_list_scroll_offset = 0;
                app.visible_commit_count = all_commits.len();
                app.has_more_commit = false;
                app.show_commit_selector = all_commits.len() > 1;
                app.commit_diff_cache.clear();
                app.review_commits = all_commits;
                app.insert_commit_message_if_single();
                app.sort_files_by_directory(true);
                app.expand_all_dirs();
                app.rebuild_annotations();

                return Ok(app);
            }

            // Resolve the revisions to commits and diff as a commit range
            let diff_files = Self::get_commit_range_diff_with_ignore(
                vcs.as_ref(),
                &vcs_info.root_path,
                &revision_range,
                highlighter,
                options.path_filter,
            )?;
            let session = Self::load_or_create_commit_range_session(&vcs_info, &commit_ids);
            // Get commit info for the inline commit selector
            let review_commits = crate::profile::time_with(
                "startup.selected_commit_info",
                || vcs.get_commits_info(&commit_ids),
                profile_commit_result,
            )?;
            // Reverse to newest-first display order
            let review_commits: Vec<CommitInfo> = review_commits.into_iter().rev().collect();

            let mut app = Self::build(
                vcs,
                vcs_info,
                theme,
                comment_type_configs.clone(),
                output_to_stdout,
                diff_files,
                session,
                DiffSource::CommitRange(commit_ids),
                InputMode::Normal,
                Vec::new(),
                options.path_filter,
                options.repo_url_override.clone(),
            )?;

            // Set up inline commit selector for multi-commit reviews
            if review_commits.len() > 1 {
                app.range_diff_files = Some(app.diff_files.clone());
                app.commit_list = review_commits.clone();
                app.commit_list_cursor = 0;
                app.commit_selection_range = Some((0, review_commits.len() - 1));
                app.commit_list_scroll_offset = 0;
                app.visible_commit_count = review_commits.len();
                app.has_more_commit = false;
                app.show_commit_selector = true;
                app.commit_diff_cache.clear();
            }
            app.review_commits = review_commits;
            app.insert_commit_message_if_single();
            app.sort_files_by_directory(true);
            app.expand_all_dirs();
            app.rebuild_annotations();

            Ok(app)
        } else if options.working_tree {
            // Skip commit selector, go straight to working tree diff
            let diff_files = Self::get_working_tree_diff_with_ignore(
                vcs.as_ref(),
                &vcs_info.root_path,
                highlighter,
                options.path_filter,
            )?;
            let session =
                Self::load_or_create_session(&vcs_info, SessionDiffSource::StagedAndUnstaged);

            let app = Self::build(
                vcs,
                vcs_info,
                theme,
                comment_type_configs,
                output_to_stdout,
                diff_files,
                session,
                DiffSource::StagedAndUnstaged,
                InputMode::Normal,
                Vec::new(),
                options.path_filter,
                options.repo_url_override.clone(),
            )?;

            Ok(app)
        } else {
            let change_status = Self::get_change_status_with_ignore(
                vcs.as_ref(),
                &vcs_info.root_path,
                highlighter,
                options.path_filter,
            )?;
            let has_staged_changes = change_status.staged;
            let has_unstaged_changes = change_status.unstaged;

            // No eager working-tree-diff fetch — the selector only needs to
            // know whether to render the Staged/Unstaged rows. The actual
            // diff loads when the user picks one (load_staged_selection /
            // load_unstaged_selection / load_staged_and_unstaged_selection).

            let commits = crate::profile::time_with(
                "startup.recent_commits",
                || vcs.get_recent_commits(0, VISIBLE_COMMIT_COUNT),
                profile_commit_result,
            )?;
            if !has_staged_changes && !has_unstaged_changes && commits.is_empty() {
                return Err(TuicrError::NoChanges);
            }

            let mut commit_list = commits.clone();
            if has_staged_changes {
                commit_list.insert(0, Self::staged_commit_entry());
            }
            if has_unstaged_changes {
                commit_list.insert(0, Self::unstaged_commit_entry());
            }

            let diff_source = if has_staged_changes && has_unstaged_changes {
                DiffSource::StagedAndUnstaged
            } else if has_staged_changes {
                DiffSource::Staged
            } else if has_unstaged_changes {
                DiffSource::Unstaged
            } else {
                DiffSource::WorkingTree
            };

            let session_source = if has_staged_changes && has_unstaged_changes {
                SessionDiffSource::StagedAndUnstaged
            } else if has_staged_changes {
                SessionDiffSource::Staged
            } else if has_unstaged_changes {
                SessionDiffSource::Unstaged
            } else {
                SessionDiffSource::WorkingTree
            };

            let session = Self::load_or_create_session(&vcs_info, session_source);

            let mut app = Self::build(
                vcs,
                vcs_info,
                theme,
                comment_type_configs,
                output_to_stdout,
                Vec::new(),
                session,
                diff_source,
                InputMode::CommitSelect,
                commit_list,
                options.path_filter,
                options.repo_url_override.clone(),
            )?;

            app.has_more_commit = commits.len() >= VISIBLE_COMMIT_COUNT;
            app.visible_commit_count = app.commit_list.len();
            Ok(app)
        }
    }

    /// Shared constructor: all `App::new` paths converge here.
    ///
    /// `pub(crate)` so render-snapshot tests in `ui::app_layout` can drive
    /// the full app through `render` without going through `App::new`'s
    /// filesystem/VCS requirements.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn build(
        vcs: Box<dyn VcsBackend>,
        vcs_info: VcsInfo,
        theme: Theme,
        comment_type_configs: Option<Vec<CommentTypeConfig>>,
        output_to_stdout: bool,
        diff_files: Vec<DiffFile>,
        mut session: ReviewSession,
        diff_source: DiffSource,
        input_mode: InputMode,
        commit_list: Vec<CommitInfo>,
        path_filter: Option<&str>,
        repo_url_override: Option<ForgeRepository>,
    ) -> Result<Self> {
        // Ensure all diff files are registered in the session. Persisted PR
        // subsets hydrate through the full PR diff first; keep subset-specific
        // hunk keys alive until the selected diff is loaded.
        let preserve_hunks = matches!(diff_source, DiffSource::PullRequest(_))
            && session.commit_selection_range.is_some();
        Self::register_diff_files(&mut session, &diff_files, preserve_hunks);

        let has_more_commit = commit_list.len() >= VISIBLE_COMMIT_COUNT;
        let visible_commit_count = if commit_list.is_empty() {
            VISIBLE_COMMIT_COUNT
        } else {
            commit_list.len()
        };

        let comment_types = Self::resolve_comment_types(&theme, comment_type_configs);
        let default_comment_type = Self::first_comment_type(&comment_types);
        let session_path = crate::persistence::storage::session_path(&session).ok();
        let session_file_state = session_path
            .as_deref()
            .filter(|path| path.exists())
            .and_then(|path| SessionFileState::from_path(path).ok());
        let persisted_session_snapshot = session.clone();

        let mut app = Self {
            theme,
            vcs,
            vcs_info,
            session,
            persisted_session_snapshot,
            session_path,
            session_file_state,
            review_watch_interval: Some(Duration::from_millis(DEFAULT_REVIEW_WATCH_INTERVAL_MS)),
            next_review_watch_at: Instant::now()
                + Duration::from_millis(DEFAULT_REVIEW_WATCH_INTERVAL_MS),
            ephemeral_session_paths: HashSet::new(),
            diff_files,
            diff_source,
            pending_editor_target: None,
            input_mode,
            focused_panel: FocusedPanel::Diff,
            diff_view_mode: DiffViewMode::Unified,
            file_list_state: FileListState::default(),
            comment_navigator_state: CommentNavigatorState::default(),
            diff_state: DiffState::default(),
            help_state: HelpState::default(),
            command_buffer: String::new(),
            command_completion: None,
            search_buffer: String::new(),
            last_search_pattern: None,
            comment_buffer: String::new(),
            comment_cursor: 0,
            comment_type: default_comment_type,
            comment_types,
            comment_is_review_level: false,
            comment_is_file_level: true,
            comment_line: None,
            editing_comment_id: None,
            visual_selection: None,
            mouse_drag_active: false,
            comment_line_range: None,
            commit_list,
            commit_list_cursor: 0,
            commit_list_scroll_offset: 0,
            commit_list_viewport_height: 0,
            commit_selection_range: None,
            visible_commit_count,
            commit_page_size: COMMIT_PAGE_SIZE,
            has_more_commit,
            target_tab: TargetTab::Local,
            forge_repository: None,
            repo_url_override,
            canonical_resolved: false,
            pr_tab: PullRequestsTab::new(None),
            pr_list_viewport_height: 0,
            pr_list_inner_area: None,
            pr_filter_draft: None,
            pr_load_rx: None,
            pr_open_state: None,
            pr_open_rx: None,
            pr_reload_state: None,
            pr_reload_rx: None,
            forge_backend: None,
            forge_review_threads: Vec::new(),
            forge_review_summaries: Vec::new(),
            forge_review_threads_loading: false,
            pr_threads_rx: None,
            forge_config: crate::config::ForgeConfig::default(),
            username: crate::model::comment::DEFAULT_AUTHOR.to_string(),
            submit_state: None,
            submit_picker_cursor: 0,
            pr_submit_state: None,
            pr_submit_rx: None,
            current_pr_head: None,
            should_quit: false,
            dirty: false,
            quit_warned: false,
            message: None,
            pending_confirm: None,
            supports_keyboard_enhancement: false,
            show_file_list: true,
            is_pristine_mode: false,
            is_single_file_view: false,
            primed_walk_next: false,
            primed_walk_prev: false,
            down_released_since_arm: false,
            up_released_since_arm: false,
            cursor_line_highlight: true,
            leader_key: crate::config::DEFAULT_LEADER_KEY,
            scroll_offset: 0,
            file_list_area: None,
            comment_navigator_area: None,
            diff_area: None,
            file_list_inner_area: None,
            comment_navigator_inner_area: None,
            diff_inner_area: None,
            commit_list_inner_area: None,
            diff_row_to_annotation: Vec::new(),
            expanded_dirs: HashSet::new(),
            expanded_top: HashMap::new(),
            expanded_bottom: HashMap::new(),
            file_line_count_cache: HashMap::new(),
            line_annotations: Vec::new(),
            output_to_stdout,
            pending_stdout_output: None,
            comment_cursor_screen_pos: None,
            comment_input_annotation_offset: None,
            update_info: None,
            pending_count: None,
            review_commits: Vec::new(),
            pr_commits: Vec::new(),
            pr_range_reload_state: None,
            pr_range_reload_rx: None,
            show_commit_selector: false,
            commit_diff_cache: HashMap::new(),
            range_diff_files: None,
            saved_inline_selection: None,
            path_filter: path_filter.map(|s| s.to_string()),
            export_legend: true,
        };
        // Auto-hide file list when path filter matches exactly one file
        if app.path_filter.is_some() && app.diff_files.len() == 1 {
            app.show_file_list = false;
            app.focused_panel = FocusedPanel::Diff;
        }
        app.sort_files_by_directory(true);
        app.expand_all_dirs();
        app.populate_file_line_count_cache();
        app.rebuild_annotations();
        app.detect_forge_repository();
        Ok(app)
    }

    /// Slug for the currently active session, derived from the session's
    /// embedded fields. Returns `None` if derivation fails (e.g., a local
    /// session pointing at a non-existent path). The slug is cheap to derive
    /// from PR sessions (no I/O) and a few stat calls for local sessions.
    pub fn session_slug(&self) -> Option<String> {
        crate::persistence::storage::slug_for_session(&self.session)
            .ok()
            .map(|s| s.to_string())
    }

    pub fn set_review_watch_interval_ms(&mut self, interval_ms: u64) {
        if interval_ms == 0 {
            self.review_watch_interval = None;
        } else {
            let interval = Duration::from_millis(interval_ms);
            self.review_watch_interval = Some(interval);
            self.next_review_watch_at = Instant::now() + interval;
        }
    }

    fn reset_persisted_session_tracking(&mut self) {
        self.session_path = crate::persistence::storage::session_path(&self.session).ok();
        self.session_file_state = self
            .session_path
            .as_deref()
            .filter(|path| path.exists())
            .and_then(|path| SessionFileState::from_path(path).ok());
        self.persisted_session_snapshot = self.session.clone();
        if let Err(e) = self.ensure_ephemeral_session_file() {
            self.set_warning(format!("Failed to initialize review session file: {e}"));
        }
    }

    fn mark_session_saved(&mut self, path: PathBuf, saved: ReviewSession) {
        self.session = saved.clone();
        self.persisted_session_snapshot = saved;
        self.session_path = Some(path.clone());
        self.session_file_state = SessionFileState::from_path(&path).ok();
        self.dirty = false;
    }

    pub fn ensure_ephemeral_session_file(&mut self) -> Result<Option<PathBuf>> {
        let path = match self.session_path.clone() {
            Some(path) => path,
            None => {
                let path = crate::persistence::storage::session_path(&self.session)?;
                self.session_path = Some(path.clone());
                path
            }
        };

        if path.exists() {
            if self.ephemeral_session_paths.contains(&path) {
                let saved_path = self.save_current_session_merging_external()?;
                return Ok(Some(saved_path));
            }
            self.session_file_state = SessionFileState::from_path(&path).ok();
            self.mark_current_session_active_at(&path);
            return Ok(None);
        }

        let saved_path = self.save_current_session_merging_external()?;
        self.ephemeral_session_paths.insert(saved_path.clone());
        Ok(Some(saved_path))
    }

    pub fn cleanup_empty_ephemeral_sessions(&mut self) -> Result<usize> {
        let mut deleted = 0;
        for path in self.ephemeral_session_paths.clone() {
            if crate::persistence::storage::delete_session_if_empty(&path)? {
                deleted += 1;
                if self.session_path.as_ref() == Some(&path) {
                    self.session_file_state = None;
                }
            }
            self.ephemeral_session_paths.remove(&path);
        }
        Ok(deleted)
    }

    pub fn clear_active_session_marker(&mut self) -> Result<()> {
        crate::persistence::storage::clear_active_session_for_pid()
    }

    /// Discard this session's persisted state and quit.
    ///
    /// Used when the only unsaved change is reviewed-file markers (no
    /// comments): rather than forcing `:q!`, we drop the persisted session so
    /// reopening starts clean, then quit. Reviewed markers are persisted
    /// eagerly, so the on-disk file is removed too.
    pub fn discard_session_and_quit(&mut self) {
        let path = self
            .session_path
            .clone()
            .or_else(|| crate::persistence::storage::session_path(&self.session).ok());
        if let Some(path) = path {
            let _ = crate::persistence::storage::delete_session(&path);
            self.ephemeral_session_paths.remove(&path);
            if self.session_path.as_ref() == Some(&path) {
                self.session_file_state = None;
            }
        }
        self.dirty = false;
        self.should_quit = true;
    }

    pub fn save_current_session_merging_external(&mut self) -> Result<PathBuf> {
        let identity = self.session.clone();
        let current = self.session.clone();
        let base = self.persisted_session_snapshot.clone();
        let (path, saved, _changed) =
            crate::persistence::storage::save_session_by_identity(&identity, |persisted| {
                let mut merged = current.clone();
                if let Some(latest) = persisted.as_ref() {
                    Self::merge_external_session_changes(&mut merged, &base, latest);
                }
                merged.updated_at = Utc::now();
                Ok((merged, ()))
            })?;
        self.mark_session_saved(path.clone(), saved);
        self.mark_current_session_active_at(&path);
        self.rebuild_annotations();
        Ok(path)
    }

    fn mark_current_session_active_at(&mut self, path: &Path) {
        if let Err(e) = crate::persistence::storage::mark_session_active(&self.session, path) {
            self.set_warning(format!("Failed to mark active review session: {e}"));
        }
    }

    pub fn poll_persisted_session_changes(&mut self) {
        let Some(interval) = self.review_watch_interval else {
            return;
        };
        let now = Instant::now();
        if now < self.next_review_watch_at {
            return;
        }
        self.next_review_watch_at = now + interval;

        // Do not mutate the session while the user is composing or editing a
        // comment. The next poll after the editor closes will merge changes.
        if self.input_mode == InputMode::Comment {
            return;
        }

        if let Err(err) = self.reload_persisted_session_if_changed(false) {
            self.set_warning(format!("Review reload failed: {err}"));
        }
    }

    pub fn reload_persisted_session_if_changed(&mut self, force: bool) -> Result<usize> {
        let path = match self.session_path.clone() {
            Some(path) => path,
            None => match crate::persistence::storage::session_path(&self.session) {
                Ok(path) => {
                    self.session_path = Some(path.clone());
                    path
                }
                Err(_) => return Ok(0),
            },
        };

        if !path.exists() {
            self.session_file_state = None;
            return Ok(0);
        }

        let state = SessionFileState::from_path(&path)?;
        if !force && self.session_file_state == Some(state) {
            return Ok(0);
        }

        let latest = crate::persistence::storage::load_session(&path)?;
        let before_count = Self::comment_count(&self.session);
        let changed = Self::merge_external_session_changes(
            &mut self.session,
            &self.persisted_session_snapshot,
            &latest,
        );
        self.persisted_session_snapshot = latest;
        self.session_file_state = Some(state);
        if changed > 0 {
            self.rebuild_annotations();
        }
        let after_count = Self::comment_count(&self.session);
        Ok(after_count.saturating_sub(before_count))
    }

    fn merge_external_session_changes(
        current: &mut ReviewSession,
        base: &ReviewSession,
        latest: &ReviewSession,
    ) -> usize {
        let mut changed = 0;

        for (path, latest_review) in &latest.files {
            if !current.files.contains_key(path) {
                current.files.insert(path.clone(), latest_review.clone());
                changed += latest_review.comment_count();
                continue;
            }

            let base_reviewed = base.files.get(path).map(|review| review.reviewed);
            if let Some(current_review) = current.files.get_mut(path)
                && Some(current_review.reviewed) == base_reviewed
                && current_review.reviewed != latest_review.reviewed
            {
                current_review.reviewed = latest_review.reviewed;
                changed += 1;
            }
        }

        let base_comments = Self::collect_stored_comments(base);
        let current_comments = Self::collect_stored_comments(current);
        let latest_comments = Self::collect_stored_comments(latest);

        for (id, latest_comment) in &latest_comments {
            match base_comments.get(id) {
                None => {
                    if !current_comments.contains_key(id) {
                        Self::upsert_stored_comment(current, latest_comment.clone());
                        changed += 1;
                    }
                }
                Some(base_comment) if latest_comment != base_comment => {
                    match current_comments.get(id) {
                        Some(current_comment) if current_comment == base_comment => {
                            Self::upsert_stored_comment(current, latest_comment.clone());
                            changed += 1;
                        }
                        None => {
                            // Local deletion wins over an external edit.
                        }
                        Some(_) => {
                            // Local edit wins over an external edit of the same comment.
                        }
                    }
                }
                Some(_) => {}
            }
        }

        for (id, base_comment) in &base_comments {
            if !latest_comments.contains_key(id)
                && current_comments
                    .get(id)
                    .is_some_and(|current_comment| current_comment == base_comment)
                && Self::remove_stored_comment(current, id)
            {
                changed += 1;
            }
        }

        changed
    }

    fn comment_count(session: &ReviewSession) -> usize {
        session.review_comments.len()
            + session
                .files
                .values()
                .map(|review| review.comment_count())
                .sum::<usize>()
    }

    fn collect_stored_comments(session: &ReviewSession) -> HashMap<String, StoredComment> {
        let mut comments = HashMap::new();
        for comment in &session.review_comments {
            comments.insert(
                comment.id.clone(),
                StoredComment {
                    location: StoredCommentLocation::Review,
                    comment: comment.clone(),
                },
            );
        }

        for (path, review) in &session.files {
            for comment in &review.file_comments {
                comments.insert(
                    comment.id.clone(),
                    StoredComment {
                        location: StoredCommentLocation::File { path: path.clone() },
                        comment: comment.clone(),
                    },
                );
            }

            for (line, line_comments) in &review.line_comments {
                for comment in line_comments {
                    comments.insert(
                        comment.id.clone(),
                        StoredComment {
                            location: StoredCommentLocation::Line {
                                path: path.clone(),
                                line: *line,
                            },
                            comment: comment.clone(),
                        },
                    );
                }
            }
        }

        comments
    }

    fn upsert_stored_comment(session: &mut ReviewSession, stored: StoredComment) {
        Self::remove_stored_comment(session, &stored.comment.id);
        match stored.location {
            StoredCommentLocation::Review => {
                session.review_comments.push(stored.comment);
            }
            StoredCommentLocation::File { path } => {
                if let Some(review) = session.files.get_mut(&path) {
                    review.file_comments.push(stored.comment);
                }
            }
            StoredCommentLocation::Line { path, line } => {
                if let Some(review) = session.files.get_mut(&path) {
                    review
                        .line_comments
                        .entry(line)
                        .or_default()
                        .push(stored.comment);
                }
            }
        }
    }

    fn remove_stored_comment(session: &mut ReviewSession, id: &str) -> bool {
        if let Some(index) = session
            .review_comments
            .iter()
            .position(|comment| comment.id == id)
        {
            session.review_comments.remove(index);
            return true;
        }

        for review in session.files.values_mut() {
            if let Some(index) = review
                .file_comments
                .iter()
                .position(|comment| comment.id == id)
            {
                review.file_comments.remove(index);
                return true;
            }

            let mut emptied_line = None;
            for (line, comments) in &mut review.line_comments {
                if let Some(index) = comments.iter().position(|comment| comment.id == id) {
                    comments.remove(index);
                    if comments.is_empty() {
                        emptied_line = Some(*line);
                    }
                    break;
                }
            }
            if let Some(line) = emptied_line {
                review.line_comments.remove(&line);
                return true;
            }
        }

        false
    }

    /// Detect a GitHub forge repository from the local checkout, if any.
    /// Lazily called during startup — running this synchronously is fine
    /// because it only reads local config, never the network.
    fn detect_forge_repository(&mut self) {
        // `--repo-url` short-circuits detection: the user has told us
        // exactly which repo to target, so skip both the local-remote
        // probe and the `gh api` parent lookup that runs on PR-tab entry.
        if let Some(override_repo) = self.repo_url_override.clone() {
            self.forge_repository = Some(override_repo.clone());
            self.pr_tab = PullRequestsTab::new(Some(override_repo));
            self.canonical_resolved = true;
            return;
        }
        let repo = crate::forge::detect_forge_repository(&self.vcs_info.root_path);
        self.forge_repository = repo.clone();
        self.pr_tab = PullRequestsTab::new(repo);
    }

    fn resolve_comment_types(
        theme: &Theme,
        comment_type_configs: Option<Vec<CommentTypeConfig>>,
    ) -> Vec<CommentTypeDefinition> {
        let defaults = vec![
            CommentTypeDefinition {
                id: "note".to_string(),
                label: "note".to_string(),
                definition: Some("observations".to_string()),
                color: Some(theme.comment_note),
            },
            CommentTypeDefinition {
                id: "suggestion".to_string(),
                label: "suggestion".to_string(),
                definition: Some("improvements".to_string()),
                color: Some(theme.comment_suggestion),
            },
            CommentTypeDefinition {
                id: "issue".to_string(),
                label: "issue".to_string(),
                definition: Some("problems to fix".to_string()),
                color: Some(theme.comment_issue),
            },
            CommentTypeDefinition {
                id: "praise".to_string(),
                label: "praise".to_string(),
                definition: Some("positive feedback".to_string()),
                color: Some(theme.comment_praise),
            },
        ];

        let Some(configs) = comment_type_configs else {
            return defaults;
        };

        let mut resolved = Vec::new();
        for config in configs {
            let id = config.id;
            let label = config.label.unwrap_or_else(|| id.clone());
            let definition = config.definition;
            let color = config.color.as_deref().and_then(Self::parse_config_color);
            resolved.push(CommentTypeDefinition {
                id,
                label,
                definition,
                color,
            });
        }

        if resolved.is_empty() {
            defaults
        } else {
            resolved
        }
    }

    fn first_comment_type(comment_types: &[CommentTypeDefinition]) -> CommentType {
        comment_types
            .first()
            .map(|comment_type| CommentType::from_id(&comment_type.id))
            .unwrap_or_default()
    }

    fn default_comment_type(&self) -> CommentType {
        Self::first_comment_type(&self.comment_types)
    }

    fn parse_config_color(value: &str) -> Option<Color> {
        let normalized = value.trim().to_ascii_lowercase();
        if normalized.is_empty() {
            return None;
        }

        if let Some(hex) = normalized.strip_prefix('#')
            && hex.len() == 6
            && let Ok(rgb) = u32::from_str_radix(hex, 16)
        {
            let r = ((rgb >> 16) & 0xff) as u8;
            let g = ((rgb >> 8) & 0xff) as u8;
            let b = (rgb & 0xff) as u8;
            return Some(Color::Rgb(r, g, b));
        }

        match normalized.as_str() {
            "black" => Some(Color::Black),
            "red" => Some(Color::Red),
            "green" => Some(Color::Green),
            "yellow" => Some(Color::Yellow),
            "blue" => Some(Color::Blue),
            "magenta" => Some(Color::Magenta),
            "cyan" => Some(Color::Cyan),
            "gray" | "grey" => Some(Color::Gray),
            "darkgray" | "dark_gray" | "darkgrey" | "dark_grey" => Some(Color::DarkGray),
            "lightred" | "light_red" => Some(Color::LightRed),
            "lightgreen" | "light_green" => Some(Color::LightGreen),
            "lightyellow" | "light_yellow" => Some(Color::LightYellow),
            "lightblue" | "light_blue" => Some(Color::LightBlue),
            "lightmagenta" | "light_magenta" => Some(Color::LightMagenta),
            "lightcyan" | "light_cyan" => Some(Color::LightCyan),
            "white" => Some(Color::White),
            _ => None,
        }
    }

    pub fn comment_type_label(&self, comment_type: &CommentType) -> String {
        if let Some(definition) = self
            .comment_types
            .iter()
            .find(|definition| definition.id == comment_type.id())
        {
            return definition.label.to_ascii_uppercase();
        }

        comment_type.as_str()
    }

    pub fn comment_type_color(&self, comment_type: &CommentType) -> Color {
        if let Some(definition) = self
            .comment_types
            .iter()
            .find(|definition| definition.id == comment_type.id())
            && let Some(color) = definition.color
        {
            return color;
        }

        match comment_type.id() {
            "note" => self.theme.comment_note,
            "suggestion" => self.theme.comment_suggestion,
            "issue" => self.theme.comment_issue,
            "praise" => self.theme.comment_praise,
            _ => self.theme.fg_secondary,
        }
    }

    /// Load or create a session for a commit range (used by revisions and commit selection).
    fn load_or_create_commit_range_session(
        vcs_info: &VcsInfo,
        commit_ids: &[String],
    ) -> ReviewSession {
        let newest_commit_id = commit_ids.last().unwrap().clone();
        let loaded = load_latest_session_for_context(
            &vcs_info.root_path,
            vcs_info.branch_name.as_deref(),
            &newest_commit_id,
            SessionDiffSource::CommitRange,
            Some(commit_ids),
        )
        .ok()
        .and_then(|found| found.map(|(_path, session)| session));

        let mut session = loaded.unwrap_or_else(|| {
            let mut s = ReviewSession::new(
                vcs_info.root_path.clone(),
                newest_commit_id,
                vcs_info.branch_name.clone(),
                SessionDiffSource::CommitRange,
            );
            s.commit_range = Some(commit_ids.to_vec());
            s
        });

        if session.commit_range.is_none() {
            session.commit_range = Some(commit_ids.to_vec());
            session.updated_at = chrono::Utc::now();
        }
        session
    }

    fn load_or_create_staged_unstaged_and_commits_session(
        vcs_info: &VcsInfo,
        commit_ids: &[String],
    ) -> ReviewSession {
        let newest_commit_id = commit_ids.last().unwrap().clone();
        let loaded = load_latest_session_for_context(
            &vcs_info.root_path,
            vcs_info.branch_name.as_deref(),
            &newest_commit_id,
            SessionDiffSource::StagedUnstagedAndCommits,
            Some(commit_ids),
        )
        .ok()
        .and_then(|found| found.map(|(_path, session)| session));

        let mut session = loaded.unwrap_or_else(|| {
            let mut s = ReviewSession::new(
                vcs_info.root_path.clone(),
                newest_commit_id,
                vcs_info.branch_name.clone(),
                SessionDiffSource::StagedUnstagedAndCommits,
            );
            s.commit_range = Some(commit_ids.to_vec());
            s
        });

        if session.commit_range.is_none() {
            session.commit_range = Some(commit_ids.to_vec());
            session.updated_at = chrono::Utc::now();
        }
        session
    }

    fn load_or_create_session(vcs_info: &VcsInfo, diff_source: SessionDiffSource) -> ReviewSession {
        let new_session = || {
            ReviewSession::new(
                vcs_info.root_path.clone(),
                vcs_info.head_commit.clone(),
                vcs_info.branch_name.clone(),
                diff_source,
            )
        };

        let Ok(found) = load_latest_session_for_context(
            &vcs_info.root_path,
            vcs_info.branch_name.as_deref(),
            &vcs_info.head_commit,
            diff_source,
            None,
        ) else {
            return new_session();
        };

        let Some((_path, mut session)) = found else {
            return new_session();
        };

        let mut updated = false;
        if session.branch_name.is_none() && vcs_info.branch_name.is_some() {
            session.branch_name = vcs_info.branch_name.clone();
            updated = true;
        }

        if vcs_info.branch_name.is_some() && session.base_commit != vcs_info.head_commit {
            session.base_commit = vcs_info.head_commit.clone();
            updated = true;
        }

        if updated {
            session.updated_at = chrono::Utc::now();
        }

        session
    }

    /// Materialize a PR session from an already-opened PR. Reattaches the
    /// most recent persisted session for the same head SHA when present so
    /// reviewed markers and local comments survive a reopen.
    fn load_or_apply_pr_session(opened: &mut crate::forge::pr_open::OpenedPullRequest) {
        let key = opened.key.clone();
        let Ok(Some((_path, mut persisted))) = crate::persistence::load_pr_session(&key) else {
            return;
        };

        // Re-register diff files against the loaded session so any new files
        // in the PR appear with content_hash tracking, and any deleted files
        // simply stop appearing in the file list.
        // Strict subset sessions are reloaded through a full PR diff first, so
        // pruning here would discard hunk keys hidden by the active selector.
        let preserve_hunks = Self::is_strict_commit_selection(
            persisted.commit_selection_range,
            opened.commits.len(),
        );
        Self::register_diff_files(&mut persisted, &opened.diff_files, preserve_hunks);
        persisted.pr_session_key = Some(key);
        persisted.diff_source = SessionDiffSource::PullRequest;
        persisted.updated_at = chrono::Utc::now();
        opened.session = persisted;
    }

    fn is_strict_commit_selection(range: Option<(usize, usize)>, total: usize) -> bool {
        range.is_some_and(|(start, end)| {
            total > 0 && start <= end && end < total && (start > 0 || end + 1 < total)
        })
    }

    fn register_diff_files(
        session: &mut ReviewSession,
        diff_files: &[DiffFile],
        preserve_hunks: bool,
    ) {
        for file in diff_files {
            if preserve_hunks {
                session.add_diff_file_preserving_hunks(file);
            } else {
                session.add_diff_file(file);
            }
        }
    }

    /// Direct-entry PR open: `tuicr pr <target>`.
    pub fn new_from_pr_target(
        theme: Theme,
        comment_type_configs: Option<Vec<CommentTypeConfig>>,
        output_to_stdout: bool,
        target: &str,
        repo_url_override: Option<ForgeRepository>,
    ) -> Result<Self> {
        use crate::forge::github::gh::parse_pull_request_target;
        use crate::forge::gitlab::glab::parse_pull_request_target_gitlab;
        use crate::forge::pr_open::open_pull_request;
        use crate::forge::traits::ForgeKind;

        // Try GitHub-style target first (numeric, GitHub URL, owner/repo#N).
        // If it embeds a GitLab URL, the GitLab parser picks it up.
        let parsed = parse_pull_request_target(target)
            .or_else(|_| parse_pull_request_target_gitlab(target))?;

        // Resolution order when the target lacks an explicit repo
        // (`tuicr pr 125`):
        //   1. `--repo-url` override (explicit user intent; no I/O)
        //   2. canonical of the local `origin` (gh api parent lookup —
        //      so `tuicr pr 125` from a fork checkout opens the PR on
        //      the upstream, matching the PR-tab behavior)
        //   3. detected local `origin` as the final fallback
        // URL- and owner-repo-hash targets carry their own repository, which
        // wins over all of the above since it's PR-specific.
        let local_repo_root = std::env::current_dir().ok();
        let detected_repo = local_repo_root
            .as_deref()
            .and_then(crate::forge::detect_forge_repository);

        // Canonical resolution (fork parent lookup) only works for GitHub.
        let canonical_repo = detected_repo.as_ref().and_then(|origin| {
            if origin.kind == ForgeKind::GitHub {
                use crate::forge::canonical::resolve_canonical_repository;
                use crate::forge::github::gh::SystemGhRunner;
                Some(resolve_canonical_repository(
                    origin,
                    repo_url_override.as_ref(),
                    &SystemGhRunner,
                ))
            } else {
                None
            }
        });
        let target_repo = parsed
            .repository
            .clone()
            .or_else(|| repo_url_override.clone())
            .or_else(|| canonical_repo.clone())
            .or_else(|| detected_repo.clone())
            .ok_or_else(|| {
                TuicrError::Forge(
                    "tuicr pr <number> requires a local forge remote. \
                     Use owner/repo#N or a full PR URL outside a checkout."
                        .to_string(),
                )
            })?;

        // Use the local checkout for `.tuicrignore` only when it matches the
        // PR's target repository — using a foreign repo's checkout would
        // mis-filter the PR diff.
        let local_checkout_for_target = local_repo_root.as_deref().and_then(|root| {
            let detected = crate::forge::detect_forge_repository(root)?;
            if detected == target_repo {
                Some(root.to_path_buf())
            } else {
                None
            }
        });

        let backend = create_forge_backend(&target_repo, local_checkout_for_target.clone());
        let highlighter = theme.syntax_highlighter();
        let mut opened = open_pull_request(
            backend.as_ref(),
            parsed,
            local_checkout_for_target.as_deref(),
            highlighter,
        )?;

        Self::load_or_apply_pr_session(&mut opened);

        let pr_source = PullRequestDiffSource::from_details(&opened.details);
        let diff_source = DiffSource::PullRequest(Box::new(pr_source));
        let vcs_info = VcsInfo {
            root_path: opened.session.repo_path.clone(),
            head_commit: opened.details.head_sha.clone(),
            branch_name: Some(opened.details.head_ref_name.clone()),
            vcs_type: VcsType::File,
        };
        // FileBackend acts as a no-op VCS placeholder; PR context expansion
        // routes through the forge backend, not the VCS box.
        let vcs: Box<dyn VcsBackend> = Box::new(PrNoopVcs::new(vcs_info.clone()));

        // Snapshot the PR details before consuming `opened` so we can kick
        // off the remote-thread fetch after `Self::build` returns.
        let details_for_threads = opened.details.clone();
        let mut app = Self::build(
            vcs,
            vcs_info,
            theme,
            comment_type_configs,
            output_to_stdout,
            opened.diff_files,
            opened.session,
            diff_source,
            InputMode::Normal,
            Vec::new(),
            None,
            repo_url_override,
        )?;

        // Wire the forge backend so context expansion routes through it.
        app.forge_backend = Some(backend);
        app.forge_repository = Some(target_repo);
        // PR open establishes the target repo directly; no further canonical
        // resolution needed on PR-tab entry (which won't happen anyway since
        // the user came straight from CLI into PR diff mode).
        app.canonical_resolved = true;
        app.current_pr_head = Some(details_for_threads.head_sha.clone());
        if let DiffSource::PullRequest(pr) = &app.diff_source.clone()
            && pr.is_read_only()
        {
            let reason = pr.read_only_reason().unwrap_or("read only");
            app.set_warning(format!("This PR is {reason} — review is read-only"));
        }
        // Spawn thread-fetch on startup; the main event loop will drain
        // the receiver via `poll_pr_threads_events` once it begins.
        app.spawn_pr_threads_fetch(&details_for_threads, local_checkout_for_target);
        Ok(app)
    }

    /// Re-enter PR mode after we've already opened a PR via the selector.
    /// Used by the selector → PR open path and by `:reload` in PR mode.
    pub fn enter_pr_diff_mode(
        &mut self,
        backend: Box<dyn ForgeBackend>,
        opened: crate::forge::pr_open::OpenedPullRequest,
    ) -> Result<()> {
        let crate::forge::pr_open::OpenedPullRequest {
            details,
            diff_files,
            session,
            key,
            commits,
        } = opened;

        // Save the current session before transitioning so local-mode work
        // isn't lost.
        let _ = self.save_current_session_merging_external();

        let pr_source = PullRequestDiffSource::from_details(&details);
        let read_only_reason = pr_source.read_only_reason();
        let virtual_root = session.repo_path.clone();

        self.vcs_info = VcsInfo {
            root_path: virtual_root.clone(),
            head_commit: details.head_sha.clone(),
            branch_name: Some(details.head_ref_name.clone()),
            vcs_type: VcsType::File,
        };
        self.vcs = Box::new(PrNoopVcs::new(self.vcs_info.clone()));
        self.session = session;
        self.diff_files = diff_files;
        self.reset_persisted_session_tracking();
        self.diff_source = DiffSource::PullRequest(Box::new(pr_source));
        self.forge_backend = Some(backend);
        self.forge_repository = Some(key.repository.clone());
        // Reset remote-comment state on every PR mode entry; the new PR's
        // threads will be fetched separately by spawn_pr_threads_fetch.
        self.forge_review_threads = Vec::new();
        self.forge_review_summaries = Vec::new();
        self.forge_review_threads_loading = false;
        self.pr_threads_rx = None;
        // Latest known remote head — equal to the session head at open time;
        // refreshed by future `gh pr view` calls in PR 6.
        self.current_pr_head = Some(details.head_sha.clone());
        self.input_mode = InputMode::Normal;
        self.focused_panel = FocusedPanel::Diff;
        self.clear_expanded_gaps();
        self.commit_list.clear();
        self.commit_selection_range = None;
        self.review_commits.clear();
        self.pr_commits.clear();
        self.show_commit_selector = false;
        self.range_diff_files = None;
        self.saved_inline_selection = None;
        self.diff_state = DiffState::default();

        // PR mode populates the inline selector with the PR's commits when
        // there are at least two. Single-commit PRs hide the selector to
        // match the local-mode UX. We mirror `commit_list` and
        // `review_commits` into shared App state so the existing
        // inline_commit_selector renderer Just Works.
        if commits.len() > 1 {
            self.pr_commits = commits.clone();
            let mapped: Vec<CommitInfo> = commits.iter().map(pr_commit_to_commit_info).collect();
            self.range_diff_files = Some(self.diff_files.clone());
            self.commit_list = mapped.clone();
            self.commit_list_cursor = 0;
            self.commit_list_scroll_offset = 0;
            self.visible_commit_count = mapped.len();
            self.has_more_commit = false;
            self.show_commit_selector = true;
            let mut range = (0, mapped.len() - 1);
            // Restore any persisted range scoped to this head SHA. If the
            // restored range exceeds the current commit count (e.g., the PR
            // was rebased), fall back to "all".
            if let Some(persisted) = self.session.commit_selection_range
                && persisted.1 < mapped.len()
                && persisted.0 <= persisted.1
            {
                range = persisted;
            }
            self.commit_selection_range = Some(range);
            self.review_commits = mapped;
        }

        // Ensure session has all files registered after the swap. A strict
        // selector range is a filtered view, not a new review scope.
        let preserve_hunks =
            Self::is_strict_commit_selection(self.commit_selection_range, self.pr_commits.len());
        Self::register_diff_files(&mut self.session, &self.diff_files, preserve_hunks);

        self.sort_files_by_directory(true);
        self.expand_all_dirs();
        self.rebuild_annotations();

        if let Some(reason) = read_only_reason {
            self.set_warning(format!("This PR is {reason} — review is read-only"));
        }

        // If the restored selection is a strict subset, fire an initial
        // range re-fetch so the diff matches the persisted scope.
        if matches!(&self.diff_source, DiffSource::PullRequest(_))
            && let Some(range) = self.commit_selection_range
            && !self.pr_commits.is_empty()
            && (range.0 > 0 || range.1 + 1 < self.pr_commits.len())
        {
            self.spawn_pr_range_reload();
        }

        Ok(())
    }

    /// Reload the PR's head from the forge. If the head SHA changed, this
    /// switches sessions so old-head draft comments stay with the old
    /// session and the new session starts clean.
    /// Capture the cursor's current file + line numbers so we can try to
    /// land back here after `:e` rebuilds the diff. Returns `None` when
    /// the cursor isn't on a diff line (e.g., it's on a header / comment
    /// / hunk header / expander).
    fn capture_pr_cursor_anchor(&self) -> Option<PrCursorAnchor> {
        let annotation = self.line_annotations.get(self.diff_state.cursor_line)?;
        let (file_idx, old_lineno, new_lineno) = match annotation {
            AnnotatedLine::DiffLine {
                file_idx,
                old_lineno,
                new_lineno,
                ..
            } => (*file_idx, *old_lineno, *new_lineno),
            AnnotatedLine::SideBySideLine {
                file_idx,
                old_lineno,
                new_lineno,
                ..
            } => (*file_idx, *old_lineno, *new_lineno),
            AnnotatedLine::ExpandedContext { gap_id, .. } => {
                // Approximate: drop back to the file index from the gap.
                let file_idx = gap_id.file_idx;
                (file_idx, None, None)
            }
            _ => {
                let file_idx = annotation_file_idx(annotation)?;
                (file_idx, None, None)
            }
        };
        let path = self.diff_files.get(file_idx)?.display_path().clone();
        Some(PrCursorAnchor {
            path,
            new_lineno,
            old_lineno,
        })
    }

    /// Move the cursor to a sensible spot after a reload that may have
    /// shifted file ordering / hunk boundaries. Best-effort: match the
    /// exact `(path, new_lineno)` if it still exists, else the same
    /// `(path, old_lineno)` on the LEFT side, else the file's first
    /// annotation, else stay at line 0.
    fn restore_pr_cursor_to_anchor(&mut self, anchor: &PrCursorAnchor) {
        let mut best: Option<usize> = None;
        let mut file_first: Option<usize> = None;
        for (idx, ann) in self.line_annotations.iter().enumerate() {
            let file_idx = match ann {
                AnnotatedLine::DiffLine { file_idx, .. }
                | AnnotatedLine::SideBySideLine { file_idx, .. }
                | AnnotatedLine::HunkHeader { file_idx, .. }
                | AnnotatedLine::FileHeader { file_idx, .. } => *file_idx,
                _ => continue,
            };
            let Some(file) = self.diff_files.get(file_idx) else {
                continue;
            };
            if file.display_path() != &anchor.path {
                continue;
            }
            file_first.get_or_insert(idx);
            let (line_new, line_old) = match ann {
                AnnotatedLine::DiffLine {
                    old_lineno,
                    new_lineno,
                    ..
                }
                | AnnotatedLine::SideBySideLine {
                    old_lineno,
                    new_lineno,
                    ..
                } => (*new_lineno, *old_lineno),
                _ => (None, None),
            };
            if anchor.new_lineno.is_some() && line_new == anchor.new_lineno {
                best = Some(idx);
                break;
            }
            if anchor.old_lineno.is_some() && line_old == anchor.old_lineno {
                best = Some(idx);
                // Don't break — a later RIGHT-side match may still be better.
            }
        }
        let target = best.or(file_first).unwrap_or(0);
        // `move_cursor_to_annotation` updates cursor_line AND adjusts
        // `scroll_offset` so the cursor stays in the viewport. Without
        // it the viewport snaps back to the top of the diff after the
        // reload.
        self.move_cursor_to_annotation(target);
    }

    /// Persist the active inline selection on the session (PR mode only).
    /// `None` is written when the range covers all commits so re-open
    /// doesn't trigger an unnecessary subset re-fetch.
    pub fn persist_pr_commit_selection_range(&mut self) {
        if !matches!(self.diff_source, DiffSource::PullRequest(_)) {
            return;
        }
        let total = self.pr_commits.len();
        let value = match self.commit_selection_range {
            Some((s, e)) if total > 0 && (s > 0 || e + 1 < total) => Some((s, e)),
            _ => None,
        };
        self.session.commit_selection_range = value;
        self.session.updated_at = chrono::Utc::now();
        let _ = self.save_current_session_merging_external();
    }

    /// Resolve the active inline selection (PR mode) to (start_sha,
    /// end_sha). `start_sha` is the parent of the *oldest* selected
    /// commit; `end_sha` is the *newest*. Because `pr_commits` is stored
    /// newest-first, the oldest selected commit is at `range.1` and the
    /// newest at `range.0`.
    ///
    /// Returns `None` outside PR mode, when the selection is empty, or
    /// when the resolved parent isn't available — in that case the
    /// caller falls back to the cached cumulative PR diff.
    pub fn pr_range_sha_pair(&self) -> Option<(String, String)> {
        let DiffSource::PullRequest(ref pr) = self.diff_source else {
            return None;
        };
        let (start_idx, end_idx) = self.commit_selection_range?;
        if self.pr_commits.is_empty() || start_idx > end_idx || end_idx >= self.pr_commits.len() {
            return None;
        }
        // Newest-first: `end_idx` is the oldest, `start_idx` is the newest.
        let newest = self.pr_commits.get(start_idx)?;
        // Parent of the oldest selected commit. If the oldest selected commit
        // is the PR's first commit (oldest commit overall, at the bottom of
        // the list), its parent is the PR's base SHA.
        let parent_sha = if end_idx + 1 < self.pr_commits.len() {
            self.pr_commits[end_idx + 1].oid.clone()
        } else {
            pr.base_sha.clone()
        };
        Some((parent_sha, newest.oid.clone()))
    }

    /// Reload the PR diff for the currently selected inline commit
    /// subrange. Uses the cached cumulative diff when the selection
    /// covers all commits; spawns a background `compare` fetch otherwise.
    pub fn reload_pr_inline_selection(&mut self) {
        // No-op outside PR mode.
        if !matches!(self.diff_source, DiffSource::PullRequest(_)) {
            return;
        }
        let Some(range) = self.commit_selection_range else {
            return;
        };
        let total = self.pr_commits.len();
        if total == 0 {
            return;
        }

        // Full-range selection: restore the cached cumulative diff
        // without hitting the network.
        if range.0 == 0 && range.1 + 1 == total {
            self.apply_cached_full_pr_diff();
            return;
        }

        // Strict subset → range re-fetch on a background thread.
        self.spawn_pr_range_reload();
    }

    /// Restore the cached cumulative PR diff into the diff view. Used when
    /// the user toggles the selector back to "all commits".
    fn apply_cached_full_pr_diff(&mut self) {
        let Some(files) = self.range_diff_files.clone() else {
            return;
        };
        let anchor = self.capture_pr_cursor_anchor();
        self.diff_files = files;
        self.clear_expanded_gaps();
        for file in &self.diff_files {
            self.session.add_diff_file(file);
        }
        self.sort_files_by_directory(true);
        self.expand_all_dirs();
        self.rebuild_annotations();
        if let Some(anchor) = anchor {
            self.restore_pr_cursor_to_anchor(&anchor);
        }
    }

    /// Kick off a background fetch of `compare/<start>...<end>` and apply
    /// it on the main thread. Cancels any in-flight range reload (a fresh
    /// toggle invalidates the previous request).
    pub fn spawn_pr_range_reload(&mut self) {
        let DiffSource::PullRequest(current) = self.diff_source.clone() else {
            return;
        };
        let Some((start_sha, end_sha)) = self.pr_range_sha_pair() else {
            return;
        };
        let Some(range) = self.commit_selection_range else {
            return;
        };

        let anchor = self.capture_pr_cursor_anchor();
        let request = PrRangeReloadRequest {
            repository: current.key.repository.clone(),
            pr_number: current.key.number,
            head_sha: current.key.head_sha.clone(),
            start_sha: start_sha.clone(),
            end_sha: end_sha.clone(),
            range,
            started_at: Instant::now(),
            anchor,
        };
        // A fresh toggle supersedes any in-flight fetch.
        self.pr_range_reload_state = Some(request.clone());

        let local_checkout = self
            .forge_backend
            .as_deref()
            .and_then(|backend| backend.local_checkout_path());

        let (tx, rx) = std::sync::mpsc::channel();
        self.pr_range_reload_rx = Some(rx);

        let repository = current.key.repository.clone();
        let pr_number = current.key.number;
        let head_sha = current.key.head_sha.clone();
        let base_sha = current.base_sha.clone();
        std::thread::spawn(move || {
            let backend = create_forge_backend(&repository, local_checkout);
            let details = crate::forge::traits::PullRequestDetails {
                repository: repository.clone(),
                number: pr_number,
                title: String::new(),
                url: String::new(),
                state: "OPEN".to_string(),
                is_draft: false,
                author: None,
                head_ref_name: String::new(),
                base_ref_name: String::new(),
                head_sha,
                base_sha,
                body: String::new(),
                updated_at: None,
                closed: false,
                merged_at: None,
                diff_start_sha: None,
            };
            let outcome = backend
                .get_pull_request_commit_range_diff(&details, &start_sha, &end_sha)
                .map_err(|e| e.to_string());
            let _ = tx.send(PrRangeReloadEvent::Done {
                request,
                result: outcome,
            });
        });
    }

    /// Pump any pending range-reload result, parse on the main thread, and
    /// apply. Stale results (the user toggled again, or left PR mode) are
    /// silently dropped.
    pub fn poll_pr_range_reload_events(&mut self) {
        let Some(rx) = self.pr_range_reload_rx.as_ref() else {
            return;
        };
        let event = match rx.try_recv() {
            Ok(e) => e,
            Err(_) => return,
        };
        self.pr_range_reload_rx = None;

        let PrRangeReloadEvent::Done { request, result } = event;
        let in_flight = self.pr_range_reload_state.clone();
        // Only apply if this result still matches the active in-flight
        // request — toggling again, switching PRs, or reloading the head
        // before this lands invalidates it.
        let still_active = in_flight.as_ref().is_some_and(|s| {
            s.start_sha == request.start_sha
                && s.end_sha == request.end_sha
                && s.repository == request.repository
                && s.pr_number == request.pr_number
                && s.head_sha == request.head_sha
                && s.range == request.range
        });
        if !still_active {
            return;
        }
        self.pr_range_reload_state = None;

        match result {
            Ok(patch) => {
                if let Err(e) = self.finish_pr_range_reload(&request, &patch) {
                    self.set_error(format!("Range diff failed: {e}"));
                }
            }
            Err(e) => {
                self.set_error(format!("Range diff failed: {e}"));
            }
        }
    }

    fn finish_pr_range_reload(
        &mut self,
        request: &PrRangeReloadRequest,
        patch: &str,
    ) -> Result<()> {
        use crate::vcs::diff_parser::{DiffFormat, parse_unified_diff};

        let highlighter = self.theme.syntax_highlighter();
        let parsed = match parse_unified_diff(patch, DiffFormat::GitStyle, highlighter) {
            Ok(files) => files,
            Err(TuicrError::NoChanges) => Vec::new(),
            Err(e) => return Err(e),
        };

        let local_checkout = self
            .forge_backend
            .as_deref()
            .and_then(|b| b.local_checkout_path());
        let files = match local_checkout.as_deref() {
            Some(root) => crate::tuicrignore::filter_diff_files(root, parsed),
            None => parsed,
        };

        self.diff_files = files;
        self.clear_expanded_gaps();
        // Range diffs can hide hunks that are still reviewed in the broader
        // PR session, so registration must not prune them.
        Self::register_diff_files(&mut self.session, &self.diff_files, true);
        self.sort_files_by_directory(true);
        self.expand_all_dirs();
        self.rebuild_annotations();

        if let Some(anchor) = &request.anchor {
            self.restore_pr_cursor_to_anchor(anchor);
        }
        Ok(())
    }

    /// Kick off `:e` asynchronously. Captures the cursor anchor, sets
    /// the reload state for the spinner, and spawns the network fetch
    /// on a background thread. Returns immediately. The result is
    /// applied later in `poll_pr_reload_events`.
    pub fn spawn_pr_reload(&mut self) -> Result<()> {
        use crate::forge::pr_open::fetch_pr_data;
        use crate::forge::traits::PullRequestTarget;

        let DiffSource::PullRequest(current) = self.diff_source.clone() else {
            return Err(TuicrError::UnsupportedOperation(
                "Not in PR mode".to_string(),
            ));
        };
        if self.pr_reload_state.is_some() {
            return Ok(()); // already in flight; the existing spinner is enough
        }

        let anchor = self.capture_pr_cursor_anchor();
        let request = PrReloadRequest {
            repository: current.key.repository.clone(),
            pr_number: current.key.number,
            head_sha: current.key.head_sha.clone(),
            started_at: Instant::now(),
            anchor,
        };
        self.pr_reload_state = Some(request.clone());

        let local_checkout = self
            .forge_backend
            .as_deref()
            .and_then(|backend| backend.local_checkout_path());

        let (tx, rx) = std::sync::mpsc::channel();
        self.pr_reload_rx = Some(rx);

        let repository = current.key.repository.clone();
        let pr_number = current.key.number;
        std::thread::spawn(move || {
            let backend = create_forge_backend(&repository, local_checkout);
            let target =
                PullRequestTarget::with_repository(repository, pr_number, pr_number.to_string());
            let outcome = fetch_pr_data(backend.as_ref(), target).map_err(|e| e.to_string());
            let _ = tx.send(PrReloadEvent::Done {
                request,
                result: outcome,
            });
        });
        Ok(())
    }

    /// Pump a pending reload result. Parses + applies on the main thread,
    /// then restores the cursor to the remembered anchor.
    pub fn poll_pr_reload_events(&mut self) {
        let Some(rx) = self.pr_reload_rx.as_ref() else {
            return;
        };
        let event = match rx.try_recv() {
            Ok(e) => e,
            Err(_) => return,
        };
        self.pr_reload_rx = None;
        let in_flight = self.pr_reload_state.clone();
        self.pr_reload_state = None;
        let PrReloadEvent::Done { request, result } = event;
        if !in_flight
            .as_ref()
            .is_some_and(|s| s.pr_number == request.pr_number && s.repository == request.repository)
        {
            return;
        }
        match result {
            Ok((details, patch, commits)) => {
                if let Err(e) = self.finish_pr_reload(details, patch, commits, &request) {
                    self.set_error(format!("Reload failed: {e}"));
                }
            }
            Err(e) => {
                self.set_error(format!("Reload failed: {e}"));
            }
        }
    }

    fn finish_pr_reload(
        &mut self,
        details: crate::forge::traits::PullRequestDetails,
        patch: String,
        commits: Vec<crate::forge::traits::PullRequestCommit>,
        request: &PrReloadRequest,
    ) -> Result<()> {
        use crate::forge::pr_open::prepare_open_pr;

        let local_checkout = self
            .forge_backend
            .as_deref()
            .and_then(|backend| backend.local_checkout_path());
        let highlighter = self.theme.syntax_highlighter();
        let mut opened = prepare_open_pr(
            details,
            &patch,
            commits,
            local_checkout.as_deref(),
            highlighter,
        )?;

        let head_changed = opened.details.head_sha != request.head_sha;
        if head_changed {
            let _ = self.save_current_session_merging_external();
            let details_for_threads = opened.details.clone();
            Self::load_or_apply_pr_session(&mut opened);
            let backend = create_forge_backend(&request.repository, local_checkout.clone());
            self.enter_pr_diff_mode(backend, opened)?;
            self.spawn_pr_threads_fetch(&details_for_threads, local_checkout);
            self.set_message("Reloaded PR at new head — switched to fresh session".to_string());
        } else {
            self.diff_files = opened.diff_files;
            self.clear_expanded_gaps();
            for file in &self.diff_files {
                self.session.add_diff_file(file);
            }
            self.sort_files_by_directory(true);
            self.expand_all_dirs();
            self.rebuild_annotations();
            self.refetch_pr_threads();
            self.set_message("Reloaded PR (no new commits)".to_string());
        }

        if let Some(anchor) = &request.anchor {
            self.restore_pr_cursor_to_anchor(anchor);
        }
        Ok(())
    }

    /// Synchronous reload. Production code uses `spawn_pr_reload` for the
    /// async path; kept as a seam for tests that need to drive a reload
    /// in one call without an mpsc round-trip.
    #[allow(dead_code)]
    pub fn reload_pull_request(&mut self) -> Result<bool> {
        let DiffSource::PullRequest(current) = self.diff_source.clone() else {
            return Err(TuicrError::UnsupportedOperation(
                "Not in PR mode".to_string(),
            ));
        };

        let local_checkout = self
            .forge_backend
            .as_deref()
            .and_then(|backend| backend.local_checkout_path());
        let backend = create_forge_backend(&current.key.repository, local_checkout.clone());
        self.reload_pull_request_with_backend(backend, local_checkout)
    }

    /// Inner reload path. Takes the forge backend as a parameter so tests
    /// can inject a fake without going through `gh`.
    #[allow(dead_code)]
    pub fn reload_pull_request_with_backend(
        &mut self,
        backend: Box<dyn ForgeBackend>,
        local_checkout: Option<std::path::PathBuf>,
    ) -> Result<bool> {
        use crate::forge::pr_open::open_pull_request;

        let DiffSource::PullRequest(current) = self.diff_source.clone() else {
            return Err(TuicrError::UnsupportedOperation(
                "Not in PR mode".to_string(),
            ));
        };

        let target = crate::forge::traits::PullRequestTarget::with_repository(
            current.key.repository.clone(),
            current.key.number,
            current.key.number.to_string(),
        );
        let highlighter = self.theme.syntax_highlighter();
        let mut opened = open_pull_request(
            backend.as_ref(),
            target,
            local_checkout.as_deref(),
            highlighter,
        )?;

        let head_changed = opened.details.head_sha != current.key.head_sha;
        if head_changed {
            // Save the old-head session before switching so drafts persist.
            let _ = self.save_current_session_merging_external();
            let details_for_threads = opened.details.clone();
            Self::load_or_apply_pr_session(&mut opened);
            self.enter_pr_diff_mode(backend, opened)?;
            // Fetch threads against the new head; old-head threads stay
            // tied to the old session and are dropped here.
            self.spawn_pr_threads_fetch(&details_for_threads, local_checkout.clone());
        } else {
            // Same head: re-parse the diff to pick up any side-channel
            // changes (rare), but keep the session intact.
            self.diff_files = opened.diff_files;
            self.clear_expanded_gaps();
            for file in &self.diff_files {
                self.session.add_diff_file(file);
            }
            self.sort_files_by_directory(true);
            self.expand_all_dirs();
            self.rebuild_annotations();
        }

        Ok(head_changed)
    }

    fn staged_commit_entry() -> CommitInfo {
        CommitInfo {
            id: STAGED_SELECTION_ID.to_string(),
            short_id: "STAGED".to_string(),
            branch_name: None,
            summary: "Staged changes".to_string(),
            body: None,
            author: String::new(),
            time: Utc::now(),
        }
    }

    fn unstaged_commit_entry() -> CommitInfo {
        CommitInfo {
            id: UNSTAGED_SELECTION_ID.to_string(),
            short_id: "UNSTAGED".to_string(),
            branch_name: None,
            summary: "Unstaged changes".to_string(),
            body: None,
            author: String::new(),
            time: Utc::now(),
        }
    }

    /// If we are viewing a single commit, insert a "Commit Message" DiffFile at index 0.
    ///
    /// The synthetic path embeds the commit's short id (`Commit Message (<sha>)`)
    /// so that comments on different commits' messages get distinct session keys
    /// (the session indexes comments by path) and the exported review records
    /// which commit each commit-message comment belongs to.
    fn insert_commit_message_if_single(&mut self) {
        self.diff_files.retain(|f| !f.is_commit_message);

        let commit = if let Some((start, end)) = self.commit_selection_range {
            if start == end {
                self.review_commits.get(start)
            } else {
                None
            }
        } else if self.review_commits.len() == 1 {
            self.review_commits.first()
        } else {
            None
        };

        let Some(commit) = commit else { return };
        if Self::is_special_commit(commit) {
            return;
        }

        let mut full_message = commit.summary.clone();
        if let Some(ref body) = commit.body {
            full_message.push('\n');
            full_message.push('\n');
            full_message.push_str(body);
        }

        let diff_lines: Vec<DiffLine> = full_message
            .lines()
            .enumerate()
            .map(|(i, line)| DiffLine {
                origin: LineOrigin::Context,
                content: line.to_string(),
                old_lineno: None,
                new_lineno: Some(i as u32 + 1),
                highlighted_spans: None,
            })
            .collect();
        let line_count = diff_lines.len() as u32;
        let hunks = vec![DiffHunk {
            header: String::new(),
            lines: diff_lines,
            old_start: 0,
            old_count: 0,
            new_start: 1,
            new_count: line_count,
        }];
        let content_hash = DiffFile::compute_content_hash(&hunks);
        let commit_msg_file = DiffFile {
            old_path: None,
            new_path: Some(PathBuf::from(format!(
                "Commit Message ({})",
                commit.short_id
            ))),
            status: FileStatus::Added,
            hunks,
            is_binary: false,
            is_too_large: false,
            is_commit_message: true,
            content_hash,
        };
        self.diff_files.insert(0, commit_msg_file);
        self.session.add_diff_file(&self.diff_files[0]);
    }

    fn is_staged_commit(commit: &CommitInfo) -> bool {
        commit.id == STAGED_SELECTION_ID
    }

    fn is_unstaged_commit(commit: &CommitInfo) -> bool {
        commit.id == UNSTAGED_SELECTION_ID
    }

    fn is_special_commit(commit: &CommitInfo) -> bool {
        Self::is_staged_commit(commit) || Self::is_unstaged_commit(commit)
    }

    fn special_commit_count(&self) -> usize {
        self.commit_list
            .iter()
            .take_while(|commit| Self::is_special_commit(commit))
            .count()
    }

    fn loaded_history_commit_count(&self) -> usize {
        self.commit_list
            .len()
            .saturating_sub(self.special_commit_count())
    }

    fn filter_ignored_diff_files(repo_root: &Path, diff_files: Vec<DiffFile>) -> Vec<DiffFile> {
        crate::tuicrignore::filter_diff_files(repo_root, diff_files)
    }

    fn filter_by_path(diff_files: Vec<DiffFile>, path: &str) -> Vec<DiffFile> {
        let path = path.trim_end_matches('/');
        diff_files
            .into_iter()
            .filter(|f| {
                let display = f.display_path().to_string_lossy();
                display == path || display.starts_with(&format!("{path}/"))
            })
            .collect()
    }

    fn require_non_empty_diff_files(diff_files: Vec<DiffFile>) -> Result<Vec<DiffFile>> {
        if diff_files.is_empty() {
            return Err(TuicrError::NoChanges);
        }
        Ok(diff_files)
    }

    fn get_working_tree_diff_with_ignore(
        vcs: &dyn VcsBackend,
        repo_root: &Path,
        highlighter: &SyntaxHighlighter,
        path_filter: Option<&str>,
    ) -> Result<Vec<DiffFile>> {
        let diff_files = crate::profile::time_with(
            "diff.load_working_tree",
            || vcs.get_working_tree_diff(highlighter),
            profile_diff_result,
        )?;
        let diff_files = Self::filter_ignored_diff_files(repo_root, diff_files);
        let diff_files = if let Some(path) = path_filter {
            Self::filter_by_path(diff_files, path)
        } else {
            diff_files
        };
        Self::require_non_empty_diff_files(diff_files)
    }

    fn get_staged_diff_with_ignore(
        vcs: &dyn VcsBackend,
        repo_root: &Path,
        highlighter: &SyntaxHighlighter,
        path_filter: Option<&str>,
    ) -> Result<Vec<DiffFile>> {
        let diff_files = crate::profile::time_with(
            "diff.load_staged",
            || vcs.get_staged_diff(highlighter),
            profile_diff_result,
        )?;
        let diff_files = Self::filter_ignored_diff_files(repo_root, diff_files);
        let diff_files = if let Some(path) = path_filter {
            Self::filter_by_path(diff_files, path)
        } else {
            diff_files
        };
        Self::require_non_empty_diff_files(diff_files)
    }

    fn get_unstaged_diff_with_ignore(
        vcs: &dyn VcsBackend,
        repo_root: &Path,
        highlighter: &SyntaxHighlighter,
        path_filter: Option<&str>,
    ) -> Result<Vec<DiffFile>> {
        let diff_files = match crate::profile::time_with(
            "diff.load_unstaged",
            || vcs.get_unstaged_diff(highlighter),
            profile_diff_result,
        ) {
            Ok(diff_files) => diff_files,
            Err(TuicrError::UnsupportedOperation(_)) => crate::profile::time_with(
                "diff.load_unstaged_fallback_working_tree",
                || vcs.get_working_tree_diff(highlighter),
                profile_diff_result,
            )?,
            Err(e) => return Err(e),
        };
        let diff_files = Self::filter_ignored_diff_files(repo_root, diff_files);
        let diff_files = if let Some(path) = path_filter {
            Self::filter_by_path(diff_files, path)
        } else {
            diff_files
        };
        Self::require_non_empty_diff_files(diff_files)
    }

    fn get_commit_range_diff_with_ignore(
        vcs: &dyn VcsBackend,
        repo_root: &Path,
        revision_range: &ResolvedRevisionRange<'_>,
        highlighter: &SyntaxHighlighter,
        path_filter: Option<&str>,
    ) -> Result<Vec<DiffFile>> {
        let diff_files = crate::profile::time_with(
            "diff.load_commit_range",
            || vcs.get_commit_range_diff(revision_range, highlighter),
            profile_diff_result,
        )?;
        let diff_files = Self::filter_ignored_diff_files(repo_root, diff_files);
        let diff_files = if let Some(path) = path_filter {
            Self::filter_by_path(diff_files, path)
        } else {
            diff_files
        };
        Self::require_non_empty_diff_files(diff_files)
    }

    fn get_working_tree_with_commits_diff_with_ignore(
        vcs: &dyn VcsBackend,
        repo_root: &Path,
        commit_ids: &[String],
        highlighter: &SyntaxHighlighter,
        path_filter: Option<&str>,
    ) -> Result<Vec<DiffFile>> {
        let diff_files = crate::profile::time_with(
            "diff.load_working_tree_with_commits",
            || vcs.get_working_tree_with_commits_diff(commit_ids, highlighter),
            profile_diff_result,
        )?;
        let diff_files = Self::filter_ignored_diff_files(repo_root, diff_files);
        let diff_files = if let Some(path) = path_filter {
            Self::filter_by_path(diff_files, path)
        } else {
            diff_files
        };
        Self::require_non_empty_diff_files(diff_files)
    }

    /// Resolve the staged/unstaged status the commit selector renders.
    ///
    /// When `.gitignore`/`.tuicrignore` rules are present the cheap probe alone
    /// can't be trusted — a file the probe sees may be ignored. To verify
    /// without paying the full-diff cost, we ask the backend for just the
    /// changed paths and filter them through the same ignore rules. Backends
    /// that don't expose a path probe fall back to parsing the full diff.
    fn get_change_status_with_ignore(
        vcs: &dyn VcsBackend,
        repo_root: &Path,
        highlighter: &SyntaxHighlighter,
        path_filter: Option<&str>,
    ) -> Result<VcsChangeStatus> {
        if path_filter.is_none() {
            match vcs.get_change_status() {
                Ok(status) => {
                    if !crate::tuicrignore::has_ignore_rules(repo_root) {
                        return Ok(status);
                    }
                    return Self::verify_status_against_ignore(
                        vcs,
                        repo_root,
                        highlighter,
                        path_filter,
                        status,
                    );
                }
                Err(TuicrError::UnsupportedOperation(_)) => {}
                Err(e) => return Err(e),
            }
        }

        Self::verify_status_against_ignore(
            vcs,
            repo_root,
            highlighter,
            path_filter,
            VcsChangeStatus {
                staged: true,
                unstaged: true,
            },
        )
    }

    /// Refine `assumed_status` by checking each side actually has at least one
    /// non-ignored, non-filtered path. Tries the cheap path probe first; falls
    /// back to parsing the full diff for backends that don't implement it.
    fn verify_status_against_ignore(
        vcs: &dyn VcsBackend,
        repo_root: &Path,
        highlighter: &SyntaxHighlighter,
        path_filter: Option<&str>,
        assumed_status: VcsChangeStatus,
    ) -> Result<VcsChangeStatus> {
        let staged = if assumed_status.staged {
            Self::side_has_visible_changes(
                vcs,
                repo_root,
                highlighter,
                path_filter,
                ChangeKind::Staged,
            )?
        } else {
            false
        };
        let unstaged = if assumed_status.unstaged {
            Self::side_has_visible_changes(
                vcs,
                repo_root,
                highlighter,
                path_filter,
                ChangeKind::Unstaged,
            )?
        } else {
            false
        };
        Ok(VcsChangeStatus { staged, unstaged })
    }

    fn side_has_visible_changes(
        vcs: &dyn VcsBackend,
        repo_root: &Path,
        highlighter: &SyntaxHighlighter,
        path_filter: Option<&str>,
        kind: ChangeKind,
    ) -> Result<bool> {
        match vcs.list_changed_paths(kind) {
            Ok(paths) => Ok(Self::any_path_survives_filters(
                paths,
                repo_root,
                path_filter,
            )),
            Err(TuicrError::UnsupportedOperation(_)) => {
                // Backend can't list paths cheaply — parse the diff to see if
                // anything survives. This still happens for jj/hg today.
                let diff_result = match kind {
                    ChangeKind::Staged => {
                        Self::get_staged_diff_with_ignore(vcs, repo_root, highlighter, path_filter)
                    }
                    ChangeKind::Unstaged => Self::get_unstaged_diff_with_ignore(
                        vcs,
                        repo_root,
                        highlighter,
                        path_filter,
                    ),
                };
                match diff_result {
                    Ok(_) => Ok(true),
                    Err(TuicrError::NoChanges) | Err(TuicrError::UnsupportedOperation(_)) => {
                        Ok(false)
                    }
                    Err(e) => Err(e),
                }
            }
            Err(e) => Err(e),
        }
    }

    fn any_path_survives_filters(
        paths: Vec<PathBuf>,
        repo_root: &Path,
        path_filter: Option<&str>,
    ) -> bool {
        let after_ignore = crate::tuicrignore::filter_paths(repo_root, paths);
        let after_path = match path_filter {
            Some(p) => Self::filter_paths_by_pathspec(after_ignore, p),
            None => after_ignore,
        };
        !after_path.is_empty()
    }

    fn filter_paths_by_pathspec(paths: Vec<PathBuf>, pathspec: &str) -> Vec<PathBuf> {
        let pathspec = pathspec.trim_end_matches('/');
        paths
            .into_iter()
            .filter(|p| {
                let display = p.to_string_lossy();
                display == pathspec || display.starts_with(&format!("{pathspec}/"))
            })
            .collect()
    }

    fn load_staged_and_unstaged_selection(&mut self) -> Result<()> {
        let highlighter = self.theme.syntax_highlighter();
        let diff_files = match Self::get_working_tree_diff_with_ignore(
            self.vcs.as_ref(),
            &self.vcs_info.root_path,
            highlighter,
            self.path_filter.as_deref(),
        ) {
            Ok(diff_files) => diff_files,
            Err(TuicrError::NoChanges) => {
                self.set_message("No staged or unstaged changes");
                return Ok(());
            }
            Err(e) => return Err(e),
        };

        self.session =
            Self::load_or_create_session(&self.vcs_info, SessionDiffSource::StagedAndUnstaged);
        for file in &diff_files {
            self.session.add_diff_file(file);
        }
        self.reset_persisted_session_tracking();

        self.diff_files = diff_files;
        self.diff_source = DiffSource::StagedAndUnstaged;
        self.input_mode = InputMode::Normal;
        self.diff_state = DiffState::default();
        self.file_list_state = FileListState::default();
        self.clear_expanded_gaps();
        self.sort_files_by_directory(true);
        self.expand_all_dirs();
        self.rebuild_annotations();

        Ok(())
    }

    fn load_staged_selection(&mut self) -> Result<()> {
        let highlighter = self.theme.syntax_highlighter();
        let diff_files = match Self::get_staged_diff_with_ignore(
            self.vcs.as_ref(),
            &self.vcs_info.root_path,
            highlighter,
            self.path_filter.as_deref(),
        ) {
            Ok(diff_files) => diff_files,
            Err(TuicrError::NoChanges) => {
                self.set_message("No staged changes");
                return Ok(());
            }
            Err(e) => return Err(e),
        };

        self.session = Self::load_or_create_session(&self.vcs_info, SessionDiffSource::Staged);
        for file in &diff_files {
            self.session.add_diff_file(file);
        }
        self.reset_persisted_session_tracking();

        self.diff_files = diff_files;
        self.diff_source = DiffSource::Staged;
        self.input_mode = InputMode::Normal;
        self.diff_state = DiffState::default();
        self.file_list_state = FileListState::default();
        self.clear_expanded_gaps();
        self.sort_files_by_directory(true);
        self.expand_all_dirs();
        self.rebuild_annotations();

        Ok(())
    }

    fn load_unstaged_selection(&mut self) -> Result<()> {
        let highlighter = self.theme.syntax_highlighter();
        let diff_files = match Self::get_unstaged_diff_with_ignore(
            self.vcs.as_ref(),
            &self.vcs_info.root_path,
            highlighter,
            self.path_filter.as_deref(),
        ) {
            Ok(diff_files) => diff_files,
            Err(TuicrError::NoChanges) => {
                self.set_message("No unstaged changes");
                return Ok(());
            }
            Err(e) => return Err(e),
        };

        self.session = Self::load_or_create_session(&self.vcs_info, SessionDiffSource::Unstaged);
        for file in &diff_files {
            self.session.add_diff_file(file);
        }
        self.reset_persisted_session_tracking();

        self.diff_files = diff_files;
        self.diff_source = DiffSource::Unstaged;
        self.input_mode = InputMode::Normal;
        self.diff_state = DiffState::default();
        self.file_list_state = FileListState::default();
        self.clear_expanded_gaps();
        self.sort_files_by_directory(true);
        self.expand_all_dirs();
        self.rebuild_annotations();

        Ok(())
    }

    /// Reloads diff files from disk. Returns `(file_count, invalidated_count)` where
    /// `invalidated_count` is the number of previously reviewed files whose content changed.
    pub fn reload_diff_files(&mut self) -> Result<(usize, usize)> {
        let current_path = self.current_file_path().cloned();
        let prev_file_idx = self.diff_state.current_file_idx;
        let prev_cursor_line = self.diff_state.cursor_line;
        let prev_viewport_offset = self
            .diff_state
            .cursor_line
            .saturating_sub(self.diff_state.scroll_offset);
        let prev_relative_line = if self.diff_files.is_empty() {
            0
        } else {
            let start = self.calculate_file_scroll_offset(self.diff_state.current_file_idx);
            prev_cursor_line.saturating_sub(start)
        };

        let highlighter = self.theme.syntax_highlighter();
        let diff_files = match &self.diff_source {
            DiffSource::CommitRange(commit_ids) => Self::get_commit_range_diff_with_ignore(
                self.vcs.as_ref(),
                &self.vcs_info.root_path,
                &ResolvedRevisionRange::from_commit_ids(commit_ids, RevisionDiffTarget::CommitList),
                highlighter,
                self.path_filter.as_deref(),
            )?,
            DiffSource::StagedUnstagedAndCommits(commit_ids) => {
                let ids = commit_ids.clone();
                Self::get_working_tree_with_commits_diff_with_ignore(
                    self.vcs.as_ref(),
                    &self.vcs_info.root_path,
                    &ids,
                    highlighter,
                    self.path_filter.as_deref(),
                )?
            }
            DiffSource::Staged => Self::get_staged_diff_with_ignore(
                self.vcs.as_ref(),
                &self.vcs_info.root_path,
                highlighter,
                self.path_filter.as_deref(),
            )?,
            DiffSource::Unstaged => Self::get_unstaged_diff_with_ignore(
                self.vcs.as_ref(),
                &self.vcs_info.root_path,
                highlighter,
                self.path_filter.as_deref(),
            )?,
            DiffSource::StagedAndUnstaged | DiffSource::WorkingTree => {
                Self::get_working_tree_diff_with_ignore(
                    self.vcs.as_ref(),
                    &self.vcs_info.root_path,
                    highlighter,
                    self.path_filter.as_deref(),
                )?
            }
            DiffSource::PullRequest(_) => {
                // PR reload is a separate code path that may switch sessions
                // when the head SHA advances; callers dispatch via
                // `reload_pull_request` instead of going through this
                // local-reload helper.
                return Err(TuicrError::UnsupportedOperation(
                    "Use :reload from the command line in PR mode".to_string(),
                ));
            }
        };

        let mut invalidated = 0;
        for file in &diff_files {
            if self.session.add_diff_file(file) {
                invalidated += 1;
            }
        }

        self.diff_files = diff_files;
        self.clear_expanded_gaps();

        self.sort_files_by_directory(false);
        self.populate_file_line_count_cache();
        self.expand_all_dirs();

        if self.diff_files.is_empty() {
            self.diff_state.current_file_idx = 0;
            self.diff_state.cursor_line = 0;
            self.diff_state.scroll_offset = 0;
            self.file_list_state.select(0);
        } else {
            let target_idx = if let Some(path) = current_path {
                self.diff_files
                    .iter()
                    .position(|file| file.display_path() == &path)
                    .unwrap_or_else(|| prev_file_idx.min(self.diff_files.len().saturating_sub(1)))
            } else {
                prev_file_idx.min(self.diff_files.len().saturating_sub(1))
            };

            self.jump_to_file(target_idx);

            let file_start = self.calculate_file_scroll_offset(target_idx);
            let file_height = self.file_render_height(target_idx, &self.diff_files[target_idx]);
            let relative_line = prev_relative_line.min(file_height.saturating_sub(1));
            self.diff_state.cursor_line = file_start.saturating_add(relative_line);

            let viewport = self.diff_state.viewport_height.max(1);
            let max_relative = viewport.saturating_sub(1);
            let relative_offset = prev_viewport_offset.min(max_relative);
            if self.total_lines() == 0 {
                self.diff_state.scroll_offset = 0;
            } else {
                let max_scroll = self.max_scroll_offset();
                let desired = self
                    .diff_state
                    .cursor_line
                    .saturating_sub(relative_offset)
                    .min(max_scroll);
                self.diff_state.scroll_offset = desired;
            }

            self.ensure_cursor_visible();
            self.update_current_file_from_cursor();
        }

        self.rebuild_annotations();
        Ok((self.diff_files.len(), invalidated))
    }

    pub fn can_stage(&self) -> bool {
        matches!(
            self.diff_source,
            DiffSource::Unstaged | DiffSource::StagedAndUnstaged
        )
    }

    pub fn stage_reviewed_files(&mut self) {
        if !self.can_stage() {
            self.set_error("Staging only available when viewing unstaged diffs");
            return;
        }
        let reviewed_paths: Vec<_> = self
            .session
            .files
            .iter()
            .filter(|(_, review)| review.reviewed)
            .map(|(path, _)| path.clone())
            .collect();
        if reviewed_paths.is_empty() {
            self.set_warning("No reviewed files to stage");
            return;
        }
        let mut staged = 0;
        for path in &reviewed_paths {
            if let Err(e) = self.vcs.stage_file(path) {
                self.set_error(format!("Failed to stage {}: {e}", path.display()));
                return;
            }
            staged += 1;
        }
        self.set_message(format!("Staged {} reviewed file(s)", staged));
        if let Err(TuicrError::NoChanges) = self.reload_diff_files() {
            self.diff_files.clear();
            self.diff_state = DiffState::default();
            self.file_list_state = FileListState::default();
            self.clear_expanded_gaps();
            self.rebuild_annotations();
        }
    }

    pub fn current_file(&self) -> Option<&DiffFile> {
        self.diff_files.get(self.diff_state.current_file_idx)
    }

    pub fn current_file_path(&self) -> Option<&PathBuf> {
        self.current_file().map(|f| f.display_path())
    }

    /// Takes the queued editor target after action dispatch.
    ///
    /// The main event loop consumes this after leaving raw mode and the
    /// alternate screen,
    /// because `App` does not own terminal state.
    pub fn take_pending_editor_target(&mut self) -> Option<EditorTarget> {
        self.pending_editor_target.take()
    }

    /// Resolves the currently focused UI item into an editor target.
    ///
    /// The resolved target is queued on `pending_editor_target` so the main
    /// event loop can perform the terminal handoff.
    /// Invalid focus states are reported through the status bar instead.
    pub fn queue_editor_for_focused_item(&mut self) {
        match self.focused_panel {
            FocusedPanel::FileList => match self.get_selected_tree_item() {
                Some(FileTreeItem::File { file_idx, .. }) => {
                    self.queue_editor_for_file_idx(file_idx, None)
                }
                Some(FileTreeItem::Directory { .. }) => {
                    self.set_warning("Select a file to open in editor");
                }
                None => self.set_warning("No file selected"),
            },
            FocusedPanel::Diff => {
                let annotation = self.line_annotations.get(self.diff_state.cursor_line);
                let file_idx = match annotation {
                    Some(
                        AnnotatedLine::Expander { gap_id, .. }
                        | AnnotatedLine::HiddenLines { gap_id, .. }
                        | AnnotatedLine::ExpandedContext { gap_id, .. },
                    ) => gap_id.file_idx,
                    Some(annotation) => {
                        annotation_file_idx(annotation).unwrap_or(self.diff_state.current_file_idx)
                    }
                    None => self.diff_state.current_file_idx,
                };
                let line = match annotation {
                    Some(AnnotatedLine::ExpandedContext { gap_id, line_idx }) => self
                        .get_expanded_line(gap_id, *line_idx)
                        .and_then(|line| line.new_lineno.or(line.old_lineno)),
                    _ => self.get_line_at_cursor().map(|(line, _side)| line),
                };
                self.queue_editor_for_file_idx(file_idx, line);
            }
            FocusedPanel::Comments | FocusedPanel::CommitSelector => {
                self.set_warning("Focus a file or diff line to open in editor");
            }
        }
    }

    fn queue_editor_for_file_idx(&mut self, file_idx: usize, line: Option<u32>) {
        let Some(file) = self.diff_files.get(file_idx) else {
            self.set_warning("No file selected");
            return;
        };
        if file.is_commit_message {
            self.set_warning("Commit message has no local file to open");
            return;
        }

        let display_path = file.display_path().clone();
        // PR mode uses a synthetic forge root when there is no local checkout
        // to hand to an editor.
        if !self.vcs_info.root_path.is_absolute() {
            self.set_warning(format!(
                "Cannot open {}: no local checkout",
                display_path.display()
            ));
            return;
        }

        let path = self.vcs_info.root_path.join(&display_path);
        // Deleted files and remote-only PR files have diff rows,
        // but no worktree file the external editor can open.
        if !path.exists() {
            self.set_warning(format!(
                "Cannot open {}: file does not exist",
                path.display()
            ));
            return;
        }

        self.pending_editor_target = Some(EditorTarget { path, line });
    }

    pub fn toggle_reviewed(&mut self) {
        let file_idx = self.diff_state.current_file_idx;
        self.toggle_reviewed_for_file_idx(file_idx, true);
    }

    pub fn toggle_reviewed_for_file_idx(&mut self, file_idx: usize, adjust_cursor: bool) {
        let Some(path) = self
            .diff_files
            .get(file_idx)
            .map(|file| file.display_path().clone())
        else {
            return;
        };

        if let Some(review) = self.session.get_file_mut(&path) {
            review.reviewed = !review.reviewed;
            self.dirty = true;

            // Update current_file_idx before rebuilding annotations:
            // single-file view filters annotations against it.
            if adjust_cursor {
                self.diff_state.current_file_idx = file_idx;
            }
            self.rebuild_annotations();

            if adjust_cursor {
                let header_line = self.calculate_file_scroll_offset(file_idx);
                self.diff_state.cursor_line = header_line;
                self.ensure_cursor_visible();
            }
        }
    }

    fn hunk_at_cursor(&self) -> Option<(usize, usize)> {
        match self.line_annotations.get(self.diff_state.cursor_line)? {
            AnnotatedLine::HunkHeader { file_idx, hunk_idx }
            | AnnotatedLine::DiffLine {
                file_idx, hunk_idx, ..
            }
            | AnnotatedLine::SideBySideLine {
                file_idx, hunk_idx, ..
            } => Some((*file_idx, *hunk_idx)),
            _ => None,
        }
    }

    fn hunk_review_target(&self, file_idx: usize, hunk_idx: usize) -> Option<(PathBuf, String)> {
        let file = self.diff_files.get(file_idx)?;
        let key = file.hunk_review_key(hunk_idx)?;
        Some((file.display_path().clone(), key))
    }

    fn hunk_header_line(&self, file_idx: usize, hunk_idx: usize) -> Option<usize> {
        self.line_annotations.iter().position(|line| {
            matches!(
                line,
                AnnotatedLine::HunkHeader {
                    file_idx: candidate_file_idx,
                    hunk_idx: candidate_hunk_idx
                } if *candidate_file_idx == file_idx && *candidate_hunk_idx == hunk_idx
            )
        })
    }

    pub fn is_hunk_reviewed(&self, file_idx: usize, hunk_idx: usize) -> bool {
        // Skip key computation (which hashes every hunk in the file) when this
        // file has no reviewed hunks — the common case for users not using `R`.
        let Some(file) = self.diff_files.get(file_idx) else {
            return false;
        };
        match self.session.files.get(file.display_path()) {
            Some(review) if !review.reviewed_hunks.is_empty() => {}
            _ => return false,
        }

        let Some((path, key)) = self.hunk_review_target(file_idx, hunk_idx) else {
            return false;
        };
        self.session.is_hunk_reviewed(&path, &key)
    }

    pub fn should_render_gap_before_hunk(&self, file_idx: usize, hunk_idx: usize) -> bool {
        // Reviewed hunks collapse as a complete review unit: their body and
        // adjoining hidden-context controls disappear with the header.
        !self.is_hunk_reviewed(file_idx, hunk_idx)
            && (hunk_idx == 0 || !self.is_hunk_reviewed(file_idx, hunk_idx - 1))
    }

    pub fn toggle_hunk_reviewed(&mut self) {
        let Some((file_idx, hunk_idx)) = self.hunk_at_cursor() else {
            self.set_warning("Move cursor to a hunk to toggle reviewed");
            return;
        };

        let Some((path, key)) = self.hunk_review_target(file_idx, hunk_idx) else {
            self.set_warning("Move cursor to a hunk to toggle reviewed");
            return;
        };

        let Some(review) = self.session.get_file_mut(&path) else {
            return;
        };

        let reviewed = review.toggle_hunk_reviewed(key);
        self.dirty = true;
        self.rebuild_annotations();
        self.diff_state.current_file_idx = file_idx;
        if let Some(tree_idx) = self.file_idx_to_tree_idx(file_idx) {
            self.file_list_state.select(tree_idx);
        }
        if let Some(header_line) = self.hunk_header_line(file_idx, hunk_idx) {
            self.diff_state.cursor_line = header_line;
        }
        self.ensure_cursor_visible();

        if reviewed {
            self.set_message("Hunk marked reviewed");
        } else {
            self.set_message("Hunk marked unreviewed");
        }
    }

    pub fn file_count(&self) -> usize {
        self.diff_files.len()
    }

    pub fn reviewed_count(&self) -> usize {
        self.session.reviewed_count()
    }

    /// Returns `(total_files, total_additions, total_deletions)` across all diff files.
    pub fn diff_stat(&self) -> (usize, usize, usize) {
        let mut additions = 0;
        let mut deletions = 0;
        for file in &self.diff_files {
            let (a, d) = file.stat();
            additions += a;
            deletions += d;
        }
        (self.diff_files.len(), additions, deletions)
    }

    /// Returns true when the cursor is in the review comments area above all files.
    pub fn is_cursor_in_overview(&self) -> bool {
        self.diff_state.cursor_line < self.review_comments_render_height()
    }

    pub fn set_message(&mut self, msg: impl Into<String>) {
        self.set_message_inner(msg, MessageType::Info, Some(MESSAGE_TTL_INFO));
    }

    pub fn set_warning(&mut self, msg: impl Into<String>) {
        self.set_message_inner(msg, MessageType::Warning, Some(MESSAGE_TTL_WARNING));
    }

    pub fn set_error(&mut self, msg: impl Into<String>) {
        self.set_message_inner(msg, MessageType::Error, None);
    }

    /// Warning that stays until something else overwrites it. Used for state-tied
    /// messages like the dirty-quit prompt where the visual must outlive any TTL.
    pub fn set_sticky_warning(&mut self, msg: impl Into<String>) {
        self.set_message_inner(msg, MessageType::Warning, None);
    }

    fn set_message_inner(
        &mut self,
        msg: impl Into<String>,
        message_type: MessageType,
        ttl: Option<Duration>,
    ) {
        self.message = Some(Message {
            content: msg.into(),
            message_type,
            expires_at: ttl.map(|d| Instant::now() + d),
        });
    }

    pub fn clear_expired_message(&mut self) {
        let expired = self
            .message
            .as_ref()
            .and_then(|m| m.expires_at)
            .is_some_and(|t| Instant::now() >= t);
        if expired {
            self.message = None;
        }
    }

    pub fn cursor_down(&mut self, lines: usize) {
        let max_line = self.max_cursor_line();
        let prev_cursor = self.diff_state.cursor_line;
        let prev_scroll = self.diff_state.scroll_offset;
        let target = self.diff_state.cursor_line + lines;
        // Single-file view: first overflow press arms `primed_walk_next`
        // and parks the cursor on max. On kitty terminals the walk
        // consumes only when the Down key was released between the two
        // presses (`down_released_since_arm`) -- a held-j sequence emits
        // Press, Repeat, Repeat, ..., Release and never satisfies the
        // gate. Non-kitty terminals don't emit Release, so the gate
        // bypasses on `!supports_keyboard_enhancement` to keep the
        // two-press walk usable without modern keyboard reporting.
        self.primed_walk_prev = false;
        if self.is_single_file_view && target > max_line {
            let release_gate_ok =
                !self.supports_keyboard_enhancement || self.down_released_since_arm;
            if self.primed_walk_next && release_gate_ok {
                let next_idx = self.diff_state.current_file_idx + 1;
                if next_idx < self.diff_files.len() {
                    let overflow = target - max_line;
                    self.primed_walk_next = false;
                    self.down_released_since_arm = false;
                    self.jump_to_file(next_idx);
                    let new_top = self.diff_state.cursor_line;
                    let new_max = self.max_cursor_line();
                    self.diff_state.cursor_line =
                        (new_top + overflow.saturating_sub(1)).min(new_max);
                    self.ensure_cursor_visible();
                    return;
                }
                self.primed_walk_next = false;
                self.down_released_since_arm = false;
            } else {
                self.primed_walk_next = true;
                self.down_released_since_arm = false;
                self.diff_state.cursor_line = max_line;
                self.ensure_cursor_visible();
                return;
            }
        } else if target != prev_cursor {
            self.primed_walk_next = false;
            self.down_released_since_arm = false;
        }
        self.diff_state.cursor_line = target.min(max_line);
        if self.diff_state.cursor_line != prev_cursor {
            self.ensure_cursor_visible();
            // Cap scroll change to cursor movement to prevent multi-line jumps
            // when the view is catching up from a non-steady-state position.
            let cursor_moved = self.diff_state.cursor_line - prev_cursor;
            if self.diff_state.scroll_offset > prev_scroll + cursor_moved {
                self.diff_state.scroll_offset = prev_scroll + cursor_moved;
            }
        }
        self.update_current_file_from_cursor();
    }

    pub fn cursor_up(&mut self, lines: usize) {
        // Symmetric to cursor_down: first underflow arms `primed_walk_prev`,
        // second underflow consumes only after a Up/k release event has set
        // `up_released_since_arm`. See cursor_down for the gate rationale.
        self.primed_walk_next = false;
        if self.is_single_file_view {
            let file_top = self.calculate_file_scroll_offset(self.diff_state.current_file_idx);
            if self.diff_state.cursor_line < file_top + lines
                && self.diff_state.current_file_idx > 0
            {
                let release_gate_ok =
                    !self.supports_keyboard_enhancement || self.up_released_since_arm;
                if self.primed_walk_prev && release_gate_ok {
                    let underflow = (file_top + lines) - self.diff_state.cursor_line;
                    let prev_idx = self.diff_state.current_file_idx - 1;
                    self.primed_walk_prev = false;
                    self.up_released_since_arm = false;
                    self.jump_to_file(prev_idx);
                    let new_max = self.max_cursor_line();
                    self.diff_state.cursor_line =
                        new_max.saturating_sub(underflow.saturating_sub(1));
                    self.ensure_cursor_visible();
                    return;
                }
                self.primed_walk_prev = true;
                self.up_released_since_arm = false;
                self.diff_state.cursor_line = file_top;
                self.ensure_cursor_visible();
                return;
            }
            self.primed_walk_prev = false;
            self.up_released_since_arm = false;
        }
        self.diff_state.cursor_line = self.diff_state.cursor_line.saturating_sub(lines);
        let visible_lines = self.diff_state.effective_visible_lines();
        let scroll_margin = self.diff_state.effective_scroll_margin(self.scroll_offset);
        // Enforce top margin
        if self.diff_state.cursor_line < self.diff_state.scroll_offset + scroll_margin {
            self.diff_state.scroll_offset =
                self.diff_state.cursor_line.saturating_sub(scroll_margin);
        }
        // Ensure cursor is at least within the viewport (no bottom margin enforcement,
        // just basic visibility — handles viewport shrink or wrap-mode changes).
        if self.diff_state.cursor_line >= self.diff_state.scroll_offset + visible_lines {
            self.diff_state.scroll_offset = self.diff_state.cursor_line - visible_lines + 1;
        }
        self.update_current_file_from_cursor();
    }

    pub fn scroll_down(&mut self, lines: usize) {
        // For half-page/page scrolling, move both cursor and scroll
        let max_line = self.max_cursor_line();
        let max_scroll = self.max_scroll_offset();
        self.diff_state.cursor_line = (self.diff_state.cursor_line + lines).min(max_line);
        self.diff_state.scroll_offset = (self.diff_state.scroll_offset + lines).min(max_scroll);
        self.ensure_cursor_visible();
        self.update_current_file_from_cursor();
    }

    pub fn scroll_up(&mut self, lines: usize) {
        // For half-page/page scrolling, move both cursor and scroll
        self.diff_state.cursor_line = self.diff_state.cursor_line.saturating_sub(lines);
        self.diff_state.scroll_offset = self.diff_state.scroll_offset.saturating_sub(lines);
        self.ensure_cursor_visible();
        self.update_current_file_from_cursor();
    }

    pub fn scroll_view_down(&mut self, lines: usize) {
        let max_scroll = self.max_scroll_offset();
        self.diff_state.scroll_offset = (self.diff_state.scroll_offset + lines).min(max_scroll);
        let scroll_margin = self.diff_state.effective_scroll_margin(self.scroll_offset);
        let min_cursor =
            (self.diff_state.scroll_offset + scroll_margin).min(self.max_cursor_line());
        if self.diff_state.cursor_line < min_cursor {
            self.diff_state.cursor_line = min_cursor;
            self.update_current_file_from_cursor();
        }
    }

    pub fn scroll_view_up(&mut self, lines: usize) {
        self.diff_state.scroll_offset = self.diff_state.scroll_offset.saturating_sub(lines);
        let visible_lines = if self.diff_state.visible_line_count > 0 {
            self.diff_state.visible_line_count
        } else {
            self.diff_state.viewport_height.max(1)
        };
        let bottom = self.diff_state.scroll_offset + visible_lines.saturating_sub(1);
        if self.diff_state.cursor_line > bottom {
            self.diff_state.cursor_line = bottom;
            self.update_current_file_from_cursor();
        }
    }

    pub fn scroll_left(&mut self, cols: usize) {
        if self.diff_state.wrap_lines {
            return;
        }
        self.diff_state.scroll_x = self.diff_state.scroll_x.saturating_sub(cols);
    }

    pub fn scroll_right(&mut self, cols: usize) {
        if self.diff_state.wrap_lines {
            return;
        }
        let max_scroll_x = self
            .diff_state
            .max_content_width
            .saturating_sub(self.diff_state.viewport_width);
        self.diff_state.scroll_x =
            (self.diff_state.scroll_x.saturating_add(cols)).min(max_scroll_x);
    }

    pub fn toggle_diff_wrap(&mut self) {
        let enabled = !self.diff_state.wrap_lines;
        self.set_diff_wrap(enabled);
    }

    pub fn set_diff_wrap(&mut self, enabled: bool) {
        self.diff_state.wrap_lines = enabled;
        if enabled {
            self.diff_state.scroll_x = 0;
        }
        let status = if self.diff_state.wrap_lines {
            "on"
        } else {
            "off"
        };
        self.set_message(format!("Diff wrapping: {status}"));
    }

    /// Adjusts scroll_offset so the cursor stays within the visible viewport,
    /// respecting the configured scroll margin (minimum lines from edge).
    fn ensure_cursor_visible(&mut self) {
        // Use visible_line_count which is computed during render based on actual line widths.
        // Falls back to viewport_height if not yet set (before first render).
        let visible_lines = self.diff_state.effective_visible_lines();
        let max_scroll = self.max_scroll_offset();
        let scroll_margin = self.diff_state.effective_scroll_margin(self.scroll_offset);
        // Cursor too close to the top edge — scroll up
        if self.diff_state.cursor_line < self.diff_state.scroll_offset + scroll_margin {
            self.diff_state.scroll_offset =
                self.diff_state.cursor_line.saturating_sub(scroll_margin);
        }
        // Cursor too close to the bottom edge — scroll down.
        // Reduce the margin near EOF so we don't scroll to show empty space
        // when the last line is already visible (matches Vim behavior).
        let lines_below = self
            .max_cursor_line()
            .saturating_sub(self.diff_state.cursor_line);
        let bottom_margin = scroll_margin.min(lines_below);
        if self.diff_state.cursor_line + bottom_margin
            >= self.diff_state.scroll_offset + visible_lines
        {
            self.diff_state.scroll_offset =
                (self.diff_state.cursor_line + bottom_margin - visible_lines + 1).min(max_scroll);
        }
    }

    pub fn search_in_diff_from_cursor(&mut self) -> bool {
        let pattern = self.search_buffer.clone();
        if pattern.trim().is_empty() {
            self.set_message("Search pattern is empty");
            return false;
        }

        self.last_search_pattern = Some(pattern.clone());
        self.search_in_diff(&pattern, self.diff_state.cursor_line, true, true)
    }

    pub fn search_next_in_diff(&mut self) -> bool {
        let Some(pattern) = self.last_search_pattern.clone() else {
            self.set_message("No previous search");
            return false;
        };
        self.search_in_diff(&pattern, self.diff_state.cursor_line, true, false)
    }

    pub fn search_prev_in_diff(&mut self) -> bool {
        let Some(pattern) = self.last_search_pattern.clone() else {
            self.set_message("No previous search");
            return false;
        };
        self.search_in_diff(&pattern, self.diff_state.cursor_line, false, false)
    }

    fn search_in_diff(
        &mut self,
        pattern: &str,
        start_idx: usize,
        forward: bool,
        include_current: bool,
    ) -> bool {
        let total_lines = self.total_lines();
        if total_lines == 0 {
            self.set_message("No diff content to search");
            return false;
        }

        if forward {
            let mut idx = start_idx.min(total_lines.saturating_sub(1));
            if !include_current {
                idx = idx.saturating_add(1);
            }
            for line_idx in idx..total_lines {
                if let Some(text) = self.line_text_for_search(line_idx)
                    && text.contains(pattern)
                {
                    self.diff_state.cursor_line = line_idx;
                    self.ensure_cursor_visible();
                    self.center_cursor();
                    self.update_current_file_from_cursor();
                    return true;
                }
            }
        } else {
            let mut idx = start_idx.min(total_lines.saturating_sub(1));
            if !include_current {
                idx = idx.saturating_sub(1);
            }
            let mut line_idx = idx;
            loop {
                if let Some(text) = self.line_text_for_search(line_idx)
                    && text.contains(pattern)
                {
                    self.diff_state.cursor_line = line_idx;
                    self.ensure_cursor_visible();
                    self.center_cursor();
                    self.update_current_file_from_cursor();
                    return true;
                }
                if line_idx == 0 {
                    break;
                }
                line_idx = line_idx.saturating_sub(1);
            }
        }

        self.set_message(format!("No matches for \"{pattern}\""));
        false
    }

    fn line_text_for_search(&self, line_idx: usize) -> Option<String> {
        match self.line_annotations.get(line_idx)? {
            AnnotatedLine::ReviewCommentsHeader => Some("Review comments".to_string()),
            AnnotatedLine::ReviewComment { comment_idx } => {
                let comment = self.session.review_comments.get(*comment_idx)?;
                Some(comment.content.clone())
            }
            AnnotatedLine::RemoteReviewSummaryLine { summary_idx } => {
                let summary = self.forge_review_summaries.get(*summary_idx)?;
                let author = summary.author.as_deref().unwrap_or("unknown");
                Some(format!("github @{author} {}", summary.body))
            }
            AnnotatedLine::FileHeader { file_idx } => {
                let file = self.diff_files.get(*file_idx)?;
                Some(format!(
                    "{} [{}]",
                    file.display_path().display(),
                    file.status.as_char()
                ))
            }
            AnnotatedLine::FileComment {
                file_idx,
                comment_idx,
            } => {
                let path = self.diff_files.get(*file_idx)?.display_path();
                let review = self.session.files.get(path)?;
                let comment = review.file_comments.get(*comment_idx)?;
                Some(comment.content.clone())
            }
            AnnotatedLine::LineComment {
                file_idx,
                line,
                comment_idx,
                ..
            } => {
                let path = self.diff_files.get(*file_idx)?.display_path();
                let review = self.session.files.get(path)?;
                let comments = review.line_comments.get(line)?;
                let comment = comments.get(*comment_idx)?;
                Some(comment.content.clone())
            }
            AnnotatedLine::Expander { gap_id, direction } => {
                let arrow = match direction {
                    ExpandDirection::Down => "↓",
                    ExpandDirection::Up => "↑",
                    ExpandDirection::Both => "↕",
                };
                let gap = self.gap_size(gap_id)?;
                let top_len = self.expanded_top.get(gap_id).map_or(0, |v| v.len());
                let bot_len = self.expanded_bottom.get(gap_id).map_or(0, |v| v.len());
                let remaining = (gap as usize).saturating_sub(top_len + bot_len);
                let count = remaining.min(GAP_EXPAND_BATCH);
                Some(format!("... {arrow} expand ({count} lines) ..."))
            }
            AnnotatedLine::HiddenLines { count, .. } => {
                Some(format!("... {count} lines hidden ..."))
            }
            AnnotatedLine::ExpandedContext {
                gap_id,
                line_idx: context_idx,
            } => {
                let content = self.get_expanded_line(gap_id, *context_idx)?;
                Some(content.content.clone())
            }
            AnnotatedLine::HunkHeader { file_idx, hunk_idx } => {
                let file = self.diff_files.get(*file_idx)?;
                let hunk = file.hunks.get(*hunk_idx)?;
                Some(hunk.header.clone())
            }
            AnnotatedLine::DiffLine {
                file_idx,
                hunk_idx,
                line_idx: diff_idx,
                ..
            } => {
                let file = self.diff_files.get(*file_idx)?;
                let hunk = file.hunks.get(*hunk_idx)?;
                let line = hunk.lines.get(*diff_idx)?;
                Some(line.content.clone())
            }
            AnnotatedLine::BinaryOrEmpty { file_idx } => {
                let file = self.diff_files.get(*file_idx)?;
                if file.is_too_large {
                    Some("(file too large to display)".to_string())
                } else if file.is_binary {
                    Some("(binary file)".to_string())
                } else {
                    Some("(no changes)".to_string())
                }
            }
            AnnotatedLine::SideBySideLine {
                file_idx,
                hunk_idx,
                del_line_idx,
                add_line_idx,
                ..
            } => {
                let file = self.diff_files.get(*file_idx)?;
                let hunk = file.hunks.get(*hunk_idx)?;

                let del_content = del_line_idx
                    .and_then(|idx| hunk.lines.get(idx))
                    .map(|l| l.content.as_str())
                    .unwrap_or("");
                let add_content = add_line_idx
                    .and_then(|idx| hunk.lines.get(idx))
                    .map(|l| l.content.as_str())
                    .unwrap_or("");
                Some(format!("{} {}", del_content, add_content))
            }
            AnnotatedLine::RemoteThreadLine { thread_idx } => {
                let thread = self.forge_review_threads.get(*thread_idx)?;
                // Search matches any text in the thread (including replies).
                let mut bodies: Vec<String> =
                    thread.comments.iter().map(|c| c.body.clone()).collect();
                bodies.insert(0, format!("github {}", thread.path));
                Some(bodies.join(" "))
            }
            AnnotatedLine::Spacing => None,
        }
    }

    fn gap_size(&self, gap_id: &GapId) -> Option<u32> {
        let file = self.diff_files.get(gap_id.file_idx)?;

        if gap_id.hunk_idx == file.hunks.len() {
            // End-of-file gap
            let last_hunk = file.hunks.last()?;
            let start = last_hunk.new_start + last_hunk.new_count;
            let end = *self.file_line_count_cache.get(&gap_id.file_idx)?;
            if start > end {
                Some(0)
            } else {
                Some(end - start + 1)
            }
        } else {
            let hunk = file.hunks.get(gap_id.hunk_idx)?;
            let prev_hunk = if gap_id.hunk_idx > 0 {
                file.hunks.get(gap_id.hunk_idx - 1)
            } else {
                None
            };
            Some(calculate_gap(
                prev_hunk.map(|h| (&h.new_start, &h.new_count)),
                hunk.new_start,
            ))
        }
    }

    pub fn center_cursor(&mut self) {
        let viewport = self.diff_state.viewport_height.max(1);
        let half_viewport = viewport / 2;
        let max_scroll = self.max_scroll_offset();
        self.diff_state.scroll_offset = self
            .diff_state
            .cursor_line
            .saturating_sub(half_viewport)
            .min(max_scroll);
    }

    pub fn cursor_to_top(&mut self) {
        let scroll_margin = self.diff_state.effective_scroll_margin(self.scroll_offset);
        let max_scroll = self.max_scroll_offset();
        self.diff_state.scroll_offset = self
            .diff_state
            .cursor_line
            .saturating_sub(scroll_margin)
            .min(max_scroll);
    }

    pub fn cursor_to_bottom(&mut self) {
        let visible_lines = self.diff_state.effective_visible_lines();
        let scroll_margin = self.diff_state.effective_scroll_margin(self.scroll_offset);
        let max_scroll = self.max_scroll_offset();
        self.diff_state.scroll_offset = self
            .diff_state
            .cursor_line
            .saturating_sub(visible_lines.saturating_sub(1 + scroll_margin))
            .min(max_scroll);
    }

    pub fn go_to_source_line(&mut self, target_lineno: u32, side: LineSide) {
        let current_file = self.diff_state.current_file_idx;
        let mut result = self.find_source_line_in_diff(target_lineno, side);
        let side_label = match side {
            LineSide::New => "",
            LineSide::Old => " (old)",
        };

        // If the line isn't already annotated, see whether it falls inside a
        // collapsed (or partially collapsed) gap between hunks. If so, expand
        // *toward* the target from whichever side the cursor is on: cursor
        // above the gap expands `Down` from the previous hunk, cursor at or
        // below the gap expands `Up` from the next hunk. Either way the
        // unreached half of the gap stays collapsed behind an expander.
        if !matches!(result, FindSourceLineResult::Exact(_))
            && let Some(gap_id) = self.find_gap_containing_lineno(current_file, target_lineno, side)
        {
            let (direction, limit) = self.expand_plan_to_reach(&gap_id, target_lineno, side);
            if let Err(e) = self.expand_gap(gap_id, direction, limit) {
                self.set_error(format!("Expand failed: {e}"));
                return;
            }
            result = self.find_source_line_in_diff(target_lineno, side);
        }

        match result {
            FindSourceLineResult::Exact(idx) | FindSourceLineResult::Nearest(idx) => {
                self.diff_state.cursor_line = idx;
                self.ensure_cursor_visible();
                self.center_cursor();
                self.update_current_file_from_cursor();
                if matches!(result, FindSourceLineResult::Nearest(_)) {
                    self.set_message(format!(
                        "Line {target_lineno}{side_label} not in diff, jumped to nearest"
                    ));
                }
            }
            FindSourceLineResult::NotFound => {
                self.set_warning(format!(
                    "Line {target_lineno}{side_label} not found in current file"
                ));
            }
        }
    }

    /// Like the free `find_source_line` but also resolves `ExpandedContext`
    /// annotations through `get_expanded_line` so newly-revealed context lines
    /// count toward the match.
    fn find_source_line_in_diff(&self, target_lineno: u32, side: LineSide) -> FindSourceLineResult {
        let current_file = self.diff_state.current_file_idx;
        let mut best: Option<(usize, u32)> = None;

        for (idx, annotation) in self.line_annotations.iter().enumerate() {
            let (file_idx, candidate) = match annotation {
                AnnotatedLine::DiffLine {
                    file_idx,
                    old_lineno,
                    new_lineno,
                    ..
                }
                | AnnotatedLine::SideBySideLine {
                    file_idx,
                    old_lineno,
                    new_lineno,
                    ..
                } => {
                    let c = match side {
                        LineSide::New => *new_lineno,
                        LineSide::Old => *old_lineno,
                    };
                    (*file_idx, c)
                }
                AnnotatedLine::ExpandedContext { gap_id, line_idx } => {
                    let Some(line) = self.get_expanded_line(gap_id, *line_idx) else {
                        continue;
                    };
                    let c = match side {
                        LineSide::New => line.new_lineno,
                        LineSide::Old => line.old_lineno,
                    };
                    (gap_id.file_idx, c)
                }
                _ => continue,
            };
            if file_idx != current_file {
                continue;
            }
            if let Some(ln) = candidate {
                let dist = ln.abs_diff(target_lineno);
                if dist == 0 {
                    return FindSourceLineResult::Exact(idx);
                }
                if best.is_none() || dist < best.unwrap().1 {
                    best = Some((idx, dist));
                }
            }
        }

        match best {
            Some((idx, _)) => FindSourceLineResult::Nearest(idx),
            None => FindSourceLineResult::NotFound,
        }
    }

    /// Return the gap (between-hunk or pre-first-hunk region) in `file_idx`
    /// whose line range on `side` contains `target_lineno`, if any. Used by
    /// `go_to_source_line` to auto-expand collapsed context when the user
    /// jumps to a line that's hidden behind an expander.
    fn find_gap_containing_lineno(
        &self,
        file_idx: usize,
        target_lineno: u32,
        side: LineSide,
    ) -> Option<GapId> {
        let file = self.diff_files.get(file_idx)?;
        for hunk_idx in 0..file.hunks.len() {
            let hunk = &file.hunks[hunk_idx];
            let prev_hunk = if hunk_idx > 0 {
                Some(&file.hunks[hunk_idx - 1])
            } else {
                None
            };
            let (start, end) = match side {
                LineSide::New => match prev_hunk {
                    None => (1, hunk.new_start.saturating_sub(1)),
                    Some(p) => (p.new_start + p.new_count, hunk.new_start.saturating_sub(1)),
                },
                LineSide::Old => match prev_hunk {
                    None => (1, hunk.old_start.saturating_sub(1)),
                    Some(p) => (p.old_start + p.old_count, hunk.old_start.saturating_sub(1)),
                },
            };
            if start <= end && target_lineno >= start && target_lineno <= end {
                return Some(GapId { file_idx, hunk_idx });
            }
        }

        // Check the EOF gap (after the last hunk).
        if let Some(last) = file.hunks.last()
            && let Some(&total) = self.file_line_count_cache.get(&file_idx)
        {
            let (start, end) = match side {
                LineSide::New => (last.new_start + last.new_count, total),
                LineSide::Old => {
                    let delta = (last.new_start + last.new_count) as i64
                        - (last.old_start + last.old_count) as i64;
                    let new_start = last.new_start + last.new_count;
                    let old_start = (new_start as i64 - delta) as u32;
                    let old_end = (total as i64 - delta) as u32;
                    (old_start, old_end)
                }
            };
            if start <= end && target_lineno >= start && target_lineno <= end {
                return Some(GapId {
                    file_idx,
                    hunk_idx: file.hunks.len(),
                });
            }
        }
        None
    }

    /// Choose an `ExpandDirection` and line-count limit so that expanding
    /// `gap_id` reveals exactly the lines between the cursor and
    /// `target_lineno` (on `side`). Cursor above the gap → `Down` from the
    /// previous hunk; cursor at-or-below the gap → `Up` from the next hunk.
    ///
    /// `expand_gap` operates in new-side coordinates, so an old-side target
    /// is translated to new-side using the offset that holds across an
    /// unchanged-context gap: `new - old = hunk.new_start - hunk.old_start`
    /// where `hunk` is the hunk immediately *after* the gap.
    fn expand_plan_to_reach(
        &self,
        gap_id: &GapId,
        target_lineno: u32,
        side: LineSide,
    ) -> (ExpandDirection, Option<usize>) {
        let Some(file) = self.diff_files.get(gap_id.file_idx) else {
            return (ExpandDirection::Both, None);
        };
        // EOF gap: no next hunk, only expand downward from the last hunk.
        let is_eof_gap = gap_id.hunk_idx == file.hunks.len();
        if is_eof_gap {
            let Some(last) = file.hunks.last() else {
                return (ExpandDirection::Both, None);
            };
            let gap_start_new = last.new_start + last.new_count;
            let offset_new_minus_old =
                (last.new_start + last.new_count) as i64 - (last.old_start + last.old_count) as i64;
            let target_new = match side {
                LineSide::New => target_lineno as i64,
                LineSide::Old => target_lineno as i64 + offset_new_minus_old,
            };
            let top_len = self.expanded_top.get(gap_id).map_or(0, |v| v.len()) as i64;
            let inner_start = gap_start_new as i64 + top_len;
            let limit = (target_new - inner_start + 1).max(0) as usize;
            return (ExpandDirection::Down, Some(limit));
        }

        let hunk = &file.hunks[gap_id.hunk_idx];
        let prev_hunk = if gap_id.hunk_idx > 0 {
            Some(&file.hunks[gap_id.hunk_idx - 1])
        } else {
            None
        };
        let gap_start_new = match prev_hunk {
            None => 1,
            Some(p) => p.new_start + p.new_count,
        };
        let gap_end_new = hunk.new_start.saturating_sub(1);
        // offset := new - old, constant across the unchanged context gap
        let offset_new_minus_old = hunk.new_start as i64 - hunk.old_start as i64;
        let target_new = match side {
            LineSide::New => target_lineno as i64,
            LineSide::Old => target_lineno as i64 + offset_new_minus_old,
        };

        let cursor_below_gap = self
            .find_hunk_header_annotation_idx(gap_id)
            .is_some_and(|h| self.diff_state.cursor_line >= h);

        if cursor_below_gap {
            let bot_len = self.expanded_bottom.get(gap_id).map_or(0, |v| v.len()) as i64;
            let inner_end = gap_end_new as i64 - bot_len;
            let limit = (inner_end - target_new + 1).max(0) as usize;
            (ExpandDirection::Up, Some(limit))
        } else {
            let top_len = self.expanded_top.get(gap_id).map_or(0, |v| v.len()) as i64;
            let inner_start = gap_start_new as i64 + top_len;
            let limit = (target_new - inner_start + 1).max(0) as usize;
            (ExpandDirection::Down, Some(limit))
        }
    }

    fn find_hunk_header_annotation_idx(&self, gap_id: &GapId) -> Option<usize> {
        self.line_annotations
            .iter()
            .enumerate()
            .find_map(|(idx, a)| match a {
                AnnotatedLine::HunkHeader { file_idx, hunk_idx }
                    if *file_idx == gap_id.file_idx && *hunk_idx == gap_id.hunk_idx =>
                {
                    Some(idx)
                }
                _ => None,
            })
    }

    pub fn file_list_down(&mut self, n: usize) {
        let visible_items = self.build_visible_items();
        let max_idx = visible_items.len().saturating_sub(1);
        let new_idx = (self.file_list_state.selected() + n).min(max_idx);
        self.file_list_state.select(new_idx);
        self.follow_file_list_in_single_file_view();
    }

    pub fn file_list_up(&mut self, n: usize) {
        let new_idx = self.file_list_state.selected().saturating_sub(n);
        self.file_list_state.select(new_idx);
        self.follow_file_list_in_single_file_view();
    }

    /// In single-file view the diff panel always shows one file at a time,
    /// so navigating the file list with j/k should reveal the highlighted
    /// file immediately instead of waiting for Enter. Skips directories
    /// (jumping there has no diff target) and no-ops outside single-file
    /// view to keep multi-file scrolling exactly as before.
    fn follow_file_list_in_single_file_view(&mut self) {
        if !self.is_single_file_view {
            return;
        }
        if let Some(FileTreeItem::File { file_idx, .. }) = self.get_selected_tree_item() {
            self.jump_to_file(file_idx);
        }
    }

    /// Scroll the file-list viewport down by `lines` without moving the
    /// selection unless it would fall off the top of the viewport.
    pub fn file_list_viewport_scroll_down(&mut self, lines: usize) {
        let total = self.build_visible_items().len();
        let viewport = self.file_list_state.viewport_height.max(1);
        let max_offset = total.saturating_sub(viewport);
        let new_offset = (self.file_list_state.list_state.offset() + lines).min(max_offset);
        *self.file_list_state.list_state.offset_mut() = new_offset;
        if self.file_list_state.selected() < new_offset {
            self.file_list_state.select(new_offset);
        }
    }

    /// Scroll the file-list viewport up by `lines` without moving the
    /// selection unless it would fall off the bottom of the viewport.
    pub fn file_list_viewport_scroll_up(&mut self, lines: usize) {
        let viewport = self.file_list_state.viewport_height.max(1);
        let new_offset = self
            .file_list_state
            .list_state
            .offset()
            .saturating_sub(lines);
        *self.file_list_state.list_state.offset_mut() = new_offset;
        let max_visible = (new_offset + viewport).saturating_sub(1);
        if self.file_list_state.selected() > max_visible {
            self.file_list_state.select(max_visible);
        }
    }

    pub fn diff_annotation_at_screen_row(&self, screen_row: u16) -> Option<usize> {
        let inner = self.diff_inner_area?;
        if screen_row < inner.y || screen_row >= inner.y + inner.height {
            return None;
        }
        let rel = (screen_row - inner.y) as usize;
        self.diff_row_to_annotation.get(rel).copied()
    }

    pub fn file_list_idx_at_screen_row(&self, screen_row: u16) -> Option<usize> {
        let inner = self.file_list_inner_area?;
        if screen_row < inner.y || screen_row >= inner.y + inner.height {
            return None;
        }
        let rel = (screen_row - inner.y) as usize;
        let idx = self.file_list_state.list_state.offset() + rel;
        let total = self.build_visible_items().len();
        (idx < total).then_some(idx)
    }

    pub fn comment_navigator_idx_at_screen_row(&self, screen_row: u16) -> Option<usize> {
        let inner = self.comment_navigator_inner_area?;
        if screen_row < inner.y || screen_row >= inner.y + inner.height {
            return None;
        }
        let rel = (screen_row - inner.y) as usize;
        let idx = self.comment_navigator_state.list_state.offset() + rel;
        let total = self.build_comment_navigator_items().len();
        (idx < total).then_some(idx)
    }

    fn comment_navigator_key(annotation: &AnnotatedLine) -> Option<CommentNavigatorKey> {
        match annotation {
            AnnotatedLine::ReviewComment { comment_idx } => Some(CommentNavigatorKey::Review {
                comment_idx: *comment_idx,
            }),
            AnnotatedLine::FileComment {
                file_idx,
                comment_idx,
            } => Some(CommentNavigatorKey::File {
                file_idx: *file_idx,
                comment_idx: *comment_idx,
            }),
            AnnotatedLine::LineComment {
                file_idx,
                line,
                side,
                comment_idx,
            } => Some(CommentNavigatorKey::Line {
                file_idx: *file_idx,
                line: *line,
                side: *side,
                comment_idx: *comment_idx,
            }),
            AnnotatedLine::RemoteThreadLine { thread_idx } => Some(CommentNavigatorKey::Remote {
                thread_idx: *thread_idx,
            }),
            AnnotatedLine::RemoteReviewSummaryLine { summary_idx } => {
                Some(CommentNavigatorKey::RemoteReview {
                    summary_idx: *summary_idx,
                })
            }
            _ => None,
        }
    }

    fn comment_navigator_item_for_key(
        &self,
        key: CommentNavigatorKey,
        target_annotation: usize,
    ) -> Option<CommentNavigatorItem> {
        match key {
            CommentNavigatorKey::Review { comment_idx } => {
                let comment = self.session.review_comments.get(comment_idx)?;
                Some(CommentNavigatorItem {
                    key: CommentNavigatorKey::Review { comment_idx },
                    kind: CommentNavigatorKind::Local(comment.comment_type.clone()),
                    target_annotation,
                    path: None,
                    line: None,
                    side: None,
                    author: Some(comment.author.clone()),
                })
            }
            CommentNavigatorKey::File {
                file_idx,
                comment_idx,
            } => {
                let path = self.diff_files.get(file_idx)?.display_path();
                let review = self.session.files.get(path)?;
                let comment = review.file_comments.get(comment_idx)?;
                Some(CommentNavigatorItem {
                    key: CommentNavigatorKey::File {
                        file_idx,
                        comment_idx,
                    },
                    kind: CommentNavigatorKind::Local(comment.comment_type.clone()),
                    target_annotation,
                    path: Some(path.display().to_string()),
                    line: None,
                    side: None,
                    author: Some(comment.author.clone()),
                })
            }
            CommentNavigatorKey::Line {
                file_idx,
                line,
                side,
                comment_idx,
            } => {
                let path = self.diff_files.get(file_idx)?.display_path();
                let review = self.session.files.get(path)?;
                let comments = review.line_comments.get(&line)?;
                let comment = comments.get(comment_idx)?;
                Some(CommentNavigatorItem {
                    key: CommentNavigatorKey::Line {
                        file_idx,
                        line,
                        side,
                        comment_idx,
                    },
                    kind: CommentNavigatorKind::Local(comment.comment_type.clone()),
                    target_annotation,
                    path: Some(path.display().to_string()),
                    line: Some(line),
                    side: Some(side),
                    author: Some(comment.author.clone()),
                })
            }
            CommentNavigatorKey::Remote { thread_idx } => {
                let thread = self.forge_review_threads.get(thread_idx)?;
                let muted = self
                    .session
                    .remote_comments_visibility
                    .render_decision(thread)?;
                let side = match thread.side {
                    crate::forge::remote_comments::RemoteCommentSide::Right => LineSide::New,
                    crate::forge::remote_comments::RemoteCommentSide::Left => LineSide::Old,
                };
                let author = thread.root().and_then(|c| c.author.clone());
                Some(CommentNavigatorItem {
                    key: CommentNavigatorKey::Remote { thread_idx },
                    kind: CommentNavigatorKind::Remote { muted },
                    target_annotation,
                    path: Some(thread.path.clone()),
                    line: thread.line,
                    side: Some(side),
                    author,
                })
            }
            CommentNavigatorKey::RemoteReview { summary_idx } => {
                let summary = self.forge_review_summaries.get(summary_idx)?;
                Some(CommentNavigatorItem {
                    key: CommentNavigatorKey::RemoteReview { summary_idx },
                    kind: CommentNavigatorKind::Remote { muted: false },
                    target_annotation,
                    path: None,
                    line: None,
                    side: None,
                    author: summary.author.clone(),
                })
            }
        }
    }

    pub fn build_comment_navigator_items(&self) -> Vec<CommentNavigatorItem> {
        let mut items = Vec::new();
        let mut last_key: Option<CommentNavigatorKey> = None;

        for (idx, annotation) in self.line_annotations.iter().enumerate() {
            let Some(key) = Self::comment_navigator_key(annotation) else {
                last_key = None;
                continue;
            };

            if last_key.as_ref() == Some(&key) {
                continue;
            }

            if let Some(item) = self.comment_navigator_item_for_key(key.clone(), idx) {
                items.push(item);
                last_key = Some(key);
            }
        }

        items
    }

    pub fn has_comment_navigator_items(&self) -> bool {
        !self.build_comment_navigator_items().is_empty()
    }

    pub fn sync_comment_navigator_selection(&mut self, items: &[CommentNavigatorItem]) {
        if items.is_empty() {
            self.comment_navigator_state.list_state.select(None);
            return;
        }

        if self.focused_panel == FocusedPanel::Diff
            && let Some(annotation) = self.line_annotations.get(self.diff_state.cursor_line)
            && let Some(key) = Self::comment_navigator_key(annotation)
            && let Some(idx) = items.iter().position(|item| item.key == key)
        {
            self.comment_navigator_state.select(idx);
            return;
        }

        let selected = self
            .comment_navigator_state
            .selected()
            .min(items.len().saturating_sub(1));
        self.comment_navigator_state.select(selected);
    }

    pub fn comment_navigator_down(&mut self, n: usize) {
        let items = self.build_comment_navigator_items();
        let max_idx = items.len().saturating_sub(1);
        let new_idx = (self.comment_navigator_state.selected() + n).min(max_idx);
        self.comment_navigator_state.select(new_idx);
    }

    pub fn comment_navigator_up(&mut self, n: usize) {
        let new_idx = self.comment_navigator_state.selected().saturating_sub(n);
        self.comment_navigator_state.select(new_idx);
    }

    pub fn comment_navigator_viewport_scroll_down(&mut self, lines: usize) {
        let total = self.build_comment_navigator_items().len();
        let viewport = self.comment_navigator_state.viewport_height.max(1);
        let max_offset = total.saturating_sub(viewport);
        let new_offset = (self.comment_navigator_state.list_state.offset() + lines).min(max_offset);
        *self.comment_navigator_state.list_state.offset_mut() = new_offset;
        if self.comment_navigator_state.selected() < new_offset {
            self.comment_navigator_state.select(new_offset);
        }
    }

    pub fn comment_navigator_viewport_scroll_up(&mut self, lines: usize) {
        let viewport = self.comment_navigator_state.viewport_height.max(1);
        let new_offset = self
            .comment_navigator_state
            .list_state
            .offset()
            .saturating_sub(lines);
        *self.comment_navigator_state.list_state.offset_mut() = new_offset;
        let max_visible = (new_offset + viewport).saturating_sub(1);
        if self.comment_navigator_state.selected() > max_visible {
            self.comment_navigator_state.select(max_visible);
        }
    }

    pub fn jump_to_selected_comment(&mut self) -> bool {
        let items = self.build_comment_navigator_items();
        let Some(item) = items.get(self.comment_navigator_state.selected()) else {
            self.set_message("No comments to navigate");
            return false;
        };
        self.move_cursor_to_annotation(item.target_annotation);
        self.center_cursor();
        self.focused_panel = FocusedPanel::Diff;
        true
    }

    pub fn commit_list_idx_at_screen_row(&self, screen_row: u16) -> Option<usize> {
        let inner = self.commit_list_inner_area?;
        if screen_row < inner.y || screen_row >= inner.y + inner.height {
            return None;
        }
        let rel = (screen_row - inner.y) as usize;
        let idx = self.commit_list_scroll_offset + rel;
        let total = match self.input_mode {
            InputMode::CommitSelect => {
                self.visible_commit_count + usize::from(self.can_show_more_commits())
            }
            _ => self.review_commits.len(),
        };
        (idx < total).then_some(idx)
    }

    /// Syncs `current_file_idx` so the file list selection follows when the
    /// new cursor lands on an annotation belonging to a file.
    pub fn move_cursor_to_annotation(&mut self, idx: usize) {
        if idx >= self.line_annotations.len() {
            return;
        }
        self.diff_state.cursor_line = idx;
        if let Some(file_idx) = annotation_file_idx(&self.line_annotations[idx]) {
            self.diff_state.current_file_idx = file_idx;
        }
        let viewport = self.diff_state.viewport_height.max(1);
        if idx < self.diff_state.scroll_offset {
            self.diff_state.scroll_offset = idx;
        } else if idx >= self.diff_state.scroll_offset + viewport {
            self.diff_state.scroll_offset = idx + 1 - viewport;
        }
    }

    /// In SBS, picks Old or New per `side`, falling back to the other pane
    /// if the requested one is empty. Unified diff rows ignore `side`.
    pub fn content_for_side(&self, ann_idx: usize, side: LineSide) -> Option<&str> {
        let ann = self.line_annotations.get(ann_idx)?;
        match ann {
            AnnotatedLine::DiffLine {
                file_idx,
                hunk_idx,
                line_idx,
                ..
            } => {
                let line = self
                    .diff_files
                    .get(*file_idx)?
                    .hunks
                    .get(*hunk_idx)?
                    .lines
                    .get(*line_idx)?;
                Some(line.content.as_str())
            }
            AnnotatedLine::SideBySideLine {
                file_idx,
                hunk_idx,
                del_line_idx,
                add_line_idx,
                ..
            } => {
                let hunk = self.diff_files.get(*file_idx)?.hunks.get(*hunk_idx)?;
                let add = add_line_idx
                    .and_then(|i| hunk.lines.get(i))
                    .map(|l| l.content.as_str());
                let del = del_line_idx
                    .and_then(|i| hunk.lines.get(i))
                    .map(|l| l.content.as_str());
                match side {
                    LineSide::New => add.or(del),
                    LineSide::Old => del.or(add),
                }
            }
            AnnotatedLine::ExpandedContext { gap_id, line_idx } => self
                .get_expanded_line(gap_id, *line_idx)
                .map(|l| l.content.as_str()),
            _ => None,
        }
    }

    /// For annotations rendered outside the content gutter (hunk headers,
    /// file headers): returns a clean copy text. The selection's char range
    /// is meaningless for these — they're emitted whole or not at all.
    fn atomic_text_for_annotation(&self, ann_idx: usize) -> Option<String> {
        match self.line_annotations.get(ann_idx)? {
            AnnotatedLine::HunkHeader { file_idx, hunk_idx } => {
                let hunk = self.diff_files.get(*file_idx)?.hunks.get(*hunk_idx)?;
                Some(hunk.header.clone())
            }
            AnnotatedLine::FileHeader { file_idx } => {
                let file = self.diff_files.get(*file_idx)?;
                if file.is_commit_message {
                    Some(file.display_path().display().to_string())
                } else {
                    Some(format!(
                        "{} [{}]",
                        file.display_path().display(),
                        file.status.as_char()
                    ))
                }
            }
            _ => None,
        }
    }

    pub fn copy_visual_selection(&mut self) -> Result<usize> {
        let Some(sel) = self.visual_selection else {
            return Ok(0);
        };
        let (start, end) = sel.ordered();
        let side = sel.anchor.side;
        let mut out = String::new();
        let mut emitted = 0usize;
        for idx in start.annotation_idx..=end.annotation_idx {
            let snippet = if let Some(content) = self.content_for_side(idx, side) {
                let total = content.chars().count();
                let (lo, hi) = sel.char_range(idx, total);
                char_slice(content, lo, Some(hi)).to_string()
            } else if let Some(text) = self.atomic_text_for_annotation(idx) {
                text
            } else {
                continue;
            };
            if emitted > 0 {
                out.push('\n');
            }
            out.push_str(&snippet);
            emitted += 1;
        }
        if out.is_empty() {
            return Ok(0);
        }
        let count = out.chars().count();
        crate::output::copy_text_to_clipboard(&out)
            .map_err(|e| TuicrError::Clipboard(format!("{e}")))?;
        Ok(count)
    }

    pub fn lineno_width(&self) -> usize {
        let hunk_max = self
            .diff_files
            .iter()
            .map(|f| f.max_lineno())
            .max()
            .unwrap_or(0);
        let cache_max = self
            .file_line_count_cache
            .values()
            .copied()
            .max()
            .unwrap_or(0);
        lineno_width(hunk_max.max(cache_max))
    }

    pub fn pane_geometry(&self, inner: ratatui::layout::Rect, side: LineSide) -> PaneGeom {
        let w = self.lineno_width();
        match self.diff_view_mode {
            DiffViewMode::Unified => {
                let gutter = unified_gutter(w);
                let content_width = (inner.width as usize).saturating_sub(gutter as usize);
                PaneGeom {
                    content_x_start: inner.x + gutter,
                    content_x_end: inner.x + inner.width,
                    content_width,
                }
            }
            DiffViewMode::SideBySide => {
                let overhead = sbs_overhead(w);
                let left_gutter = sbs_left_gutter(w);
                let half_w = (inner.width.saturating_sub(overhead) / 2) as usize;
                match side {
                    LineSide::Old => PaneGeom {
                        content_x_start: inner.x + left_gutter,
                        content_x_end: inner.x + left_gutter + half_w as u16,
                        content_width: half_w,
                    },
                    LineSide::New => {
                        let start = inner.x + overhead + half_w as u16;
                        PaneGeom {
                            content_x_start: start,
                            content_x_end: start + half_w as u16,
                            content_width: half_w,
                        }
                    }
                }
            }
        }
    }

    pub fn side_at_x(
        &self,
        inner: ratatui::layout::Rect,
        x: u16,
        ann_default: LineSide,
    ) -> LineSide {
        let w = self.lineno_width();
        match self.diff_view_mode {
            DiffViewMode::Unified => ann_default,
            DiffViewMode::SideBySide => {
                let half_w = inner.width.saturating_sub(sbs_overhead(w)) / 2;
                let divider = inner.x + sbs_left_gutter(w) + half_w;
                if x < divider {
                    LineSide::Old
                } else {
                    LineSide::New
                }
            }
        }
    }

    pub fn cell_to_sel_point(&self, screen_col: u16, screen_row: u16) -> Option<SelPoint> {
        let idx = self.diff_annotation_at_screen_row(screen_row)?;
        let inner = self.diff_inner_area?;
        let ann = self.line_annotations.get(idx)?;
        let side = self.side_at_x(inner, screen_col, annotation_side_default(ann));

        let zero_point = SelPoint {
            annotation_idx: idx,
            char_offset: 0,
            side,
        };
        let Some(content) = self.content_for_side(idx, side) else {
            return Some(zero_point);
        };
        let geom = self.pane_geometry(inner, side);
        if geom.content_width == 0 {
            return Some(zero_point);
        }
        let last_col = geom.content_x_end.saturating_sub(1);
        let col = screen_col.clamp(geom.content_x_start, last_col);
        let col_in_row = (col - geom.content_x_start) as usize;

        let rel = (screen_row - inner.y) as usize;
        let mut walker = rel;
        while walker > 0 && self.diff_row_to_annotation.get(walker - 1).copied() == Some(idx) {
            walker -= 1;
        }
        let which_row = rel - walker;
        let total_chars = content.chars().count();
        let char_offset = (which_row * geom.content_width + col_in_row).min(total_chars);
        Some(SelPoint {
            annotation_idx: idx,
            char_offset,
            side,
        })
    }

    /// Mirrors `ensure_cursor_visible`'s notion of visibility (uses the
    /// renderer's `visible_line_count` when present so wrapping is honored).
    pub fn is_cursor_visible(&self) -> bool {
        let visible = if self.diff_state.visible_line_count > 0 {
            self.diff_state.visible_line_count
        } else {
            self.diff_state.viewport_height.max(1)
        };
        let cursor = self.diff_state.cursor_line;
        cursor >= self.diff_state.scroll_offset && cursor < self.diff_state.scroll_offset + visible
    }

    pub fn jump_to_file(&mut self, idx: usize) {
        use std::path::Path;

        if idx < self.diff_files.len() {
            // Deliberate jump cancels any in-flight two-press walk arming.
            self.primed_walk_next = false;
            self.primed_walk_prev = false;
            self.down_released_since_arm = false;
            self.up_released_since_arm = false;
            self.diff_state.current_file_idx = idx;
            self.diff_state.cursor_line = self.calculate_file_scroll_offset(idx);
            let max_scroll = self.max_scroll_offset();
            self.diff_state.scroll_offset = self.diff_state.cursor_line.min(max_scroll);

            let file_path = self.diff_files[idx].display_path().clone();
            let mut current = file_path.parent();
            while let Some(parent) = current {
                if parent != Path::new("") {
                    self.expanded_dirs
                        .insert(parent.to_string_lossy().to_string());
                }
                current = parent.parent();
            }

            if let Some(tree_idx) = self.file_idx_to_tree_idx(idx) {
                self.file_list_state.select(tree_idx);
            }

            // Single-file view filters `line_annotations` by
            // `current_file_idx`, so a file switch must rebuild them or
            // click hit-testing would resolve against the previous file.
            if self.is_single_file_view {
                self.rebuild_annotations();
            }
        }
    }

    pub fn jump_to_bottom(&mut self) {
        let max_line = self.max_cursor_line();
        self.diff_state.cursor_line = max_line;
        // Position so the last navigable line is at the bottom of the viewport
        let viewport = self.diff_state.viewport_height.max(1);
        self.diff_state.scroll_offset = (max_line + 1).saturating_sub(viewport);
        self.update_current_file_from_cursor();
    }

    pub fn next_file(&mut self) {
        let visible_items = self.build_visible_items();
        let current_file_idx = self.diff_state.current_file_idx;

        for item in &visible_items {
            if let FileTreeItem::File { file_idx, .. } = item
                && *file_idx > current_file_idx
            {
                self.jump_to_file(*file_idx);
                return;
            }
        }
    }

    pub fn prev_file(&mut self) {
        let visible_items = self.build_visible_items();
        let current_file_idx = self.diff_state.current_file_idx;

        for item in visible_items.iter().rev() {
            if let FileTreeItem::File { file_idx, .. } = item
                && *file_idx < current_file_idx
            {
                self.jump_to_file(*file_idx);
                return;
            }
        }
    }

    fn file_idx_to_tree_idx(&self, target_file_idx: usize) -> Option<usize> {
        let visible_items = self.build_visible_items();
        for (tree_idx, item) in visible_items.iter().enumerate() {
            if let FileTreeItem::File { file_idx, .. } = item
                && *file_idx == target_file_idx
            {
                return Some(tree_idx);
            }
        }
        None
    }

    /// Render-line indices of every visible hunk header. Respects
    /// single-file view (only the current file's hunks) and the
    /// reviewed-collapse behavior in multi-file view (skipped entirely)
    /// versus single-file view (body rendered under a banner).
    fn hunk_positions(&self) -> Vec<usize> {
        let single = self.is_single_file_view;
        let current_idx = self.diff_state.current_file_idx;
        let mut positions = Vec::new();
        let mut cumulative = self.review_comments_render_height();
        for (file_idx, file) in self.diff_files.iter().enumerate() {
            if single && file_idx != current_idx {
                continue;
            }
            let path = file.display_path();
            let is_reviewed = self.session.is_file_reviewed(path);

            if !single {
                cumulative += 1; // File header
            }
            if !single && is_reviewed {
                // multi-file collapsed: no body, no trailing spacing
                continue;
            }
            if single && is_reviewed {
                cumulative += 1; // banner
            }
            if let Some(review) = self.session.files.get(path) {
                cumulative += review.file_comments.len();
            }
            if file.is_binary || file.hunks.is_empty() {
                cumulative += 1;
            } else {
                for (hunk_idx, hunk) in file.hunks.iter().enumerate() {
                    positions.push(cumulative);
                    cumulative += 1;
                    if !self.is_hunk_reviewed(file_idx, hunk_idx) {
                        cumulative += hunk.lines.len();
                    }
                }
            }
            cumulative += 1; // trailing spacing or "next file" hint
        }
        positions
    }

    pub fn next_hunk(&mut self) {
        // Hunk navigation is a deliberate move, not a continuation of a
        // boundary walk. Clear any in-flight cursor-walk arming.
        self.primed_walk_next = false;
        self.primed_walk_prev = false;
        self.down_released_since_arm = false;
        self.up_released_since_arm = false;
        for pos in self.hunk_positions() {
            if pos > self.diff_state.cursor_line {
                self.diff_state.cursor_line = pos;
                self.ensure_cursor_visible();
                self.update_current_file_from_cursor();
                return;
            }
        }
        // No further hunks in the current frame. Single-file view crosses
        // into the next file's first hunk so `]` can step the codebase
        // hunk-by-hunk without breaking on file boundaries.
        if self.is_single_file_view {
            let next_idx = self.diff_state.current_file_idx + 1;
            if next_idx < self.diff_files.len() {
                self.jump_to_file(next_idx);
                if let Some(&first) = self.hunk_positions().first() {
                    self.diff_state.cursor_line = first;
                    self.ensure_cursor_visible();
                    self.update_current_file_from_cursor();
                }
            }
        }
    }

    pub fn prev_hunk(&mut self) {
        self.primed_walk_next = false;
        self.primed_walk_prev = false;
        self.down_released_since_arm = false;
        self.up_released_since_arm = false;
        let positions = self.hunk_positions();
        for &pos in positions.iter().rev() {
            if pos < self.diff_state.cursor_line {
                self.diff_state.cursor_line = pos;
                self.ensure_cursor_visible();
                self.update_current_file_from_cursor();
                return;
            }
        }
        // Symmetric to next_hunk: in single-file view, fall through to the
        // previous file's last hunk so `[` keeps stepping backward across
        // files.
        if self.is_single_file_view && self.diff_state.current_file_idx > 0 {
            let prev_idx = self.diff_state.current_file_idx - 1;
            self.jump_to_file(prev_idx);
            if let Some(&last) = self.hunk_positions().last() {
                self.diff_state.cursor_line = last;
                self.ensure_cursor_visible();
                self.update_current_file_from_cursor();
                return;
            }
        }
        self.diff_state.cursor_line = 0;
        self.ensure_cursor_visible();
        self.update_current_file_from_cursor();
    }

    fn calculate_file_scroll_offset(&self, file_idx: usize) -> usize {
        let mut offset = self.review_comments_render_height();
        for (i, file) in self.diff_files.iter().enumerate() {
            if i == file_idx {
                break;
            }
            offset += self.effective_file_height(i, file);
        }
        offset
    }

    fn review_comments_render_height(&self) -> usize {
        // Header line is only rendered in multi-file view. See the guards
        // in `src/ui/diff_unified.rs` and `src/ui/diff_side_by_side.rs`.
        let mut height = if self.is_single_file_view { 0 } else { 1 };
        for summary in &self.forge_review_summaries {
            height += crate::forge::remote_comments::summary_display_lines(summary);
        }
        for comment in &self.session.review_comments {
            height += Self::comment_display_lines(comment, self.diff_state.viewport_width);
        }
        // Review-level remote threads (line: None) — must mirror the filter
        // in `rebuild_annotations` or scroll offsets fall out of sync.
        {
            use crate::forge::remote_comments::{PrCommentsVisibility, thread_display_lines};
            let visibility = self.session.remote_comments_visibility;
            if !matches!(visibility, PrCommentsVisibility::Hide) {
                for thread in &self.forge_review_threads {
                    if thread.line.is_some() {
                        continue;
                    }
                    if visibility.render_decision(thread).is_none() {
                        continue;
                    }
                    height += thread_display_lines(thread);
                }
            }
        }
        if self.input_mode == InputMode::Comment
            && self.comment_is_review_level
            && self.editing_comment_id.is_none()
        {
            // Header + one content line + footer
            height += 3;
        }
        height
    }

    fn file_render_height(&self, file_idx: usize, file: &DiffFile) -> usize {
        if self.session.is_file_reviewed(file.display_path()) {
            return 1; // collapsed: header only
        }
        1 + self.file_render_body_height(file_idx, file) // header + body
    }

    /// File body height in lines (comments + content + trailing spacing),
    /// excluding the file header and ignoring the reviewed-collapse
    /// short-circuit. Used by `effective_file_height` to size the body of
    /// the focused file in single-file view, where the header is hidden
    /// and reviewed files render the body under a banner.
    fn file_render_body_height(&self, file_idx: usize, file: &DiffFile) -> usize {
        let path = file.display_path();

        let spacing_lines = 1; // Trailing blank or "next file" hint
        let mut content_lines = 0;
        let mut comment_lines = 0;

        // Pre-aggregate remote-thread rows by (line, side) for this file. Must
        // mirror the filter/anchor logic in build_remote_thread_index — the
        // rebuild_annotations renderer uses the same filter, and the two must
        // emit identical row counts or scroll math goes out of sync.
        let remote_thread_rows: HashMap<(u32, LineSide), usize> = {
            use crate::forge::remote_comments::{RemoteCommentSide, thread_display_lines};
            let mut map: HashMap<(u32, LineSide), usize> = HashMap::new();
            let path_str = path.to_string_lossy();
            let visibility = self.session.remote_comments_visibility;
            for thread in &self.forge_review_threads {
                if thread.path != *path_str {
                    continue;
                }
                if visibility.render_decision(thread).is_none() {
                    continue;
                }
                let Some(line) = thread.line else { continue };
                let side = match thread.side {
                    RemoteCommentSide::Right => LineSide::New,
                    RemoteCommentSide::Left => LineSide::Old,
                };
                *map.entry((line, side)).or_default() += thread_display_lines(thread);
            }
            map
        };

        if let Some(review) = self.session.files.get(path) {
            for comment in &review.file_comments {
                comment_lines +=
                    Self::comment_display_lines(comment, self.diff_state.viewport_width);
            }
        }

        if file.is_binary || file.hunks.is_empty() {
            content_lines = 1;
        } else {
            let line_comments = self.session.files.get(path).map(|r| &r.line_comments);

            for (hunk_idx, hunk) in file.hunks.iter().enumerate() {
                // Calculate gap before this hunk
                let prev_hunk = if hunk_idx > 0 {
                    file.hunks.get(hunk_idx - 1)
                } else {
                    None
                };
                let gap = calculate_gap(
                    prev_hunk.map(|h| (&h.new_start, &h.new_count)),
                    hunk.new_start,
                );

                let gap_id = GapId { file_idx, hunk_idx };

                if gap > 0 && self.should_render_gap_before_hunk(file_idx, hunk_idx) {
                    let top_len = self.expanded_top.get(&gap_id).map_or(0, |v| v.len());
                    let bot_len = self.expanded_bottom.get(&gap_id).map_or(0, |v| v.len());
                    let remaining = (gap as usize).saturating_sub(top_len + bot_len);
                    content_lines += top_len + bot_len;
                    content_lines += gap_annotation_line_count(hunk_idx == 0, false, remaining);
                }

                // Hunk header + diff lines
                content_lines += 1; // Hunk header
                if self.is_hunk_reviewed(file_idx, hunk_idx) {
                    continue;
                }

                // Count diff lines based on view mode
                match self.diff_view_mode {
                    DiffViewMode::Unified => {
                        for diff_line in &hunk.lines {
                            content_lines += 1;

                            if let Some(line_comments) = line_comments {
                                if let Some(old_ln) = diff_line.old_lineno
                                    && let Some(comments) = line_comments.get(&old_ln)
                                {
                                    for comment in comments {
                                        if comment.side == Some(LineSide::Old) {
                                            comment_lines += Self::comment_display_lines(
                                                comment,
                                                self.diff_state.viewport_width,
                                            );
                                        }
                                    }
                                }

                                if let Some(new_ln) = diff_line.new_lineno
                                    && let Some(comments) = line_comments.get(&new_ln)
                                {
                                    for comment in comments {
                                        if comment.side != Some(LineSide::Old) {
                                            comment_lines += Self::comment_display_lines(
                                                comment,
                                                self.diff_state.viewport_width,
                                            );
                                        }
                                    }
                                }
                            }

                            if let Some(old_ln) = diff_line.old_lineno {
                                comment_lines += remote_thread_rows
                                    .get(&(old_ln, LineSide::Old))
                                    .copied()
                                    .unwrap_or(0);
                            }
                            if let Some(new_ln) = diff_line.new_lineno {
                                comment_lines += remote_thread_rows
                                    .get(&(new_ln, LineSide::New))
                                    .copied()
                                    .unwrap_or(0);
                            }
                        }
                    }
                    DiffViewMode::SideBySide => {
                        use crate::model::LineOrigin;
                        // Side-by-side mode: pair deletions with following additions
                        let lines = &hunk.lines;
                        let mut i = 0;
                        while i < lines.len() {
                            let diff_line = &lines[i];

                            match diff_line.origin {
                                LineOrigin::Context => {
                                    content_lines += 1;

                                    // Comments for context line
                                    if let Some(line_comments) = line_comments
                                        && let Some(new_ln) = diff_line.new_lineno
                                        && let Some(comments) = line_comments.get(&new_ln)
                                    {
                                        for comment in comments {
                                            if comment.side != Some(LineSide::Old) {
                                                comment_lines += Self::comment_display_lines(
                                                    comment,
                                                    self.diff_state.viewport_width,
                                                );
                                            }
                                        }
                                    }
                                    if let Some(new_ln) = diff_line.new_lineno {
                                        comment_lines += remote_thread_rows
                                            .get(&(new_ln, LineSide::New))
                                            .copied()
                                            .unwrap_or(0);
                                    }
                                    i += 1;
                                }
                                LineOrigin::Deletion => {
                                    // Find consecutive deletions
                                    let del_start = i;
                                    let mut del_end = i + 1;
                                    while del_end < lines.len()
                                        && lines[del_end].origin == LineOrigin::Deletion
                                    {
                                        del_end += 1;
                                    }

                                    // Find consecutive additions following deletions
                                    let add_start = del_end;
                                    let mut add_end = add_start;
                                    while add_end < lines.len()
                                        && lines[add_end].origin == LineOrigin::Addition
                                    {
                                        add_end += 1;
                                    }

                                    let del_count = del_end - del_start;
                                    let add_count = add_end - add_start;
                                    // Paired lines use max of the two counts
                                    content_lines += del_count.max(add_count);

                                    // Count comments for all deletions and additions in this pair
                                    if let Some(line_comments) = line_comments {
                                        for line in &lines[del_start..del_end] {
                                            if let Some(old_ln) = line.old_lineno
                                                && let Some(comments) = line_comments.get(&old_ln)
                                            {
                                                for comment in comments {
                                                    if comment.side == Some(LineSide::Old) {
                                                        comment_lines +=
                                                            Self::comment_display_lines(
                                                                comment,
                                                                self.diff_state.viewport_width,
                                                            );
                                                    }
                                                }
                                            }
                                        }

                                        for line in &lines[add_start..add_end] {
                                            if let Some(new_ln) = line.new_lineno
                                                && let Some(comments) = line_comments.get(&new_ln)
                                            {
                                                for comment in comments {
                                                    if comment.side != Some(LineSide::Old) {
                                                        comment_lines +=
                                                            Self::comment_display_lines(
                                                                comment,
                                                                self.diff_state.viewport_width,
                                                            );
                                                    }
                                                }
                                            }
                                        }
                                    }

                                    for line in &lines[del_start..del_end] {
                                        if let Some(old_ln) = line.old_lineno {
                                            comment_lines += remote_thread_rows
                                                .get(&(old_ln, LineSide::Old))
                                                .copied()
                                                .unwrap_or(0);
                                        }
                                    }
                                    for line in &lines[add_start..add_end] {
                                        if let Some(new_ln) = line.new_lineno {
                                            comment_lines += remote_thread_rows
                                                .get(&(new_ln, LineSide::New))
                                                .copied()
                                                .unwrap_or(0);
                                        }
                                    }

                                    i = add_end;
                                }
                                LineOrigin::Addition => {
                                    // Standalone addition (not following deletions)
                                    content_lines += 1;

                                    if let Some(line_comments) = line_comments
                                        && let Some(new_ln) = diff_line.new_lineno
                                        && let Some(comments) = line_comments.get(&new_ln)
                                    {
                                        for comment in comments {
                                            if comment.side != Some(LineSide::Old) {
                                                comment_lines += Self::comment_display_lines(
                                                    comment,
                                                    self.diff_state.viewport_width,
                                                );
                                            }
                                        }
                                    }
                                    if let Some(new_ln) = diff_line.new_lineno {
                                        comment_lines += remote_thread_rows
                                            .get(&(new_ln, LineSide::New))
                                            .copied()
                                            .unwrap_or(0);
                                    }

                                    i += 1;
                                }
                            }
                        }
                    }
                }
            }

            // End-of-file gap (not for deleted files)
            if file.status != FileStatus::Deleted
                && let Some(last_hunk) = file.hunks.last()
            {
                let eof_start = last_hunk.new_start + last_hunk.new_count;
                if let Some(&total) = self.file_line_count_cache.get(&file_idx)
                    && eof_start <= total
                {
                    let gap = (total - eof_start + 1) as usize;
                    let eof_gap_id = GapId {
                        file_idx,
                        hunk_idx: file.hunks.len(),
                    };
                    let top_len = self.expanded_top.get(&eof_gap_id).map_or(0, |v| v.len());
                    let bot_len = self.expanded_bottom.get(&eof_gap_id).map_or(0, |v| v.len());
                    let remaining = gap.saturating_sub(top_len + bot_len);
                    content_lines += top_len + bot_len;
                    content_lines += gap_annotation_line_count(false, true, remaining);
                }
            }
        }

        comment_lines + content_lines + spacing_lines
    }

    /// Render-aware file height that knows about `is_single_file_view`.
    /// Multi-file view: same as `file_render_height`. Single-file view:
    /// non-current files return 0; the current file returns the body
    /// (no header) plus a 1-line banner when the file is reviewed.
    fn effective_file_height(&self, file_idx: usize, file: &DiffFile) -> usize {
        if !self.is_single_file_view {
            return self.file_render_height(file_idx, file);
        }
        if file_idx != self.diff_state.current_file_idx {
            return 0;
        }
        let banner = if self.session.is_file_reviewed(file.display_path()) {
            1
        } else {
            0
        };
        banner + self.file_render_body_height(file_idx, file)
    }

    fn update_current_file_from_cursor(&mut self) {
        // Single-file view renders one file at a time. `cursor_line` is
        // interpreted relative to that single file's content, so mapping
        // it back through cumulative-file-heights would wrongly resolve
        // every position to file 0. The "which file is visible" decision
        // is owned by `jump_to_file` / toggle / file-list-follow.
        if self.is_single_file_view {
            return;
        }
        let mut cumulative = self.review_comments_render_height();
        if self.diff_state.cursor_line < cumulative {
            if !self.diff_files.is_empty() {
                self.diff_state.current_file_idx = 0;
                self.file_list_state.select(0);
            }
            return;
        }
        for (i, file) in self.diff_files.iter().enumerate() {
            let height = self.file_render_height(i, file);
            if cumulative + height > self.diff_state.cursor_line {
                self.diff_state.current_file_idx = i;
                self.file_list_state.select(i);
                return;
            }
            cumulative += height;
        }
        if !self.diff_files.is_empty() {
            self.diff_state.current_file_idx = self.diff_files.len() - 1;
            self.file_list_state.select(self.diff_files.len() - 1);
        }
    }

    pub fn total_lines(&self) -> usize {
        self.review_comments_render_height()
            + self
                .diff_files
                .iter()
                .enumerate()
                .map(|(i, f)| self.effective_file_height(i, f))
                .sum::<usize>()
    }

    /// Last line the cursor can occupy. If the final annotation is a Spacing
    /// separator it is not navigable content and is excluded.
    pub fn max_cursor_line(&self) -> usize {
        let total = self.total_lines();
        if matches!(self.line_annotations.last(), Some(AnnotatedLine::Spacing)) {
            total.saturating_sub(2)
        } else {
            total.saturating_sub(1)
        }
    }

    /// Calculate the maximum scroll offset.
    ///
    /// Allows scrolling until the last line of content is at the top of the viewport.
    /// This permits empty space below content (e.g. when centering the cursor near EOF)
    /// while ensuring there is always at least one line of content visible at the top.
    pub fn max_scroll_offset(&self) -> usize {
        self.total_lines().saturating_sub(1)
    }

    /// Calculate the number of display lines a comment takes (header + content + footer).
    /// Uses viewport_width to account for pre-wrapped visual segments so the
    /// annotation count stays in sync with what format_comment_lines renders.
    pub(crate) fn comment_display_lines(comment: &Comment, viewport_width: usize) -> usize {
        // Mirrors the content_area calculation in format_comment_lines:
        // indicator(1) + border_prefix(7) + safety_margin(2) = 10
        let content_area = viewport_width.saturating_sub(10);
        let visual_lines: usize = comment
            .content
            .split('\n')
            .map(|line| crate::ui::comment_panel::wrap_segments(line, content_area).len())
            .sum();
        2 + visual_lines // top border + visual segments + bottom border
    }

    /// Update viewport_width and trigger annotation rebuild if it changed.
    /// Called from render functions when inner.width is computed — keeps
    /// line_annotations.len() in sync with the rendered Vec<Line>.
    pub fn sync_viewport_width(&mut self, new_width: usize) {
        if self.diff_state.viewport_width != new_width {
            self.diff_state.viewport_width = new_width;
            self.rebuild_annotations();
        }
    }

    /// Returns the source line number and side at the current cursor position, if on a diff line
    pub fn get_line_at_cursor(&self) -> Option<(u32, LineSide)> {
        let target = self.diff_state.cursor_line;
        match self.line_annotations.get(target) {
            Some(AnnotatedLine::DiffLine {
                old_lineno,
                new_lineno,
                ..
            })
            | Some(AnnotatedLine::SideBySideLine {
                old_lineno,
                new_lineno,
                ..
            }) => {
                // Prefer new line number (for added/context lines), fall back to old (for deleted)
                new_lineno
                    .map(|ln| (ln, LineSide::New))
                    .or_else(|| old_lineno.map(|ln| (ln, LineSide::Old)))
            }
            _ => None,
        }
    }

    /// True when the cursor sits on a local comment whose lifecycle state
    /// has been pushed/submitted to the forge. Such comments are locked from
    /// edit/delete in tuicr to prevent the local state from drifting from
    /// what GitHub now stores.
    pub fn cursor_on_locked_comment(&self) -> bool {
        let Some(location) = self.find_comment_at_cursor() else {
            return false;
        };
        match location {
            CommentLocation::Review { index } => self
                .session
                .review_comments
                .get(index)
                .is_some_and(|c| c.is_locked()),
            CommentLocation::File { path, index } => self
                .session
                .files
                .get(&path)
                .and_then(|review| review.file_comments.get(index))
                .is_some_and(|c| c.is_locked()),
            CommentLocation::Line {
                path,
                line,
                side,
                index,
            } => self
                .session
                .files
                .get(&path)
                .and_then(|review| review.line_comments.get(&line))
                .and_then(|comments| {
                    let mut side_idx = 0;
                    for c in comments {
                        if c.side.unwrap_or(LineSide::New) == side {
                            if side_idx == index {
                                return Some(c);
                            }
                            side_idx += 1;
                        }
                    }
                    None
                })
                .is_some_and(|c| c.is_locked()),
        }
    }

    /// Find the comment at the current cursor position
    /// True when the cursor is on a row that belongs to a fetched-from-GitHub
    /// review thread. Remote threads are read-only in v1; surfaced as a
    /// distinct condition so the handler can produce a clearer message than
    /// the generic "no comment at cursor".
    pub fn cursor_on_remote_thread(&self) -> bool {
        matches!(
            self.line_annotations.get(self.diff_state.cursor_line),
            Some(AnnotatedLine::RemoteThreadLine { .. })
        )
    }

    fn find_comment_at_cursor(&self) -> Option<CommentLocation> {
        let target = self.diff_state.cursor_line;
        match self.line_annotations.get(target) {
            Some(AnnotatedLine::ReviewComment { comment_idx }) => Some(CommentLocation::Review {
                index: *comment_idx,
            }),
            Some(AnnotatedLine::FileComment {
                file_idx,
                comment_idx,
            }) => {
                let path = self.diff_files.get(*file_idx)?.display_path().clone();
                Some(CommentLocation::File {
                    path,
                    index: *comment_idx,
                })
            }
            Some(AnnotatedLine::LineComment {
                file_idx,
                line,
                side,
                comment_idx,
            }) => {
                let path = self.diff_files.get(*file_idx)?.display_path().clone();
                Some(CommentLocation::Line {
                    path,
                    line: *line,
                    side: *side,
                    index: *comment_idx,
                })
            }
            _ => None,
        }
    }

    /// Delete the comment at the current cursor position, if any
    /// Returns true if a comment was deleted
    pub fn delete_comment_at_cursor(&mut self) -> bool {
        let location = self.find_comment_at_cursor();

        match location {
            Some(CommentLocation::Review { index })
                if index < self.session.review_comments.len() =>
            {
                self.session.review_comments.remove(index);
                self.dirty = true;
                self.set_message("Review comment deleted");
                self.rebuild_annotations();
                return true;
            }
            Some(CommentLocation::File { path, index }) => {
                if let Some(review) = self.session.get_file_mut(&path) {
                    review.file_comments.remove(index);
                    self.dirty = true;
                    self.set_message("Comment deleted");
                    self.rebuild_annotations();
                    return true;
                }
            }
            Some(CommentLocation::Line {
                path,
                line,
                side,
                index,
            }) => {
                if let Some(review) = self.session.get_file_mut(&path)
                    && let Some(comments) = review.line_comments.get_mut(&line)
                {
                    // Find the actual index by counting comments with matching side
                    let mut side_idx = 0;
                    let mut actual_idx = None;
                    for (i, comment) in comments.iter().enumerate() {
                        let comment_side = comment.side.unwrap_or(LineSide::New);
                        if comment_side == side {
                            if side_idx == index {
                                actual_idx = Some(i);
                                break;
                            }
                            side_idx += 1;
                        }
                    }
                    if let Some(idx) = actual_idx {
                        comments.remove(idx);
                        if comments.is_empty() {
                            review.line_comments.remove(&line);
                        }
                        self.dirty = true;
                        self.set_message(format!("Comment on line {line} deleted"));
                        self.rebuild_annotations();
                        return true;
                    }
                }
            }
            Some(CommentLocation::Review { .. }) | None => {}
        }

        false
    }

    pub fn clear_comments(&mut self, scope: ClearScope) {
        let (cleared, unreviewed) = self.session.clear_comments(scope);
        if cleared == 0 && unreviewed == 0 {
            self.set_message("No comments to clear");
            return;
        }

        self.dirty = true;
        self.rebuild_annotations();
        let msg = match (cleared, unreviewed) {
            (0, n) => format!("Unreviewed {n} files"),
            (c, 0) => format!("Cleared {c} comments"),
            (c, n) => format!("Cleared {c} comments, unreviewed {n} files"),
        };
        self.set_message(msg);
    }

    /// Enter edit mode for the comment at the current cursor position
    /// Returns true if a comment was found and edit mode entered
    pub fn enter_edit_mode(&mut self) -> bool {
        let location = self.find_comment_at_cursor();

        match location {
            Some(CommentLocation::Review { index }) => {
                if let Some(comment) = self.session.review_comments.get(index) {
                    self.input_mode = InputMode::Comment;
                    self.diff_state.scroll_x = 0;
                    self.comment_buffer = comment.content.clone();
                    self.comment_cursor = self.comment_buffer.len();
                    self.comment_type = comment.comment_type.clone();
                    self.comment_is_review_level = true;
                    self.comment_is_file_level = false;
                    self.comment_line = None;
                    self.editing_comment_id = Some(comment.id.clone());
                    return true;
                }
            }
            Some(CommentLocation::File { path, index }) => {
                if let Some(review) = self.session.files.get(&path)
                    && let Some(comment) = review.file_comments.get(index)
                {
                    self.input_mode = InputMode::Comment;
                    self.diff_state.scroll_x = 0;
                    self.comment_buffer = comment.content.clone();
                    self.comment_cursor = self.comment_buffer.len();
                    self.comment_type = comment.comment_type.clone();
                    self.comment_is_review_level = false;
                    self.comment_is_file_level = true;
                    self.comment_line = None;
                    self.editing_comment_id = Some(comment.id.clone());
                    return true;
                }
            }
            Some(CommentLocation::Line {
                path,
                line,
                side,
                index,
            }) => {
                if let Some(review) = self.session.files.get(&path)
                    && let Some(comments) = review.line_comments.get(&line)
                {
                    // Find the actual comment by counting comments with matching side
                    let mut side_idx = 0;
                    for comment in comments.iter() {
                        let comment_side = comment.side.unwrap_or(LineSide::New);
                        if comment_side == side {
                            if side_idx == index {
                                self.input_mode = InputMode::Comment;
                                self.diff_state.scroll_x = 0;
                                self.comment_buffer = comment.content.clone();
                                self.comment_cursor = self.comment_buffer.len();
                                self.comment_type = comment.comment_type.clone();
                                self.comment_is_review_level = false;
                                self.comment_is_file_level = false;
                                self.comment_line = Some((line, side));
                                self.editing_comment_id = Some(comment.id.clone());
                                return true;
                            }
                            side_idx += 1;
                        }
                    }
                }
            }
            None => {}
        }

        false
    }

    pub fn enter_command_mode(&mut self) {
        self.input_mode = InputMode::Command;
        self.command_buffer.clear();
        self.command_completion = None;
    }

    pub fn exit_command_mode(&mut self) {
        self.input_mode = InputMode::Normal;
        self.command_buffer.clear();
        self.command_completion = None;
    }

    pub fn enter_search_mode(&mut self) {
        self.input_mode = InputMode::Search;
        self.search_buffer.clear();
    }

    pub fn exit_search_mode(&mut self) {
        self.input_mode = InputMode::Normal;
        self.search_buffer.clear();
    }

    pub fn enter_comment_mode(&mut self, file_level: bool, line: Option<(u32, LineSide)>) {
        self.input_mode = InputMode::Comment;
        // Snap horizontal scroll back to the left edge so the inline input
        // box renders inside the viewport on long lines.
        self.diff_state.scroll_x = 0;
        self.comment_buffer.clear();
        self.comment_cursor = 0;
        self.comment_type = self.default_comment_type();
        self.comment_is_review_level = false;
        self.comment_is_file_level = file_level;
        self.comment_line = line;
    }

    pub fn enter_review_comment_mode(&mut self) {
        self.input_mode = InputMode::Comment;
        self.diff_state.scroll_x = 0;
        self.comment_buffer.clear();
        self.comment_cursor = 0;
        self.comment_type = self.default_comment_type();
        self.comment_is_review_level = true;
        self.comment_is_file_level = false;
        self.comment_line = None;
        self.comment_line_range = None;
        self.editing_comment_id = None;
    }

    pub fn exit_comment_mode(&mut self) {
        self.input_mode = InputMode::Normal;
        self.comment_buffer.clear();
        self.comment_cursor = 0;
        self.comment_is_review_level = false;
        self.editing_comment_id = None;
        self.comment_line_range = None;
    }

    pub fn enter_visual_mode_at_cursor(&mut self) {
        let idx = self.diff_state.cursor_line;
        let side = self
            .get_line_at_cursor()
            .map(|(_, s)| s)
            .unwrap_or(LineSide::New);
        let len = self.annotation_content_len(idx, side);
        let anchor = SelPoint {
            annotation_idx: idx,
            char_offset: 0,
            side,
        };
        let head = SelPoint {
            annotation_idx: idx,
            char_offset: len,
            side,
        };
        self.input_mode = InputMode::VisualSelect;
        self.visual_selection = Some(VisualSelection { anchor, head });
    }

    pub fn exit_visual_mode(&mut self) {
        self.input_mode = InputMode::Normal;
        self.visual_selection = None;
    }

    pub fn get_visual_selection(&self) -> Option<&VisualSelection> {
        if self.input_mode != InputMode::VisualSelect {
            return None;
        }
        self.visual_selection.as_ref()
    }

    pub fn annotation_content_len(&self, idx: usize, side: LineSide) -> usize {
        self.content_for_side(idx, side)
            .map(|s| s.chars().count())
            .unwrap_or(0)
    }

    pub fn extend_visual_to_cursor(&mut self) {
        let Some(sel) = self.visual_selection else {
            return;
        };
        let anchor_idx = sel.anchor.annotation_idx;
        let cursor_idx = self.diff_state.cursor_line;
        let side = sel.anchor.side;
        let anchor_len = self.annotation_content_len(anchor_idx, side);
        let cursor_len = self.annotation_content_len(cursor_idx, side);
        let (anchor_char, head_char) = if cursor_idx >= anchor_idx {
            (0, cursor_len)
        } else {
            (anchor_len, 0)
        };
        self.visual_selection = Some(VisualSelection {
            anchor: SelPoint {
                annotation_idx: anchor_idx,
                char_offset: anchor_char,
                side,
            },
            head: SelPoint {
                annotation_idx: cursor_idx,
                char_offset: head_char,
                side,
            },
        });
    }

    pub fn visual_selection_line_range(&self) -> Option<(LineRange, LineSide)> {
        let sel = self.get_visual_selection()?;
        let (start, end) = sel.ordered();
        let start_line = self.annotation_line_for_side(start.annotation_idx, start.side);
        let end_line = self.annotation_line_for_side(end.annotation_idx, end.side);
        let start_ln = start_line?;
        let end_ln = end_line?;
        Some((LineRange::new(start_ln, end_ln), start.side))
    }

    fn annotation_line_for_side(&self, idx: usize, side: LineSide) -> Option<u32> {
        match self.line_annotations.get(idx)? {
            AnnotatedLine::DiffLine {
                old_lineno,
                new_lineno,
                ..
            }
            | AnnotatedLine::SideBySideLine {
                old_lineno,
                new_lineno,
                ..
            } => match side {
                LineSide::New => *new_lineno,
                LineSide::Old => *old_lineno,
            },
            _ => None,
        }
    }

    pub fn enter_comment_from_visual(&mut self) {
        if let Some((range, side)) = self.visual_selection_line_range() {
            self.comment_line_range = Some((range, side));
            self.comment_line = Some((range.end, side));
            self.input_mode = InputMode::Comment;
            self.diff_state.scroll_x = 0;
            self.comment_buffer.clear();
            self.comment_cursor = 0;
            self.comment_type = self.default_comment_type();
            self.comment_is_review_level = false;
            self.comment_is_file_level = false;
            self.visual_selection = None;
        } else {
            self.set_warning("Invalid visual selection");
            self.exit_visual_mode();
        }
    }

    pub fn save_comment(&mut self) {
        if self.comment_buffer.trim().is_empty() {
            self.set_message("Comment cannot be empty");
            return;
        }

        let content = self.comment_buffer.trim().to_string();

        let mut message = "Error: Could not save comment".to_string();
        let mut autosave_error = None;

        // Check if we're editing an existing comment
        if let Some(editing_id) = &self.editing_comment_id {
            if let Some(comment) = self
                .session
                .review_comments
                .iter_mut()
                .find(|c| &c.id == editing_id)
            {
                comment.content = content.clone();
                comment.comment_type = self.comment_type.clone();
                message = "Review comment updated".to_string();
            } else if let Some(path) = self.current_file_path().cloned()
                && let Some(review) = self.session.get_file_mut(&path)
            {
                if let Some(comment) = review
                    .file_comments
                    .iter_mut()
                    .find(|c| &c.id == editing_id)
                {
                    comment.content = content.clone();
                    comment.comment_type = self.comment_type.clone();
                    message = "Comment updated".to_string();
                } else {
                    // If not found in file comments, search in line comments
                    let mut found_comment = None;
                    for comments in review.line_comments.values_mut() {
                        if let Some(comment) = comments.iter_mut().find(|c| &c.id == editing_id) {
                            found_comment = Some(comment);
                            break;
                        }
                    }

                    if let Some(comment) = found_comment {
                        comment.content = content.clone();
                        comment.comment_type = self.comment_type.clone();
                        message = if let Some((line, _)) = self.comment_line {
                            format!("Comment on line {line} updated")
                        } else {
                            "Comment updated".to_string()
                        };
                    } else {
                        message = "Error: Comment to edit not found".to_string();
                    }
                }
            }
        } else if self.comment_is_review_level {
            let request = AddCommentRequest {
                target: CommentTarget::Review,
                content,
                comment_type: self.comment_type.clone(),
                author: self.username.clone(),
            };
            message = match add_comment_to_session(&mut self.session, request) {
                Ok(_) => "Review comment added".to_string(),
                Err(e) => format!("Error: Could not save comment: {e}"),
            };
        } else if let Some(path) = self.current_file_path().cloned() {
            let (target, success_message) = if self.comment_is_file_level {
                (
                    CommentTarget::File { path },
                    "File comment added".to_string(),
                )
            } else if let Some((range, side)) = self.comment_line_range {
                let message = if range.is_single() {
                    format!("Comment added to line {}", range.end)
                } else {
                    format!("Comment added to lines {}-{}", range.start, range.end)
                };
                (CommentTarget::LineRange { path, range, side }, message)
            } else if let Some((line, side)) = self.comment_line {
                (
                    CommentTarget::Line { path, line, side },
                    format!("Comment added to line {line}"),
                )
            } else {
                (
                    CommentTarget::File { path },
                    "File comment added".to_string(),
                )
            };

            let request = AddCommentRequest {
                target,
                content,
                comment_type: self.comment_type.clone(),
                author: self.username.clone(),
            };
            message = match add_comment_to_session(&mut self.session, request) {
                Ok(_) => success_message,
                Err(e) => format!("Error: Could not save comment: {e}"),
            };
        }

        if !message.starts_with("Error:") {
            self.dirty = true;
            if let Err(e) = self.save_current_session_merging_external() {
                autosave_error = Some(format!("{message}; autosave failed: {e}"));
            }
        }
        if let Some(error) = autosave_error {
            self.set_error(error);
        } else {
            self.set_message(message);
        }
        self.rebuild_annotations();

        self.exit_comment_mode();
    }

    pub fn cycle_comment_type(&mut self) {
        if self.comment_types.is_empty() {
            return;
        }
        if self.comment_types.len() == 1 {
            self.set_message("Only one comment type configured");
            return;
        }

        let current_id = self.comment_type.id().to_string();
        let current_index = self
            .comment_types
            .iter()
            .position(|comment_type| comment_type.id == current_id)
            .unwrap_or(0);
        let next_index = (current_index + 1) % self.comment_types.len();
        let next_id = self.comment_types[next_index].id.clone();
        self.comment_type = CommentType::from_id(&next_id);
        let label = self.comment_type_label(&self.comment_type.clone());
        self.set_message(format!("Comment type: {label}"));
    }

    pub fn cycle_comment_type_reverse(&mut self) {
        if self.comment_types.is_empty() {
            return;
        }
        if self.comment_types.len() == 1 {
            self.set_message("Only one comment type configured");
            return;
        }

        let current_id = self.comment_type.id();
        let current_index = self
            .comment_types
            .iter()
            .position(|comment_type| comment_type.id == current_id)
            .unwrap_or(0);
        let prev_index = if current_index == 0 {
            self.comment_types.len() - 1
        } else {
            current_index - 1
        };
        self.comment_type = CommentType::from_id(&self.comment_types[prev_index].id);
        let label = self.comment_type_label(&self.comment_type.clone());
        self.set_message(format!("Comment type: {label}"));
    }

    pub fn toggle_help(&mut self) {
        if self.input_mode == InputMode::Help {
            self.input_mode = InputMode::Normal;
        } else {
            self.input_mode = InputMode::Help;
            self.help_state.scroll_offset = 0;
        }
    }

    pub fn help_scroll_down(&mut self, lines: usize) {
        let max_offset = self
            .help_state
            .total_lines
            .saturating_sub(self.help_state.viewport_height);
        self.help_state.scroll_offset = (self.help_state.scroll_offset + lines).min(max_offset);
    }

    pub fn help_scroll_up(&mut self, lines: usize) {
        self.help_state.scroll_offset = self.help_state.scroll_offset.saturating_sub(lines);
    }

    pub fn help_scroll_to_top(&mut self) {
        self.help_state.scroll_offset = 0;
    }

    pub fn help_scroll_to_bottom(&mut self) {
        let max_offset = self
            .help_state
            .total_lines
            .saturating_sub(self.help_state.viewport_height);
        self.help_state.scroll_offset = max_offset;
    }

    pub fn enter_confirm_mode(&mut self, action: ConfirmAction) {
        self.input_mode = InputMode::Confirm;
        self.pending_confirm = Some(action);
    }

    pub fn exit_confirm_mode(&mut self) {
        self.input_mode = InputMode::Normal;
        self.pending_confirm = None;
    }

    /// Drive `:submit*` preflight: walk every local-draft comment in the
    /// current PR session, map each one against the displayed diff, bucket
    /// the results, and transition into the resolver (when there are
    /// unmappable comments) or the final-confirmation modal.
    ///
    /// PR 5 does not call the network; `[y]` in the confirmation modal
    /// stubs a "PR 6 will wire the network call" info message.
    pub fn start_submit(&mut self, event: crate::forge::submit::SubmitEvent) {
        self.start_submit_with(event, false);
    }

    /// Like `start_submit`, but when `skip_confirm` is `true` the flow
    /// bypasses `SubmitConfirm`. The action-picker path uses this because
    /// picking IS the confirmation; the resolver (if any unmappable
    /// comments) still runs first, then dispatches the network call
    /// directly. `:submit <event>` callers should pass `false`.
    pub fn start_submit_with(
        &mut self,
        event: crate::forge::submit::SubmitEvent,
        skip_confirm: bool,
    ) {
        use crate::forge::submit::{
            CommentAnchor, InlineComment, ResolverAction, UnmappableItem, map_comment,
        };

        let DiffSource::PullRequest(pr) = &self.diff_source else {
            self.set_warning(":submit only applies in PR mode");
            return;
        };
        if pr.is_read_only() {
            let reason = pr.read_only_reason().unwrap_or("read only");
            self.set_warning(format!("Cannot submit: PR is {reason}"));
            return;
        }
        // When the inline commit selector shows a strict subset, comments
        // anchor to the displayed (subset) diff, so `commit_id` must be the
        // SHA the diff was computed against — otherwise GitHub rejects with
        // 422 because the line/position isn't present in the diff against
        // the cumulative PR head. `pr_commits` is stored newest-first, so
        // the head of a (start_idx..=end_idx) range is `pr_commits[start_idx]`.
        let commit_id = match self.commit_selection_range {
            Some((start_idx, end_idx))
                if !self.pr_commits.is_empty()
                    && start_idx <= end_idx
                    && end_idx < self.pr_commits.len()
                    && !(start_idx == 0 && end_idx + 1 == self.pr_commits.len()) =>
            {
                self.pr_commits[start_idx].oid.clone()
            }
            _ => pr.key.head_sha.clone(),
        };

        // Source of truth for the diff: when the inline commit selector is
        // showing a strict subset, `range_diff_files` carries the merged
        // subset diff; otherwise `diff_files` is canonical.
        let files: Vec<&DiffFile> = match self.range_diff_files.as_ref() {
            Some(range) => range.iter().collect(),
            None => self.diff_files.iter().collect(),
        };

        let mut mappable: Vec<InlineComment> = Vec::new();
        let mut unmappable: Vec<UnmappableItem> = Vec::new();
        let mut total_local_drafts = 0_usize;

        // Walk file-level and line comments in display order. Review-level
        // comments (session.review_comments) are NOT inline-mapped; they
        // appear in the body via `build_review_body`.
        for file in &files {
            let Some(review) = self.session.files.get(file.display_path()) else {
                continue;
            };
            for comment in &review.file_comments {
                if comment.is_locked() {
                    continue;
                }
                total_local_drafts += 1;
                bucket_mapping(
                    map_comment(comment, CommentAnchor::FileLevel, file, &self.forge_config),
                    &mut mappable,
                    &mut unmappable,
                );
            }
            let mut keys: Vec<&u32> = review.line_comments.keys().collect();
            keys.sort();
            for key in keys {
                for comment in &review.line_comments[key] {
                    if comment.is_locked() {
                        continue;
                    }
                    total_local_drafts += 1;
                    let anchor = if comment.line_range.is_some() {
                        CommentAnchor::Range
                    } else {
                        CommentAnchor::Line {
                            line: *key,
                            side: comment.side.unwrap_or_default(),
                        }
                    };
                    bucket_mapping(
                        map_comment(comment, anchor, file, &self.forge_config),
                        &mut mappable,
                        &mut unmappable,
                    );
                }
            }
        }

        // Approve is the one event that's meaningful with no comments — a
        // bare "LGTM" approval. Every other event needs at least one local
        // draft comment or a review-level comment, otherwise there's
        // nothing to submit.
        let bare_allowed = matches!(event, crate::forge::submit::SubmitEvent::Approve);
        if !bare_allowed && total_local_drafts == 0 && self.session.review_comments.is_empty() {
            self.set_warning("Nothing to submit — no local-draft comments");
            return;
        }

        let resolver_choices = vec![ResolverAction::default(); unmappable.len()];
        let has_unmappable = !unmappable.is_empty();
        self.submit_state = Some(SubmitState {
            event,
            mappable,
            unmappable,
            resolver_choices,
            resolver_cursor: 0,
            commit_id,
            skip_confirm,
        });

        if has_unmappable {
            self.input_mode = InputMode::SubmitResolver;
        } else if skip_confirm {
            self.input_mode = InputMode::Normal;
            self.confirm_submit();
        } else {
            self.input_mode = InputMode::SubmitConfirm;
        }
    }

    /// Open the bare-`:submit` action picker. The user picks
    /// Comment/Approve/Request changes/Draft (or cancels); the picked event
    /// then runs through preflight with `skip_confirm = true` so no extra
    /// confirmation modal follows.
    pub fn start_submit_action_picker(&mut self) {
        if !matches!(self.diff_source, DiffSource::PullRequest(_)) {
            self.set_warning(":submit only applies in PR mode");
            return;
        }
        self.submit_picker_cursor = 0;
        self.input_mode = InputMode::SubmitActionPicker;
    }

    /// Move the action-picker cursor down by one row, wrapping at the end.
    pub fn submit_picker_cursor_down(&mut self) {
        let total = SUBMIT_PICKER_EVENTS.len();
        if total > 0 {
            self.submit_picker_cursor = (self.submit_picker_cursor + 1) % total;
        }
    }

    /// Move the action-picker cursor up by one row, wrapping at the start.
    pub fn submit_picker_cursor_up(&mut self) {
        let total = SUBMIT_PICKER_EVENTS.len();
        if total > 0 {
            self.submit_picker_cursor = (self.submit_picker_cursor + total - 1) % total;
        }
    }

    /// Confirm the action picker selection: dispatch into preflight with the
    /// chosen event and `skip_confirm = true`.
    pub fn submit_picker_confirm(&mut self) {
        let Some(event) = SUBMIT_PICKER_EVENTS
            .get(self.submit_picker_cursor)
            .map(|(_, ev)| *ev)
        else {
            self.cancel_submit_action_picker();
            return;
        };
        self.input_mode = InputMode::Normal;
        self.start_submit_with(event, true);
    }

    /// Cancel the action picker without entering preflight.
    pub fn cancel_submit_action_picker(&mut self) {
        self.input_mode = InputMode::Normal;
        self.submit_picker_cursor = 0;
    }

    pub fn cancel_submit(&mut self) {
        self.submit_state = None;
        self.input_mode = InputMode::Normal;
    }

    /// Move the resolver cursor down by one row, clamped to the last row.
    pub fn submit_resolver_cursor_down(&mut self) {
        if let Some(state) = self.submit_state.as_mut()
            && state.resolver_cursor + 1 < state.unmappable.len()
        {
            state.resolver_cursor += 1;
        }
    }

    pub fn submit_resolver_cursor_up(&mut self) {
        if let Some(state) = self.submit_state.as_mut()
            && state.resolver_cursor > 0
        {
            state.resolver_cursor -= 1;
        }
    }

    pub fn submit_resolver_toggle(&mut self) {
        use crate::forge::submit::ResolverAction;
        if let Some(state) = self.submit_state.as_mut()
            && let Some(choice) = state.resolver_choices.get_mut(state.resolver_cursor)
        {
            *choice = match choice {
                ResolverAction::MoveToSummary => ResolverAction::Omit,
                ResolverAction::Omit => ResolverAction::MoveToSummary,
            };
        }
    }

    /// Advance from the resolver. When `skip_confirm` is set (action-picker
    /// path), dispatch the network call directly; otherwise route to
    /// `SubmitConfirm` for the final confirmation modal.
    pub fn submit_resolver_advance(&mut self) {
        let Some(state) = self.submit_state.as_ref() else {
            return;
        };
        if state.skip_confirm {
            self.input_mode = InputMode::Normal;
            self.confirm_submit();
        } else {
            self.input_mode = InputMode::SubmitConfirm;
        }
    }

    /// True iff the original review head and the latest known PR head
    /// disagree. PR 5 cannot trigger this (the open-time head equals
    /// `current_pr_head`), but the field is exposed so the renderer can
    /// fold the warning in once PR 6 refreshes the remote head.
    pub fn submit_head_is_stale(&self) -> bool {
        let Some(state) = self.submit_state.as_ref() else {
            return false;
        };
        match self.current_pr_head.as_deref() {
            Some(latest) => latest != state.commit_id,
            None => false,
        }
    }

    /// Confirm submit — PR 6 dispatches the async `gh api .../reviews` call.
    /// Builds the body + payload on the main thread, saves the session, then
    /// hands off to `spawn_pr_submit`. The modal disappears immediately; a
    /// status-bar spinner takes over until the result lands in
    /// `poll_pr_submit_events`.
    pub fn confirm_submit(&mut self) {
        if let Err(e) = self.spawn_pr_submit() {
            self.set_error(format!("Submit failed: {e}"));
            self.submit_state = None;
            self.input_mode = InputMode::Normal;
        }
    }

    /// Kick off the create-review call asynchronously. Pre-submit-saves the
    /// session, builds the JSON payload on the main thread, then runs the
    /// network round-trip on a background thread. The result is applied
    /// later in `poll_pr_submit_events`.
    pub fn spawn_pr_submit(&mut self) -> Result<()> {
        use crate::forge::submit::{MovedToSummaryItem, ResolverAction, build_review_body};
        use crate::forge::traits::{CreateReviewRequest, PullRequestTarget};

        // Snapshot identity from the PR diff source first so the borrow on
        // `submit_state` below doesn't conflict.
        let DiffSource::PullRequest(pr) = self.diff_source.clone() else {
            return Err(TuicrError::UnsupportedOperation(
                "Not in PR mode".to_string(),
            ));
        };
        if self.pr_submit_state.is_some() {
            return Ok(()); // already in flight; ignore
        }

        let Some(state) = self.submit_state.take() else {
            return Ok(());
        };

        let summary_items: Vec<MovedToSummaryItem> = state
            .unmappable
            .iter()
            .zip(state.resolver_choices.iter())
            .filter_map(|(item, action)| {
                if *action == ResolverAction::MoveToSummary {
                    Some(MovedToSummaryItem {
                        comment: item.comment.clone(),
                        file: item.file.clone(),
                    })
                } else {
                    None
                }
            })
            .collect();
        let summary_comment_ids: Vec<String> =
            summary_items.iter().map(|i| i.comment.id.clone()).collect();
        let review_comment_ids: Vec<String> = self
            .session
            .review_comments
            .iter()
            .map(|c| c.id.clone())
            .collect();
        let body = build_review_body(
            &self.session.review_comments,
            &summary_items,
            &self.forge_config,
        );

        // Save the session BEFORE the network call — keeps the user's
        // local-draft work durable if anything goes sideways below.
        let _ = self.save_current_session_merging_external();

        let in_flight = SubmitInFlightState {
            event: state.event,
            mappable: state.mappable.clone(),
            summary_comment_ids,
            review_comment_ids,
            moved_to_summary_count: summary_items.len(),
            head_sha_snapshot: state.commit_id.clone(),
            repository: pr.key.repository.clone(),
            pr_number: pr.key.number,
            started_at: Instant::now(),
        };
        self.pr_submit_state = Some(in_flight.clone());
        self.input_mode = InputMode::Normal;

        let local_checkout = self
            .forge_backend
            .as_deref()
            .and_then(|backend| backend.local_checkout_path());

        let (tx, rx) = std::sync::mpsc::channel();
        self.pr_submit_rx = Some(rx);

        let repository = in_flight.repository.clone();
        let pr_number = in_flight.pr_number;
        let head_sha = in_flight.head_sha_snapshot.clone();
        let event = in_flight.event;
        let mappable = in_flight.mappable.clone();
        let commit_id = state.commit_id.clone();

        std::thread::spawn(move || {
            let backend = create_forge_backend(&repository, local_checkout);
            // Need PR details for repo/owner routing; refetch lightly via
            // the same target the user opened with.
            let target = PullRequestTarget::with_repository(
                repository.clone(),
                pr_number,
                pr_number.to_string(),
            );
            let result = match backend.get_pull_request(target) {
                Ok(details) => backend
                    .create_review(
                        &details,
                        CreateReviewRequest {
                            event,
                            commit_id: &commit_id,
                            body: &body,
                            comments: &mappable,
                        },
                    )
                    .map_err(|e| e.to_string()),
                Err(e) => Err(e.to_string()),
            };
            let _ = tx.send(PrSubmitEvent::Done {
                repository,
                pr_number,
                head_sha,
                result,
            });
        });
        Ok(())
    }

    /// Pump a pending create-review result. Applies lifecycle writes + the
    /// success message, or surfaces a sticky error.
    pub fn poll_pr_submit_events(&mut self) {
        let Some(rx) = self.pr_submit_rx.as_ref() else {
            return;
        };
        let event = match rx.try_recv() {
            Ok(e) => e,
            Err(_) => return,
        };
        self.pr_submit_rx = None;
        let in_flight = self.pr_submit_state.take();
        let PrSubmitEvent::Done {
            repository,
            pr_number,
            head_sha,
            result,
        } = event;

        // Stale-result discard: if the user reloaded the PR mid-submit, the
        // active head SHA may have moved. Drop the result rather than
        // silently mutating the wrong session.
        let Some(in_flight) = in_flight else {
            return;
        };
        let stale = in_flight.repository != repository
            || in_flight.pr_number != pr_number
            || in_flight.head_sha_snapshot != head_sha;
        if stale {
            self.set_message("Discarded stale submit result (PR was reloaded)".to_string());
            return;
        }

        self.finish_pr_submit(in_flight, result);
    }

    /// Apply the create-review result on the main thread. On success: flip
    /// each included `Comment` to `Submitted` (or `PushedDraft` for the
    /// draft event), stamp `remote_review_id`, save the session again, and
    /// publish a success message. On failure: keep everything as
    /// `LocalDraft` and set a sticky error.
    pub fn finish_pr_submit(
        &mut self,
        in_flight: SubmitInFlightState,
        result: std::result::Result<crate::forge::traits::GhCreateReviewResponse, String>,
    ) {
        use crate::forge::submit::SubmitEvent;

        let response = match result {
            Ok(r) => r,
            Err(e) => {
                self.set_error(format!("Submit failed: {e}"));
                return;
            }
        };

        self.apply_submit_success(&in_flight, &response);

        // Post-submit save — captures the lifecycle transitions.
        let _ = self.save_current_session_merging_external();

        let inline_count = in_flight.mappable.len();
        let summary_count = in_flight.moved_to_summary_count;
        let forge_name = match &self.diff_source {
            DiffSource::PullRequest(pr) => match pr.key.repository.kind {
                crate::forge::traits::ForgeKind::GitHub => "GitHub",
                crate::forge::traits::ForgeKind::GitLab => "GitLab",
            },
            _ => "forge",
        };
        let message = match in_flight.event {
            SubmitEvent::Draft => {
                let pr_url = match &self.diff_source {
                    DiffSource::PullRequest(pr) => pr.url.clone(),
                    _ => String::new(),
                };
                if pr_url.is_empty() {
                    format!(
                        "Pushed pending {forge_name} review #{}: {} inline, {} moved to summary",
                        response.id, inline_count, summary_count,
                    )
                } else {
                    format!(
                        "Pushed pending {forge_name} review #{}: {} inline, {} moved to summary — Finish it in {forge_name}: {}",
                        response.id, inline_count, summary_count, pr_url,
                    )
                }
            }
            _ => format!(
                "Submitted {forge_name} review #{}: {} inline, {} moved to summary",
                response.id, inline_count, summary_count,
            ),
        };
        self.set_message(message);

        // Refetch remote threads so the just-submitted comments appear immediately.
        self.refetch_pr_threads();
    }

    /// Flip every comment that was sent — inline, summary-bound, and review-
    /// level — from `LocalDraft` to `Submitted` (or `PushedDraft` for
    /// `:submit draft`) and stamp `remote_review_id`. The comments stay in
    /// the session so the user keeps seeing their work; they're pruned by
    /// `prune_locked_comments` when remote threads are next fetched.
    pub fn apply_submit_success(
        &mut self,
        in_flight: &SubmitInFlightState,
        response: &crate::forge::traits::GhCreateReviewResponse,
    ) {
        use crate::forge::submit::SubmitEvent;
        use crate::model::comment::CommentLifecycleState;

        let new_state = match in_flight.event {
            SubmitEvent::Draft => CommentLifecycleState::PushedDraft,
            _ => CommentLifecycleState::Submitted,
        };
        let review_id = response.id.to_string();

        let target_ids: std::collections::HashSet<&str> = in_flight
            .mappable
            .iter()
            .map(|c| c.comment_id.as_str())
            .chain(in_flight.summary_comment_ids.iter().map(String::as_str))
            .chain(in_flight.review_comment_ids.iter().map(String::as_str))
            .collect();
        if target_ids.is_empty() {
            return;
        }

        for comment in self.session.review_comments.iter_mut() {
            if target_ids.contains(comment.id.as_str()) {
                comment.lifecycle_state = new_state;
                comment.remote_review_id = Some(review_id.clone());
            }
        }
        for review in self.session.files.values_mut() {
            for comment in review.file_comments.iter_mut() {
                if target_ids.contains(comment.id.as_str()) {
                    comment.lifecycle_state = new_state;
                    comment.remote_review_id = Some(review_id.clone());
                }
            }
            for comments in review.line_comments.values_mut() {
                for comment in comments.iter_mut() {
                    if target_ids.contains(comment.id.as_str()) {
                        comment.lifecycle_state = new_state;
                        comment.remote_review_id = Some(review_id.clone());
                    }
                }
            }
        }
        self.rebuild_annotations();
    }

    /// Drop locked (`Submitted`/`PushedDraft`) comments from the session.
    /// Called after a successful `forge_review_threads` fetch: anything that
    /// was published to the forge is now represented by the fresh remote
    /// threads, so keeping the locals would double-render every line.
    pub fn prune_locked_comments(&mut self) {
        self.session.review_comments.retain(|c| !c.is_locked());
        for review in self.session.files.values_mut() {
            review.file_comments.retain(|c| !c.is_locked());
            for comments in review.line_comments.values_mut() {
                comments.retain(|c| !c.is_locked());
            }
            review.line_comments.retain(|_, v| !v.is_empty());
        }
    }

    /// Open the review target selector on a specific tab.
    ///
    /// `Local` loads the recent-commits list (same as the historical commit
    /// selector). `PullRequests` switches the tab; the actual fetch is
    /// triggered lazily through `on_target_tab_entered`.
    pub fn enter_target_selector(&mut self, initial_tab: TargetTab) -> Result<()> {
        // Save inline selection state if we have review commits
        if !self.review_commits.is_empty() {
            self.saved_inline_selection = self.commit_selection_range;
        }

        let highlighter = self.theme.syntax_highlighter();
        let change_status = Self::get_change_status_with_ignore(
            self.vcs.as_ref(),
            &self.vcs_info.root_path,
            highlighter,
            self.path_filter.as_deref(),
        )?;
        let has_staged_changes = change_status.staged;
        let has_unstaged_changes = change_status.unstaged;

        let commits = self.vcs.get_recent_commits(0, VISIBLE_COMMIT_COUNT)?;
        let no_local_targets = commits.is_empty() && !has_staged_changes && !has_unstaged_changes;
        // Allow opening the selector on the Pull Requests tab even when there
        // are no local commits or changes — the PR tab is the user's reason
        // for being here.
        if no_local_targets && initial_tab == TargetTab::Local {
            self.set_message("No commits or staged/unstaged changes found");
            return Ok(());
        }

        // Check if there might be more commits
        self.has_more_commit = commits.len() >= VISIBLE_COMMIT_COUNT;
        self.commit_list = commits;
        if has_staged_changes {
            self.commit_list.insert(0, Self::staged_commit_entry());
        }
        if has_unstaged_changes {
            self.commit_list.insert(0, Self::unstaged_commit_entry());
        }
        self.commit_list_cursor = 0;
        self.commit_list_scroll_offset = 0;
        self.commit_selection_range = None;
        self.visible_commit_count = self.commit_list.len();
        self.input_mode = InputMode::CommitSelect;

        // Reset the PR tab to Idle each time the selector is opened so the
        // fetch happens lazily on first visit.
        self.pr_tab = PullRequestsTab::new(self.forge_repository.clone());
        self.pr_filter_draft = None;
        self.pr_load_rx = None;

        self.target_tab = initial_tab;
        if initial_tab == TargetTab::PullRequests {
            self.on_target_tab_entered();
        }
        Ok(())
    }

    pub fn exit_commit_select_mode(&mut self) -> Result<()> {
        self.input_mode = InputMode::Normal;

        // If we have review commits, restore the inline selector state
        if !self.review_commits.is_empty() {
            self.commit_list = self.review_commits.clone();
            self.commit_selection_range = self.saved_inline_selection;
            self.commit_list_cursor = 0;
            self.commit_list_scroll_offset = 0;
            self.visible_commit_count = self.review_commits.len();
            self.has_more_commit = false;
            self.saved_inline_selection = None;

            // Reload diff for the restored selection
            if self.commit_selection_range.is_some() {
                self.reload_inline_selection()?;
            }
            return Ok(());
        }

        // If we were viewing commits, try to go back to working tree
        if matches!(
            self.diff_source,
            DiffSource::CommitRange(_) | DiffSource::StagedUnstagedAndCommits(_)
        ) {
            let highlighter = self.theme.syntax_highlighter();
            match Self::get_working_tree_diff_with_ignore(
                self.vcs.as_ref(),
                &self.vcs_info.root_path,
                highlighter,
                self.path_filter.as_deref(),
            ) {
                Ok(diff_files) => {
                    self.diff_files = diff_files;
                    self.diff_source = DiffSource::StagedAndUnstaged;

                    // Update session for new files
                    for file in &self.diff_files {
                        self.session.add_diff_file(file);
                    }

                    self.sort_files_by_directory(true);
                    self.expand_all_dirs();
                }
                Err(_) => {
                    self.set_message("No staged or unstaged changes");
                }
            }
        }

        Ok(())
    }

    /// Switch to the next/previous tab in the review target selector.
    /// With only two tabs, forward and reverse are equivalent; the `_forward`
    /// arg is kept so callers can pass the natural direction without a cast.
    /// Triggers the lazy PR fetch the first time the PR tab is entered.
    pub fn cycle_target_tab(&mut self, _forward: bool) {
        let next = match self.target_tab {
            TargetTab::Local => TargetTab::PullRequests,
            TargetTab::PullRequests => TargetTab::Local,
        };
        self.target_tab = next;
        if next == TargetTab::PullRequests {
            self.on_target_tab_entered();
        } else {
            // Returning to Local: clear any half-typed PR filter draft.
            self.pr_filter_draft = None;
        }
    }

    /// Entry-point hook called when the PR tab becomes visible.
    /// Triggers the first network call lazily.
    fn on_target_tab_entered(&mut self) {
        if let Some((repo, scope)) = self.pr_tab.start_initial_load() {
            let override_repo = self.repo_url_override.clone();
            let skip_resolution = self.canonical_resolved;
            self.spawn_pr_initial_load(repo, override_repo, skip_resolution, scope);
        }
    }

    /// Spawn a background thread that fetches the initial PR list. The
    /// resulting `PrLoadEvent::Initial` is delivered through `pr_load_rx`
    /// and applied in the main loop via `poll_pr_load_events`.
    ///
    /// On the first PR-tab visit (`skip_resolution=false`) the thread first
    /// resolves the detected origin to its canonical (fork-parent) repo via
    /// `gh api`, then lists PRs against that canonical. Subsequent visits
    /// reuse the already-resolved repo and skip the resolver.
    fn spawn_pr_initial_load(
        &mut self,
        origin: ForgeRepository,
        override_repo: Option<ForgeRepository>,
        skip_resolution: bool,
        scope: crate::forge::traits::PullRequestListScope,
    ) {
        use crate::forge::selector::PR_PAGE_SIZE;
        use crate::forge::traits::{ForgeKind, PullRequestListQuery};

        let (tx, rx) = std::sync::mpsc::channel();
        self.pr_load_rx = Some(rx);

        std::thread::spawn(move || {
            // Canonical resolution (fork parent lookup) is GitHub-only.
            let canonical = if skip_resolution || origin.kind != ForgeKind::GitHub {
                override_repo.unwrap_or(origin)
            } else {
                use crate::forge::canonical::resolve_canonical_repository;
                use crate::forge::github::gh::SystemGhRunner;
                let runner = SystemGhRunner;
                resolve_canonical_repository(&origin, override_repo.as_ref(), &runner)
            };
            let backend = create_forge_backend(&canonical, None);
            let query =
                PullRequestListQuery::first_page_with_scope(canonical.clone(), PR_PAGE_SIZE, scope);
            let result = backend
                .list_pull_requests(query)
                .map(|page| (page.pull_requests, page.has_more))
                .map_err(|err| err.to_string());
            let _ = tx.send(PrLoadEvent::Initial { canonical, result });
        });
    }

    /// Spawn a background thread that fetches the next page of PRs.
    fn spawn_pr_load_more(
        &mut self,
        repository: ForgeRepository,
        scope: crate::forge::traits::PullRequestListScope,
        already_loaded: usize,
    ) {
        use crate::forge::selector::PR_PAGE_SIZE;
        use crate::forge::traits::PullRequestListQuery;

        let (tx, rx) = std::sync::mpsc::channel();
        self.pr_load_rx = Some(rx);

        std::thread::spawn(move || {
            let backend = create_forge_backend(&repository, None);
            let query = PullRequestListQuery {
                repository,
                already_loaded,
                page_size: PR_PAGE_SIZE,
                scope,
            };
            let result = backend
                .list_pull_requests(query)
                .map(|page| (page.pull_requests, page.has_more))
                .map_err(|err| err.to_string());
            let _ = tx.send(PrLoadEvent::LoadMore(result));
        });
    }

    /// Pump any pending PR fetch events into the tab state.
    /// Called from the main loop each tick; non-blocking.
    pub fn poll_pr_load_events(&mut self) {
        let Some(rx) = self.pr_load_rx.as_ref() else {
            return;
        };
        let mut events = Vec::new();
        while let Ok(event) = rx.try_recv() {
            events.push(event);
        }
        if events.is_empty() {
            return;
        }
        // The channel is single-use per fetch; drop the receiver once a
        // result has arrived so we don't keep checking it.
        self.pr_load_rx = None;
        for event in events {
            match event {
                PrLoadEvent::Initial { canonical, result } => {
                    // The background thread may have resolved a fork's origin
                    // to its upstream parent. Promote App state to the
                    // resolved repo so every PR-related call (open, threads,
                    // submit, load-more) targets the canonical from here on.
                    self.pr_tab.apply_canonical(canonical.clone());
                    self.forge_repository = Some(canonical);
                    self.canonical_resolved = true;
                    self.pr_tab.apply_initial_load(result);
                }
                PrLoadEvent::LoadMore(result) => self.pr_tab.apply_load_more(result),
            }
        }
        self.pr_tab.clamp_cursor();
    }

    pub fn pr_tab_cursor_up(&mut self) {
        self.pr_tab.cursor_up();
        self.pr_tab
            .ensure_cursor_visible(self.pr_list_viewport_height);
    }

    pub fn pr_tab_cursor_down(&mut self) {
        self.pr_tab.cursor_down();
        self.pr_tab
            .ensure_cursor_visible(self.pr_list_viewport_height);
    }

    pub fn toggle_pr_review_requested_filter(&mut self) {
        let Some((repo, scope)) = self.pr_tab.toggle_scope_and_start_reload() else {
            return;
        };
        self.pr_filter_draft = None;
        let override_repo = self.repo_url_override.clone();
        let skip_resolution = self.canonical_resolved;
        self.spawn_pr_initial_load(repo, override_repo, skip_resolution, scope);
        self.set_message(format!("PR list: {}", scope.label()));
    }

    /// Handle Enter on the PR tab. Returns true when the action was handled
    /// (load more triggered, PR open kicked off, error surfaced, etc).
    pub fn pr_tab_select(&mut self) -> bool {
        // Block re-entry while a previous open is still resolving — the
        // spinner glyph on the row already tells the user something is in
        // flight.
        if self.pr_open_state.is_some() {
            return true;
        }
        if self.pr_tab.cursor_on_load_more() {
            if let Some((repo, scope, already)) = self.pr_tab.start_load_more() {
                self.spawn_pr_load_more(repo, scope, already);
            }
            return true;
        }
        // Clone the summary so we drop the immutable borrow before mutating
        // the app to enter PR mode.
        let Some(summary) = self.pr_tab.cursor_pr().cloned() else {
            return false;
        };
        self.spawn_pr_open(&summary);
        true
    }

    /// Kick off the background fetch for a PR open. The main thread keeps
    /// rendering and pumping events; the resulting `PrOpenEvent::Done` is
    /// drained in `poll_pr_open_events` where parsing happens and PR mode
    /// is entered.
    fn spawn_pr_open(&mut self, summary: &crate::forge::traits::PullRequestSummary) {
        use crate::forge::pr_open::fetch_pr_data;
        use crate::forge::traits::PullRequestTarget;

        let local_checkout = Some(self.vcs_info.root_path.clone());
        let request = PrOpenRequest {
            repository: summary.repository.clone(),
            pr_number: summary.number,
            started_at: Instant::now(),
        };
        self.pr_open_state = Some(request.clone());

        let (tx, rx) = std::sync::mpsc::channel();
        self.pr_open_rx = Some(rx);

        let summary_repo = summary.repository.clone();
        let pr_number = summary.number;
        let thread_local_checkout = local_checkout.clone();
        std::thread::spawn(move || {
            let backend = create_forge_backend(&summary_repo, thread_local_checkout);
            let target =
                PullRequestTarget::with_repository(summary_repo, pr_number, pr_number.to_string());
            let outcome = fetch_pr_data(backend.as_ref(), target).map_err(|e| e.to_string());
            let _ = tx.send(PrOpenEvent::Done {
                request,
                result: outcome,
            });
        });
    }

    /// Drain any pending PR-open result and apply it. On success, parses
    /// the diff and enters PR diff mode; on failure, routes the error
    /// into the selector banner. Either way, clears `pr_open_state` and
    /// the receiver so the spinner stops animating.
    pub fn poll_pr_open_events(&mut self) {
        let Some(rx) = self.pr_open_rx.as_ref() else {
            return;
        };
        let event = match rx.try_recv() {
            Ok(e) => e,
            Err(_) => return,
        };
        self.pr_open_rx = None;
        let in_flight = self.pr_open_state.clone();
        self.pr_open_state = None;
        match event {
            PrOpenEvent::Done { request, result } => {
                // If the user cancelled (cleared pr_open_state) but the
                // background thread sent a result before being torn down,
                // ignore the result rather than entering PR mode.
                if !in_flight
                    .as_ref()
                    .map(|s| s.matches(&request.repository, request.pr_number))
                    .unwrap_or(false)
                {
                    return;
                }
                match result {
                    Ok((details, patch, commits)) => {
                        if let Err(e) = self.finish_pr_open(details, patch, commits, &request) {
                            self.set_error(format!(
                                "Failed to open PR #{}: {}",
                                request.pr_number, e
                            ));
                        }
                    }
                    Err(e) => {
                        self.set_error(format!("Failed to open PR #{}: {}", request.pr_number, e));
                    }
                }
            }
        }
    }

    /// Main-thread half of the PR open: parse the patch, build the
    /// session, and enter PR diff mode. Mirrors what the previous synchronous
    /// `open_pr_with_backend` did, but the network fetch has already
    /// happened on the background thread.
    fn finish_pr_open(
        &mut self,
        details: crate::forge::traits::PullRequestDetails,
        patch: String,
        commits: Vec<crate::forge::traits::PullRequestCommit>,
        request: &PrOpenRequest,
    ) -> Result<()> {
        use crate::forge::pr_open::prepare_open_pr;

        let local_checkout = Some(self.vcs_info.root_path.clone());
        let highlighter = self.theme.syntax_highlighter();
        let mut opened = prepare_open_pr(
            details.clone(),
            &patch,
            commits,
            local_checkout.as_deref(),
            highlighter,
        )?;
        Self::load_or_apply_pr_session(&mut opened);
        let backend = create_forge_backend(&request.repository, local_checkout.clone());
        self.enter_pr_diff_mode(backend, opened)?;
        // Kick the remote-thread fetch off on a fresh background thread.
        // The diff view is already up; threads fade in once they land.
        self.spawn_pr_threads_fetch(&details, local_checkout);
        self.set_message(format!(
            "Opened PR {}#{}",
            request.repository.display_name(),
            request.pr_number,
        ));
        Ok(())
    }

    /// Kick off a background fetch of remote review threads for `details`.
    /// Replaces any in-flight fetch — we don't try to merge results across
    /// concurrent fetches because the head SHA scopes everything.
    fn spawn_pr_threads_fetch(
        &mut self,
        details: &crate::forge::traits::PullRequestDetails,
        local_checkout: Option<std::path::PathBuf>,
    ) {
        self.forge_review_threads.clear();
        self.forge_review_threads_loading = true;

        let (tx, rx) = std::sync::mpsc::channel();
        self.pr_threads_rx = Some(rx);

        let details_clone = details.clone();
        let repository = details.repository.clone();
        let pr_number = details.number;
        let head_sha = details.head_sha.clone();

        std::thread::spawn(move || {
            let backend = create_forge_backend(&repository, local_checkout);
            let threads = backend
                .list_review_threads(&details_clone)
                .map_err(|e| e.to_string());
            let summaries = backend
                .list_review_summaries(&details_clone)
                .map_err(|e| e.to_string());
            let _ = tx.send(PrThreadsEvent::Done {
                repository,
                pr_number,
                head_sha,
                threads,
                summaries,
            });
        });
    }

    /// Drain any pending remote-thread fetch result and apply it. Stale
    /// results (a result that arrived after the user switched to a
    /// different PR) are discarded.
    pub fn poll_pr_threads_events(&mut self) {
        let Some(rx) = self.pr_threads_rx.as_ref() else {
            return;
        };
        let event = match rx.try_recv() {
            Ok(e) => e,
            Err(_) => return,
        };
        self.pr_threads_rx = None;
        self.forge_review_threads_loading = false;

        match event {
            PrThreadsEvent::Done {
                repository,
                pr_number,
                head_sha,
                threads,
                summaries,
            } => {
                // Validate against the currently open PR. If the user has
                // opened a different PR (or left PR mode) while the fetch
                // was in flight, drop the result silently.
                let current = match &self.diff_source {
                    DiffSource::PullRequest(pr) => Some((
                        pr.key.repository.clone(),
                        pr.key.number,
                        pr.key.head_sha.clone(),
                    )),
                    _ => None,
                };
                let still_relevant = current
                    .as_ref()
                    .map(|(r, n, sha)| *r == repository && *n == pr_number && *sha == head_sha)
                    .unwrap_or(false);
                if !still_relevant {
                    return;
                }
                let mut had_error = false;
                let mut threads_loaded = false;
                match threads {
                    Ok(t) => {
                        self.forge_review_threads = t;
                        threads_loaded = true;
                    }
                    Err(e) => {
                        self.forge_review_threads = Vec::new();
                        self.set_warning(format!("Failed to load remote comments: {e}"));
                        had_error = true;
                    }
                }
                match summaries {
                    Ok(s) => {
                        self.forge_review_summaries = s;
                    }
                    Err(e) => {
                        self.forge_review_summaries = Vec::new();
                        had_error = true;
                        // Only surface the summary error if the threads call
                        // succeeded — otherwise the user already got a
                        // warning for the broader failure.
                        if threads_loaded {
                            self.set_warning(format!("Failed to load remote reviews: {e}"));
                        }
                    }
                }
                if !had_error {
                    self.prune_locked_comments();
                    let _ = self.save_current_session_merging_external();
                }
                self.rebuild_annotations();
            }
        }
    }

    /// Update the per-session remote comments visibility and repaint.
    /// Returns `true` if the visibility actually changed.
    pub fn set_remote_comments_visibility(
        &mut self,
        visibility: crate::forge::remote_comments::PrCommentsVisibility,
    ) -> bool {
        if self.session.remote_comments_visibility == visibility {
            return false;
        }
        self.session.remote_comments_visibility = visibility;
        self.rebuild_annotations();
        true
    }

    /// Abort an in-flight PR open. Drops the receiver so the eventual
    /// thread send becomes a no-op; clears the spinner state.
    pub fn cancel_pr_open(&mut self) -> bool {
        if self.pr_open_state.is_none() {
            return false;
        }
        self.pr_open_state = None;
        self.pr_open_rx = None;
        self.set_message("PR open cancelled".to_string());
        true
    }

    /// Re-fetch remote review threads for the currently open PR. Called
    /// from `:e` so users can pull the latest discussions without
    /// reopening the PR. No-op outside PR mode.
    pub fn refetch_pr_threads(&mut self) {
        let local_checkout = self
            .forge_backend
            .as_deref()
            .and_then(|b| b.local_checkout_path());
        let details = match &self.diff_source {
            DiffSource::PullRequest(pr) => crate::forge::traits::PullRequestDetails {
                repository: pr.key.repository.clone(),
                number: pr.key.number,
                title: pr.title.clone(),
                url: pr.url.clone(),
                state: pr.state.clone(),
                is_draft: false,
                author: None,
                head_ref_name: pr.head_ref_name.clone(),
                base_ref_name: pr.base_ref_name.clone(),
                head_sha: pr.key.head_sha.clone(),
                base_sha: pr.base_sha.clone(),
                body: String::new(),
                updated_at: None,
                closed: pr.closed,
                merged_at: None,
                diff_start_sha: None,
            },
            _ => return,
        };
        self.spawn_pr_threads_fetch(&details, local_checkout);
    }

    /// Open a PR using the provided forge backend, synchronously. Exists
    /// as a seam for tests that want to drive the open without spinning
    /// up a background thread + mpsc round-trip. Production paths go
    /// through `spawn_pr_open` (selector) or `new_from_pr_target` (CLI).
    ///
    /// Synchronously fetches `list_review_threads` from the same backend
    /// and applies it before returning. This is the convenient seam for
    /// integration tests; the production async path uses
    /// `spawn_pr_threads_fetch` instead.
    #[allow(dead_code)]
    pub fn open_pr_with_backend(
        &mut self,
        summary: &crate::forge::traits::PullRequestSummary,
        backend: Box<dyn ForgeBackend>,
        local_checkout: Option<std::path::PathBuf>,
    ) -> Result<()> {
        use crate::forge::pr_open::open_pull_request;
        use crate::forge::traits::PullRequestTarget;

        let target = PullRequestTarget::with_repository(
            summary.repository.clone(),
            summary.number,
            summary.number.to_string(),
        );
        let highlighter = self.theme.syntax_highlighter();
        let mut opened = open_pull_request(
            backend.as_ref(),
            target,
            local_checkout.as_deref(),
            highlighter,
        )?;
        Self::load_or_apply_pr_session(&mut opened);
        // Sync thread + summary fetch — tests assert on
        // `app.forge_review_threads`/`forge_review_summaries` immediately
        // after this returns.
        let threads = backend
            .list_review_threads(&opened.details)
            .unwrap_or_default();
        let summaries = backend
            .list_review_summaries(&opened.details)
            .unwrap_or_default();
        self.enter_pr_diff_mode(backend, opened)?;
        self.forge_review_threads = threads;
        self.forge_review_summaries = summaries;
        self.prune_locked_comments();
        self.rebuild_annotations();
        Ok(())
    }

    pub fn begin_pr_filter(&mut self) {
        if !self.pr_tab.is_loaded() {
            return;
        }
        // Seed the draft from the current applied filter so the user can
        // refine it. Starting from empty is also reasonable; preserving the
        // current filter feels less surprising when re-opening.
        let current = match &self.pr_tab {
            PullRequestsTab::Loaded { filter, .. } => filter.clone(),
            _ => String::new(),
        };
        self.pr_filter_draft = Some(current);
    }

    pub fn commit_pr_filter(&mut self) {
        if let Some(draft) = self.pr_filter_draft.take() {
            self.pr_tab.set_filter(draft);
        }
    }

    pub fn cancel_pr_filter(&mut self) {
        self.pr_filter_draft = None;
    }

    pub fn pr_filter_insert_char(&mut self, ch: char) {
        if let Some(draft) = self.pr_filter_draft.as_mut() {
            draft.push(ch);
        }
    }

    pub fn pr_filter_delete_char(&mut self) {
        if let Some(draft) = self.pr_filter_draft.as_mut() {
            draft.pop();
        }
    }

    pub fn pr_filter_clear(&mut self) {
        if let Some(draft) = self.pr_filter_draft.as_mut() {
            draft.clear();
        }
    }

    pub fn pr_filter_editing(&self) -> bool {
        self.pr_filter_draft.is_some()
    }

    pub fn toggle_diff_view_mode(&mut self) {
        if self.is_pristine_mode {
            // Side-by-side has nothing to show in pristine mode: there is no
            // diff, so the two panes would render identical content. Keep
            // the view unified and tell the user why the toggle did nothing.
            self.set_message("side-by-side not available in pristine mode");
            return;
        }
        self.diff_view_mode = match self.diff_view_mode {
            DiffViewMode::Unified => DiffViewMode::SideBySide,
            DiffViewMode::SideBySide => DiffViewMode::Unified,
        };
        let mode_name = match self.diff_view_mode {
            DiffViewMode::Unified => "unified",
            DiffViewMode::SideBySide => "side-by-side",
        };
        self.set_message(format!("Diff view mode: {mode_name}"));
        self.rebuild_annotations();
    }

    pub fn toggle_file_list(&mut self) {
        self.show_file_list = !self.show_file_list;
        if !self.show_file_list
            && matches!(
                self.focused_panel,
                FocusedPanel::FileList | FocusedPanel::Comments
            )
        {
            self.focused_panel = FocusedPanel::Diff;
        }
        let status = if self.show_file_list {
            "visible"
        } else {
            "hidden"
        };
        self.set_message(format!("File list: {status}"));
    }

    /// Toggle single-file view. When on, the diff panel renders only the
    /// currently focused file instead of the full continuous-scroll
    /// concatenation. Annotations, navigation, and export work the same
    /// way on the rendered subset.
    pub fn toggle_single_file_view(&mut self) {
        self.is_single_file_view = !self.is_single_file_view;
        // `calculate_file_scroll_offset` changes meaning across modes
        // (single-file stops at review-comments header, all-files
        // accumulates), so re-snap the viewport to the current file.
        let start = self.calculate_file_scroll_offset(self.diff_state.current_file_idx);
        self.diff_state.scroll_offset = start;
        self.diff_state.cursor_line = start;
        let status = if self.is_single_file_view {
            "single file"
        } else {
            "all files"
        };
        self.set_message(format!("View: {status}"));
        self.rebuild_annotations();
    }

    /// Whether the inline commit selector panel should be displayed.
    pub fn has_inline_commit_selector(&self) -> bool {
        self.show_commit_selector
            && self.review_commits.len() > 1
            && !matches!(&self.diff_source, DiffSource::WorkingTree)
    }

    // Commit selection methods

    pub fn commit_select_up(&mut self) {
        if self.commit_list_cursor > 0 {
            self.commit_list_cursor -= 1;
            // Scroll up if cursor goes above visible area
            if self.commit_list_cursor < self.commit_list_scroll_offset {
                self.commit_list_scroll_offset = self.commit_list_cursor;
            }
        }
    }

    pub fn commit_select_down(&mut self) {
        let max_cursor = if self.can_show_more_commits() {
            self.visible_commit_count
        } else {
            self.visible_commit_count.saturating_sub(1)
        };

        if self.commit_list_cursor < max_cursor {
            self.commit_list_cursor += 1;
            // Scroll down if cursor goes below visible area
            if self.commit_list_viewport_height > 0
                && self.commit_list_cursor
                    >= self.commit_list_scroll_offset + self.commit_list_viewport_height
            {
                self.commit_list_scroll_offset =
                    self.commit_list_cursor - self.commit_list_viewport_height + 1;
            }
        }
    }

    /// Toggle the cursor commit's membership in the selection range, then
    /// (only if the cursor commit was newly added to the selection) move the
    /// cursor past the end of the range. Lets the user press Enter/Space
    /// repeatedly to sweep a contiguous run of commits.
    ///
    /// Other toggle outcomes leave the cursor in place: edge presses
    /// (deselect the cursor commit), middle presses (truncate the range
    /// without unselecting the cursor commit), and clearing the last
    /// selection. Those aren't "sweep" actions, so advancing would surprise.
    pub fn toggle_commit_selection_and_advance(&mut self) {
        let cursor = self.commit_list_cursor;
        let was_selected = self.is_commit_selected(cursor);
        self.toggle_commit_selection();
        let now_selected = self.is_commit_selected(cursor);
        if was_selected || !now_selected {
            return;
        }
        if let Some((_, end)) = self.commit_selection_range {
            while self.commit_list_cursor <= end {
                let before = self.commit_list_cursor;
                self.commit_select_down();
                if self.commit_list_cursor == before {
                    return;
                }
            }
        }
    }

    // Check if cursor is on the commit expand row
    pub fn is_on_expand_row(&self) -> bool {
        self.can_show_more_commits() && self.commit_list_cursor == self.visible_commit_count
    }

    pub fn can_show_more_commits(&self) -> bool {
        self.visible_commit_count < self.commit_list.len() || self.has_more_commit
    }

    // Expand the commit list to show more commits
    pub fn expand_commit(&mut self) -> Result<()> {
        if self.visible_commit_count < self.commit_list.len() {
            self.visible_commit_count =
                (self.visible_commit_count + self.commit_page_size).min(self.commit_list.len());
            return Ok(());
        }

        if !self.has_more_commit {
            self.set_message("No more commits");
            return Ok(());
        }

        let offset = self.loaded_history_commit_count();
        let limit = self.commit_page_size;

        let new_commits = self.vcs.get_recent_commits(offset, limit)?;

        if new_commits.is_empty() {
            self.has_more_commit = false;
            self.set_message("No more commits");
            return Ok(());
        }

        if new_commits.len() < limit {
            self.has_more_commit = false;
            self.set_message("No more commits");
        }

        self.commit_list.extend(new_commits);
        self.visible_commit_count = self.commit_list.len();

        Ok(())
    }

    pub fn toggle_commit_selection(&mut self) {
        let cursor = self.commit_list_cursor;
        if cursor >= self.commit_list.len() {
            return;
        }

        match self.commit_selection_range {
            None => {
                // No selection yet - select just this commit
                self.commit_selection_range = Some((cursor, cursor));
            }
            Some((start, end)) => {
                let all_commits_selected =
                    self.commit_list.len() > 1 && start == 0 && end == self.commit_list.len() - 1;
                if all_commits_selected {
                    self.commit_selection_range = Some((cursor, cursor));
                    return;
                }

                if cursor >= start && cursor <= end {
                    // Cursor is within the range - shrink or deselect
                    if start == end {
                        // Only one commit selected, deselect all
                        self.commit_selection_range = None;
                    } else if cursor == start {
                        // At start edge - shrink from start
                        self.commit_selection_range = Some((start + 1, end));
                    } else if cursor == end {
                        // At end edge - shrink from end
                        self.commit_selection_range = Some((start, end - 1));
                    } else {
                        // In the middle - deselect cursor and everything after it
                        self.commit_selection_range = Some((start, cursor - 1));
                    }
                } else {
                    // Cursor is outside the range - extend to include it
                    let new_start = start.min(cursor);
                    let new_end = end.max(cursor);
                    self.commit_selection_range = Some((new_start, new_end));
                }
            }
        }
    }

    /// Check if a commit at the given index is selected
    pub fn is_commit_selected(&self, index: usize) -> bool {
        match self.commit_selection_range {
            Some((start, end)) => index >= start && index <= end,
            None => false,
        }
    }

    /// Cycle inline commit selector to the next individual commit (`)` key).
    /// all → last, i → i+1, last → all
    pub fn cycle_commit_next(&mut self) {
        if self.review_commits.is_empty() {
            return;
        }
        let n = self.review_commits.len();
        let all_selected = Some((0, n - 1));

        if self.commit_selection_range == all_selected {
            // all → last
            self.commit_selection_range = Some((n - 1, n - 1));
            self.commit_list_cursor = n - 1;
        } else if let Some((i, j)) = self.commit_selection_range {
            if i == j {
                // Single commit selected
                if i == n - 1 {
                    // last → all
                    self.commit_selection_range = all_selected;
                } else {
                    // i → i+1
                    self.commit_selection_range = Some((i + 1, i + 1));
                    self.commit_list_cursor = i + 1;
                }
            } else {
                // Multi-commit subrange → select last of that range
                self.commit_selection_range = Some((j, j));
                self.commit_list_cursor = j;
            }
        } else {
            // None selected → select all
            self.commit_selection_range = all_selected;
        }
    }

    /// Cycle inline commit selector to the previous individual commit (`(` key).
    /// all → first, i → i-1, first → all
    pub fn cycle_commit_prev(&mut self) {
        if self.review_commits.is_empty() {
            return;
        }
        let n = self.review_commits.len();
        let all_selected = Some((0, n - 1));

        if self.commit_selection_range == all_selected {
            // all → first
            self.commit_selection_range = Some((0, 0));
            self.commit_list_cursor = 0;
        } else if let Some((i, j)) = self.commit_selection_range {
            if i == j {
                // Single commit selected
                if i == 0 {
                    // first → all
                    self.commit_selection_range = all_selected;
                } else {
                    // i → i-1
                    self.commit_selection_range = Some((i - 1, i - 1));
                    self.commit_list_cursor = i - 1;
                }
            } else {
                // Multi-commit subrange → select first of that range
                self.commit_selection_range = Some((i, i));
                self.commit_list_cursor = i;
            }
        } else {
            // None selected → select all
            self.commit_selection_range = all_selected;
        }
    }

    pub fn confirm_commit_selection(&mut self) -> Result<()> {
        let selection = match self.commit_selection_range {
            Some((start, end)) => format!(
                "range={start}..={end}, rows={}",
                end.saturating_sub(start) + 1
            ),
            None => "range=none, rows=0".to_string(),
        };
        crate::profile::time_with(
            "commit_select.confirm_selection",
            || self.confirm_commit_selection_inner(),
            |result| format!("{selection}, {}", profile_unit_result(result)),
        )
    }

    fn confirm_commit_selection_inner(&mut self) -> Result<()> {
        let Some((start, end)) = self.commit_selection_range else {
            self.set_message("Select at least one commit");
            return Ok(());
        };

        // Collect selected entries in order from oldest to newest (end..start).
        let selected_commits: Vec<CommitInfo> = (start..=end)
            .rev()
            .filter_map(|i| self.commit_list.get(i))
            .cloned()
            .collect();

        if selected_commits.is_empty() {
            self.set_message("Select at least one commit");
            return Ok(());
        }

        let selected_staged = selected_commits.iter().any(Self::is_staged_commit);
        let selected_unstaged = selected_commits.iter().any(Self::is_unstaged_commit);
        let selected_ids: Vec<String> = selected_commits
            .iter()
            .filter(|c| !Self::is_special_commit(c))
            .map(|c| c.id.clone())
            .collect();

        if (selected_staged || selected_unstaged) && !selected_ids.is_empty() {
            return self.load_staged_unstaged_and_commits_selection(selected_ids, selected_commits);
        }

        if selected_staged && selected_unstaged {
            return self.load_staged_and_unstaged_selection();
        }

        if selected_staged {
            return self.load_staged_selection();
        }

        if selected_unstaged {
            return self.load_unstaged_selection();
        }

        // Get the diff for the selected commits
        let highlighter = self.theme.syntax_highlighter();
        let diff_files = Self::get_commit_range_diff_with_ignore(
            self.vcs.as_ref(),
            &self.vcs_info.root_path,
            &ResolvedRevisionRange::from_commit_ids(&selected_ids, RevisionDiffTarget::CommitList),
            highlighter,
            self.path_filter.as_deref(),
        )?;

        if diff_files.is_empty() {
            self.set_message("No changes in selected commits");
            return Ok(());
        }

        // Update session with the newest commit as base
        let newest_commit_id = selected_ids.last().unwrap().clone();
        let loaded_session = load_latest_session_for_context(
            &self.vcs_info.root_path,
            self.vcs_info.branch_name.as_deref(),
            &newest_commit_id,
            SessionDiffSource::CommitRange,
            Some(selected_ids.as_slice()),
        )
        .ok()
        .and_then(|found| found.map(|(_path, session)| session));

        let mut session = loaded_session.unwrap_or_else(|| {
            let mut session = ReviewSession::new(
                self.vcs_info.root_path.clone(),
                newest_commit_id,
                self.vcs_info.branch_name.clone(),
                SessionDiffSource::CommitRange,
            );
            session.commit_range = Some(selected_ids.clone());
            session
        });

        if session.commit_range.is_none() {
            session.commit_range = Some(selected_ids.clone());
            session.updated_at = chrono::Utc::now();
        }

        self.session = session;

        // Add files to session
        for file in &diff_files {
            self.session.add_diff_file(file);
        }
        self.reset_persisted_session_tracking();

        // Update app state
        self.diff_files = diff_files;
        self.diff_source = DiffSource::CommitRange(selected_ids);
        self.input_mode = InputMode::Normal;

        // Reset navigation state
        self.diff_state = DiffState::default();
        self.file_list_state = FileListState::default();

        // Set up inline commit selector for multi-commit reviews (newest-first display order)
        self.review_commits = selected_commits.iter().rev().cloned().collect();
        self.range_diff_files = Some(self.diff_files.clone());
        self.commit_list = self.review_commits.clone();
        self.commit_list_cursor = 0;
        self.commit_selection_range = if self.review_commits.is_empty() {
            None
        } else {
            Some((0, self.review_commits.len() - 1))
        };
        self.commit_list_scroll_offset = 0;
        self.visible_commit_count = self.review_commits.len();
        self.has_more_commit = false;
        self.show_commit_selector = self.review_commits.len() > 1;
        self.commit_diff_cache.clear();
        self.saved_inline_selection = None;

        self.sort_files_by_directory(true);
        self.expand_all_dirs();
        self.rebuild_annotations();

        Ok(())
    }

    /// Reload the diff for the currently selected inline commit subrange.
    pub fn reload_inline_selection(&mut self) -> Result<()> {
        let Some((start, end)) = self.commit_selection_range else {
            self.set_message("Select at least one commit");
            return Ok(());
        };

        // Check if all commits selected -> use cached range_diff_files
        if start == 0
            && end == self.review_commits.len() - 1
            && let Some(ref files) = self.range_diff_files
        {
            self.diff_files = files.clone();
            let wrap = self.diff_state.wrap_lines;
            self.diff_state = DiffState::default();
            self.diff_state.wrap_lines = wrap;
            self.file_list_state = FileListState::default();
            self.expanded_top.clear();
            self.expanded_bottom.clear();
            self.insert_commit_message_if_single();
            self.sort_files_by_directory(true);
            self.expand_all_dirs();
            self.rebuild_annotations();
            return Ok(());
        }

        // Check cache for this subrange
        if let Some(files) = self.commit_diff_cache.get(&(start, end)) {
            self.diff_files = files.clone();
            let wrap = self.diff_state.wrap_lines;
            self.diff_state = DiffState::default();
            self.diff_state.wrap_lines = wrap;
            self.file_list_state = FileListState::default();
            self.expanded_top.clear();
            self.expanded_bottom.clear();
            self.insert_commit_message_if_single();
            self.sort_files_by_directory(true);
            self.expand_all_dirs();
            self.rebuild_annotations();
            return Ok(());
        }

        // Load diff for selected subrange
        let has_staged = (start..=end).any(|i| {
            self.review_commits
                .get(i)
                .is_some_and(Self::is_staged_commit)
        });
        let has_unstaged = (start..=end).any(|i| {
            self.review_commits
                .get(i)
                .is_some_and(Self::is_unstaged_commit)
        });
        let selected_ids: Vec<String> = (start..=end)
            .rev() // oldest to newest
            .filter_map(|i| self.review_commits.get(i))
            .filter(|c| !Self::is_special_commit(c))
            .map(|c| c.id.clone())
            .collect();

        let highlighter = self.theme.syntax_highlighter();
        let diff_files = if (has_staged || has_unstaged) && !selected_ids.is_empty() {
            match Self::get_working_tree_with_commits_diff_with_ignore(
                self.vcs.as_ref(),
                &self.vcs_info.root_path,
                &selected_ids,
                highlighter,
                self.path_filter.as_deref(),
            ) {
                Ok(files) => files,
                Err(TuicrError::NoChanges) => Vec::new(),
                Err(e) => return Err(e),
            }
        } else if has_staged && has_unstaged {
            match Self::get_working_tree_diff_with_ignore(
                self.vcs.as_ref(),
                &self.vcs_info.root_path,
                highlighter,
                self.path_filter.as_deref(),
            ) {
                Ok(files) => files,
                Err(TuicrError::NoChanges) => Vec::new(),
                Err(e) => return Err(e),
            }
        } else if has_staged {
            match Self::get_staged_diff_with_ignore(
                self.vcs.as_ref(),
                &self.vcs_info.root_path,
                highlighter,
                self.path_filter.as_deref(),
            ) {
                Ok(files) => files,
                Err(TuicrError::NoChanges) => Vec::new(),
                Err(e) => return Err(e),
            }
        } else if has_unstaged {
            match Self::get_unstaged_diff_with_ignore(
                self.vcs.as_ref(),
                &self.vcs_info.root_path,
                highlighter,
                self.path_filter.as_deref(),
            ) {
                Ok(files) => files,
                Err(TuicrError::NoChanges) => Vec::new(),
                Err(e) => return Err(e),
            }
        } else {
            match Self::get_commit_range_diff_with_ignore(
                self.vcs.as_ref(),
                &self.vcs_info.root_path,
                &ResolvedRevisionRange::from_commit_ids(
                    &selected_ids,
                    RevisionDiffTarget::CommitList,
                ),
                highlighter,
                self.path_filter.as_deref(),
            ) {
                Ok(files) => files,
                Err(TuicrError::NoChanges) => Vec::new(),
                Err(e) => return Err(e),
            }
        };
        self.commit_diff_cache
            .insert((start, end), diff_files.clone());
        self.diff_files = diff_files;

        // Reset navigation, rebuild file tree + annotations
        let wrap = self.diff_state.wrap_lines;
        self.diff_state = DiffState::default();
        self.diff_state.wrap_lines = wrap;
        self.file_list_state = FileListState::default();
        self.expanded_top.clear();
        self.expanded_bottom.clear();
        self.insert_commit_message_if_single();
        self.sort_files_by_directory(true);
        self.expand_all_dirs();
        self.rebuild_annotations();

        Ok(())
    }

    fn load_staged_unstaged_and_commits_selection(
        &mut self,
        selected_ids: Vec<String>,
        selected_commits: Vec<CommitInfo>,
    ) -> Result<()> {
        let highlighter = self.theme.syntax_highlighter();
        let diff_files = match Self::get_working_tree_with_commits_diff_with_ignore(
            self.vcs.as_ref(),
            &self.vcs_info.root_path,
            &selected_ids,
            highlighter,
            self.path_filter.as_deref(),
        ) {
            Ok(diff_files) => diff_files,
            Err(TuicrError::NoChanges) => {
                self.set_message("No changes in selected commits + staged/unstaged");
                return Ok(());
            }
            Err(e) => return Err(e),
        };

        self.session =
            Self::load_or_create_staged_unstaged_and_commits_session(&self.vcs_info, &selected_ids);

        for file in &diff_files {
            self.session.add_diff_file(file);
        }
        self.reset_persisted_session_tracking();

        self.diff_files = diff_files;
        self.diff_source = DiffSource::StagedUnstagedAndCommits(selected_ids);
        self.input_mode = InputMode::Normal;
        self.diff_state = DiffState::default();
        self.file_list_state = FileListState::default();

        // Set up inline commit selector (newest-first display order)
        self.review_commits = selected_commits.into_iter().rev().collect();
        self.range_diff_files = Some(self.diff_files.clone());
        self.commit_list = self.review_commits.clone();
        self.commit_list_cursor = 0;
        self.commit_selection_range = if self.review_commits.is_empty() {
            None
        } else {
            Some((0, self.review_commits.len() - 1))
        };
        self.commit_list_scroll_offset = 0;
        self.visible_commit_count = self.review_commits.len();
        self.has_more_commit = false;
        self.show_commit_selector = self.review_commits.len() > 1;
        self.commit_diff_cache.clear();
        self.saved_inline_selection = None;

        self.insert_commit_message_if_single();
        self.sort_files_by_directory(true);
        self.expand_all_dirs();
        self.rebuild_annotations();
        Ok(())
    }

    fn sort_files_by_directory(&mut self, reset_position: bool) {
        use std::collections::BTreeMap;
        use std::path::Path;

        self.file_line_count_cache.clear();

        let current_path = if !reset_position {
            self.current_file_path().cloned()
        } else {
            None
        };

        let mut dir_map: BTreeMap<String, Vec<DiffFile>> = BTreeMap::new();
        let mut commit_msg_files: Vec<DiffFile> = Vec::new();

        for file in self.diff_files.drain(..) {
            if file.is_commit_message {
                commit_msg_files.push(file);
                continue;
            }
            let path = file.display_path();
            let dir = if let Some(parent) = path.parent() {
                if parent == Path::new("") {
                    ".".to_string()
                } else {
                    parent.to_string_lossy().to_string()
                }
            } else {
                ".".to_string()
            };

            dir_map.entry(dir).or_default().push(file);
        }

        self.diff_files.extend(commit_msg_files);
        for (_dir, files) in dir_map {
            self.diff_files.extend(files);
        }

        if let Some(path) = current_path
            && let Some(idx) = self
                .diff_files
                .iter()
                .position(|f| f.display_path() == &path)
        {
            self.jump_to_file(idx);
            return;
        }

        // Start at the overview position (review comments header)
        // so the diff title shows total stats on launch.
        self.diff_state.cursor_line = 0;
        self.diff_state.scroll_offset = 0;
        self.diff_state.current_file_idx = 0;
    }

    pub fn expand_all_dirs(&mut self) {
        use std::path::Path;

        self.expanded_dirs.clear();
        for file in &self.diff_files {
            let path = file.display_path();
            let mut current = path.parent();
            while let Some(parent) = current {
                if parent != Path::new("") {
                    self.expanded_dirs
                        .insert(parent.to_string_lossy().to_string());
                }
                current = parent.parent();
            }
        }
        self.ensure_valid_tree_selection();
    }

    pub fn collapse_all_dirs(&mut self) {
        self.expanded_dirs.clear();
        self.ensure_valid_tree_selection();
    }

    pub fn toggle_directory(&mut self, dir_path: &str) {
        if self.expanded_dirs.contains(dir_path) {
            self.expanded_dirs.remove(dir_path);
            self.ensure_valid_tree_selection();
        } else {
            self.expanded_dirs.insert(dir_path.to_string());
        }
    }

    /// Get the line boundaries (start_line, end_line) of a gap.
    fn gap_boundaries(&self, gap_id: &GapId) -> Option<(u32, u32)> {
        let file = self.diff_files.get(gap_id.file_idx)?;

        if gap_id.hunk_idx == file.hunks.len() {
            // End-of-file gap: starts after last hunk, ends at file end
            let last_hunk = file.hunks.last()?;
            let start = last_hunk.new_start + last_hunk.new_count;
            let end = *self.file_line_count_cache.get(&gap_id.file_idx)?;
            if start > end {
                None
            } else {
                Some((start, end))
            }
        } else {
            let hunk = file.hunks.get(gap_id.hunk_idx)?;
            let prev_hunk = if gap_id.hunk_idx > 0 {
                file.hunks.get(gap_id.hunk_idx - 1)
            } else {
                None
            };
            let (start, end) = match prev_hunk {
                None => (1, hunk.new_start.saturating_sub(1)),
                Some(prev) => (
                    prev.new_start + prev.new_count,
                    hunk.new_start.saturating_sub(1),
                ),
            };
            if start > end {
                None
            } else {
                Some((start, end))
            }
        }
    }

    /// Look up an expanded context line by sequential index across top + bottom.
    fn get_expanded_line(&self, gap_id: &GapId, idx: usize) -> Option<&DiffLine> {
        let top = self.expanded_top.get(gap_id);
        let top_len = top.map_or(0, |v| v.len());
        if idx < top_len {
            top?.get(idx)
        } else {
            self.expanded_bottom.get(gap_id)?.get(idx - top_len)
        }
    }

    /// Expand a gap in the given direction.
    /// If `limit` is Some(n), expand up to n lines. If None, expand all remaining.
    pub fn expand_gap(
        &mut self,
        gap_id: GapId,
        direction: ExpandDirection,
        limit: Option<usize>,
    ) -> Result<()> {
        // Ensure file line count is cached for EOF gaps
        self.ensure_file_line_count_cached(gap_id.file_idx);

        let (gap_start, gap_end) = self
            .gap_boundaries(&gap_id)
            .ok_or_else(|| TuicrError::CorruptedSession(format!("Invalid gap: {:?}", gap_id)))?;

        let file = &self.diff_files[gap_id.file_idx];
        let old_path = file.old_path.clone();
        let new_path = file.new_path.clone();
        let file_status = file.status;

        let top_len = self.expanded_top.get(&gap_id).map_or(0, |v| v.len()) as u32;
        let bot_len = self.expanded_bottom.get(&gap_id).map_or(0, |v| v.len()) as u32;

        // The unexpanded region runs from (gap_start + top_len) to (gap_end - bot_len)
        let inner_start = gap_start + top_len;
        let inner_end = gap_end.saturating_sub(bot_len);

        if inner_start > inner_end {
            return Ok(()); // Fully expanded
        }

        // Compute the delta between new-side and old-side line numbers for this
        // gap. Expanded context is fetched in new-side coordinates; the old-side
        // number is `new_lineno - delta`.
        let delta = {
            let file = &self.diff_files[gap_id.file_idx];
            if gap_id.hunk_idx == 0 {
                0i64
            } else {
                let prev = &file.hunks[gap_id.hunk_idx - 1];
                let old_end = prev.old_start as i64 + prev.old_count as i64;
                let new_end = prev.new_start as i64 + prev.new_count as i64;
                new_end - old_end
            }
        };

        let fetch = |start: u32, end: u32| -> Result<Vec<DiffLine>> {
            let mut lines = self.context_provider().fetch_context_lines(
                old_path.as_ref(),
                new_path.as_ref(),
                file_status,
                start,
                end,
            )?;
            for line in &mut lines {
                if let Some(n) = line.new_lineno {
                    line.old_lineno = Some((n as i64 - delta) as u32);
                }
            }
            Ok(lines)
        };

        match direction {
            ExpandDirection::Down => {
                let n = limit.unwrap_or(usize::MAX) as u32;
                let fetch_end = inner_start.saturating_add(n - 1).min(inner_end);
                let new_lines = fetch(inner_start, fetch_end)?;
                self.expanded_top
                    .entry(gap_id.clone())
                    .or_default()
                    .extend(new_lines);
            }
            ExpandDirection::Up => {
                let n = limit.unwrap_or(usize::MAX) as u32;
                let fetch_start = inner_end.saturating_sub(n - 1).max(inner_start);
                let new_lines = fetch(fetch_start, inner_end)?;
                // Prepend: new lines go before existing bottom lines
                let existing = self.expanded_bottom.remove(&gap_id).unwrap_or_default();
                let mut combined = new_lines;
                combined.extend(existing);
                self.expanded_bottom.insert(gap_id.clone(), combined);
            }
            ExpandDirection::Both => {
                // Fetch everything remaining
                let new_lines = fetch(inner_start, inner_end)?;
                self.expanded_top
                    .entry(gap_id.clone())
                    .or_default()
                    .extend(new_lines);
            }
        }

        self.rebuild_annotations();
        Ok(())
    }

    /// Resolve the right `ContextProvider` for the current diff source.
    /// In PR mode (with a forge backend present), expansion goes through the
    /// forge; otherwise it goes through the local VCS backend.
    fn ref_commit(&self) -> Option<&str> {
        match &self.diff_source {
            DiffSource::CommitRange(commits) => {
                // When the inline commit selector narrows to a subrange,
                // review_commits is newest-first so index `start` is the
                // newest selected commit — that's the snapshot to read from.
                if let Some((start, _)) = self.commit_selection_range {
                    self.review_commits
                        .get(start)
                        .map(|c| c.id.as_str())
                        .or_else(|| commits.last().map(|s| s.as_str()))
                } else {
                    commits.last().map(|s| s.as_str())
                }
            }
            _ => None,
        }
    }

    fn context_provider(&self) -> Box<dyn ContextProvider + '_> {
        if let (DiffSource::PullRequest(pr), Some(backend)) =
            (&self.diff_source, self.forge_backend.as_ref())
        {
            Box::new(ForgeContextProvider {
                forge: backend.as_ref(),
                repository: pr.key.repository.clone(),
                base_sha: pr.base_sha.clone(),
                head_sha: pr.key.head_sha.clone(),
            })
        } else {
            Box::new(VcsContextProvider {
                vcs: self.vcs.as_ref(),
                ref_commit: self.ref_commit().map(|s| s.to_string()),
            })
        }
    }

    /// Collapse an expanded gap
    pub fn collapse_gap(&mut self, gap_id: GapId) {
        self.expanded_top.remove(&gap_id);
        self.expanded_bottom.remove(&gap_id);
        self.rebuild_annotations();
    }

    /// Clear all expanded gaps (called when reloading diffs)
    pub fn clear_expanded_gaps(&mut self) {
        self.expanded_top.clear();
        self.expanded_bottom.clear();
        self.file_line_count_cache.clear();
    }

    fn eof_gap_enabled(&self) -> bool {
        matches!(
            self.diff_source,
            DiffSource::WorkingTree
                | DiffSource::Unstaged
                | DiffSource::StagedAndUnstaged
                | DiffSource::StagedUnstagedAndCommits(_)
                | DiffSource::CommitRange(_)
        )
    }

    /// Ensure the file line count cache is populated for a given file.
    fn ensure_file_line_count_cached(&mut self, file_idx: usize) {
        if !self.eof_gap_enabled() || self.file_line_count_cache.contains_key(&file_idx) {
            return;
        }
        if let Some(file) = self.diff_files.get(file_idx) {
            let path = file.display_path().clone();
            let status = file.status;
            let ref_commit = self.ref_commit().map(|s| s.to_string());
            if let Ok(count) = self
                .vcs
                .file_line_count(&path, status, ref_commit.as_deref())
            {
                self.file_line_count_cache.insert(file_idx, count);
            }
        }
    }

    /// Populate the file line count cache for all eligible files.
    /// Only enabled for diff sources where the worktree/index is the correct snapshot.
    fn populate_file_line_count_cache(&mut self) {
        self.file_line_count_cache.clear();
        if self.eof_gap_enabled() {
            for file_idx in 0..self.diff_files.len() {
                let file = &self.diff_files[file_idx];
                if !file.hunks.is_empty() && file.status != FileStatus::Deleted {
                    self.ensure_file_line_count_cached(file_idx);
                }
            }
        }
    }

    /// Rebuild the line annotations cache. Call this when:
    /// - Diff files change (load/reload)
    /// - Expansion state changes (expand/collapse gap)
    /// - Comments are added/removed
    /// - Diff view mode changes
    pub fn rebuild_annotations(&mut self) {
        if self.file_line_count_cache.is_empty() {
            self.populate_file_line_count_cache();
        }

        self.line_annotations.clear();

        // Pre-index remote threads by (path, line, side) for quick lookup
        // during the file/hunk walk. Threads whose visibility is
        // suppressed don't appear in this map at all, so no annotations
        // are emitted for them.
        let remote_index = self.build_remote_thread_index();

        // The review-comments header is omitted in single-file view (see
        // the matching guard in `src/ui/diff_unified.rs`), so the
        // annotation list mirrors the render.
        if !self.is_single_file_view {
            self.line_annotations
                .push(AnnotatedLine::ReviewCommentsHeader);
        }
        for (summary_idx, summary) in self.forge_review_summaries.iter().enumerate() {
            let summary_lines = crate::forge::remote_comments::summary_display_lines(summary);
            for _ in 0..summary_lines {
                self.line_annotations
                    .push(AnnotatedLine::RemoteReviewSummaryLine { summary_idx });
            }
        }
        for (comment_idx, comment) in self.session.review_comments.iter().enumerate() {
            let comment_lines =
                Self::comment_display_lines(comment, self.diff_state.viewport_width);
            for _ in 0..comment_lines {
                self.line_annotations
                    .push(AnnotatedLine::ReviewComment { comment_idx });
            }
        }

        // Emit annotation entries for remote review-level threads (line: None).
        {
            use crate::forge::remote_comments::{PrCommentsVisibility, thread_display_lines};
            let visibility = self.session.remote_comments_visibility;
            if !matches!(visibility, PrCommentsVisibility::Hide) {
                for (thread_idx, thread) in self.forge_review_threads.iter().enumerate() {
                    if thread.line.is_some() {
                        continue;
                    }
                    let Some(_muted) = visibility.render_decision(thread) else {
                        continue;
                    };
                    let n = thread_display_lines(thread);
                    for _ in 0..n {
                        self.line_annotations
                            .push(AnnotatedLine::RemoteThreadLine { thread_idx });
                    }
                }
            }
        }

        for (file_idx, file) in self.diff_files.iter().enumerate() {
            // Single-file view renders only the currently focused file,
            // so the annotation stream must skip every other file or
            // click handling lands on lines that aren't visible.
            if self.is_single_file_view && file_idx != self.diff_state.current_file_idx {
                continue;
            }
            let path = file.display_path();

            // File header (only when shown — same gate as the renderer).
            if !self.is_single_file_view {
                self.line_annotations
                    .push(AnnotatedLine::FileHeader { file_idx });
            }

            // If reviewed, skip all content for this file. Single-file
            // view ignores the reviewed-collapse since the user
            // explicitly focused this file.
            if self.session.is_file_reviewed(path) && !self.is_single_file_view {
                continue;
            }

            // File comments
            if let Some(review) = self.session.files.get(path) {
                for (comment_idx, comment) in review.file_comments.iter().enumerate() {
                    let comment_lines =
                        Self::comment_display_lines(comment, self.diff_state.viewport_width);
                    for _ in 0..comment_lines {
                        self.line_annotations.push(AnnotatedLine::FileComment {
                            file_idx,
                            comment_idx,
                        });
                    }
                }
            }

            if file.is_binary || file.hunks.is_empty() {
                self.line_annotations
                    .push(AnnotatedLine::BinaryOrEmpty { file_idx });
            } else {
                // Get line comments for this file
                let line_comments = self
                    .session
                    .files
                    .get(path)
                    .map(|r| &r.line_comments)
                    .cloned()
                    .unwrap_or_default();

                for (hunk_idx, hunk) in file.hunks.iter().enumerate() {
                    // Calculate gap before this hunk
                    let prev_hunk = if hunk_idx > 0 {
                        file.hunks.get(hunk_idx - 1)
                    } else {
                        None
                    };
                    let gap = calculate_gap(
                        prev_hunk.map(|h| (&h.new_start, &h.new_count)),
                        hunk.new_start,
                    );

                    let gap_id = GapId { file_idx, hunk_idx };

                    if gap > 0 && self.should_render_gap_before_hunk(file_idx, hunk_idx) {
                        let top_len = self.expanded_top.get(&gap_id).map_or(0, |v| v.len());
                        let bot_len = self.expanded_bottom.get(&gap_id).map_or(0, |v| v.len());
                        let remaining = (gap as usize).saturating_sub(top_len + bot_len);
                        let is_top_of_file = hunk_idx == 0;

                        // Sequential line_idx counter across top + bottom
                        let mut ctx_idx = 0;

                        // --- Top expanded lines (↓ direction) ---
                        for _ in 0..top_len {
                            self.line_annotations.push(AnnotatedLine::ExpandedContext {
                                gap_id: gap_id.clone(),
                                line_idx: ctx_idx,
                            });
                            ctx_idx += 1;
                        }

                        // --- Expanders / hidden lines ---
                        if remaining > 0 {
                            if is_top_of_file {
                                // Top-of-file: HiddenLines (if > batch) + ↑
                                if remaining > GAP_EXPAND_BATCH {
                                    self.line_annotations.push(AnnotatedLine::HiddenLines {
                                        gap_id: gap_id.clone(),
                                        count: remaining,
                                    });
                                }
                                self.line_annotations.push(AnnotatedLine::Expander {
                                    gap_id: gap_id.clone(),
                                    direction: ExpandDirection::Up,
                                });
                            } else if remaining >= GAP_EXPAND_BATCH {
                                // Between-hunk, large: ↓ + HiddenLines + ↑
                                self.line_annotations.push(AnnotatedLine::Expander {
                                    gap_id: gap_id.clone(),
                                    direction: ExpandDirection::Down,
                                });
                                self.line_annotations.push(AnnotatedLine::HiddenLines {
                                    gap_id: gap_id.clone(),
                                    count: remaining,
                                });
                                self.line_annotations.push(AnnotatedLine::Expander {
                                    gap_id: gap_id.clone(),
                                    direction: ExpandDirection::Up,
                                });
                            } else {
                                // Between-hunk, small: merged ↕
                                self.line_annotations.push(AnnotatedLine::Expander {
                                    gap_id: gap_id.clone(),
                                    direction: ExpandDirection::Both,
                                });
                            }
                        }

                        // --- Bottom expanded lines (↑ direction) ---
                        for _ in 0..bot_len {
                            self.line_annotations.push(AnnotatedLine::ExpandedContext {
                                gap_id: gap_id.clone(),
                                line_idx: ctx_idx,
                            });
                            ctx_idx += 1;
                        }
                    }

                    // Hunk header
                    self.line_annotations
                        .push(AnnotatedLine::HunkHeader { file_idx, hunk_idx });
                    if self.is_hunk_reviewed(file_idx, hunk_idx) {
                        continue;
                    }

                    // Diff lines - handle differently based on view mode
                    match self.diff_view_mode {
                        DiffViewMode::Unified => {
                            Self::build_unified_diff_annotations(
                                &mut self.line_annotations,
                                file_idx,
                                hunk_idx,
                                &hunk.lines,
                                &line_comments,
                                path,
                                &self.forge_review_threads,
                                &remote_index,
                                self.diff_state.viewport_width,
                            );
                        }
                        DiffViewMode::SideBySide => {
                            Self::build_side_by_side_annotations(
                                &mut self.line_annotations,
                                file_idx,
                                hunk_idx,
                                &hunk.lines,
                                &line_comments,
                                path,
                                &self.forge_review_threads,
                                &remote_index,
                                self.diff_state.viewport_width,
                            );
                        }
                    }
                }

                // End-of-file gap (after all hunks, not for deleted files)
                if file.status != FileStatus::Deleted
                    && let Some(last_hunk) = file.hunks.last()
                {
                    let eof_start = last_hunk.new_start + last_hunk.new_count;
                    if let Some(&total) = self.file_line_count_cache.get(&file_idx)
                        && eof_start <= total
                    {
                        let gap = (total - eof_start + 1) as usize;
                        let eof_gap_id = GapId {
                            file_idx,
                            hunk_idx: file.hunks.len(),
                        };

                        let top_len = self.expanded_top.get(&eof_gap_id).map_or(0, |v| v.len());
                        let bot_len = self.expanded_bottom.get(&eof_gap_id).map_or(0, |v| v.len());
                        let remaining = gap.saturating_sub(top_len + bot_len);

                        let mut ctx_idx = 0;

                        // Top expanded lines (↓ direction)
                        for _ in 0..top_len {
                            self.line_annotations.push(AnnotatedLine::ExpandedContext {
                                gap_id: eof_gap_id.clone(),
                                line_idx: ctx_idx,
                            });
                            ctx_idx += 1;
                        }

                        // Expanders / hidden lines
                        if remaining > 0 {
                            self.line_annotations.push(AnnotatedLine::Expander {
                                gap_id: eof_gap_id.clone(),
                                direction: ExpandDirection::Down,
                            });
                            if remaining > GAP_EXPAND_BATCH {
                                self.line_annotations.push(AnnotatedLine::HiddenLines {
                                    gap_id: eof_gap_id.clone(),
                                    count: remaining,
                                });
                            }
                        }

                        // Bottom expanded lines (↑ direction)
                        for _ in 0..bot_len {
                            self.line_annotations.push(AnnotatedLine::ExpandedContext {
                                gap_id: eof_gap_id.clone(),
                                line_idx: ctx_idx,
                            });
                            ctx_idx += 1;
                        }
                    }
                }
            }

            // Spacing line
            self.line_annotations.push(AnnotatedLine::Spacing);
        }
    }

    fn push_comments(
        annotations: &mut Vec<AnnotatedLine>,
        file_idx: usize,
        line_no: Option<u32>,
        line_comments: &std::collections::HashMap<u32, Vec<crate::model::Comment>>,
        side: LineSide,
        viewport_width: usize,
    ) {
        let Some(ln) = line_no else {
            return;
        };

        let Some(comments) = line_comments.get(&ln) else {
            return;
        };

        for (idx, comment) in comments.iter().enumerate() {
            let matches_side =
                comment.side == Some(side) || (side == LineSide::New && comment.side.is_none());

            if !matches_side {
                continue;
            }

            let comment_lines = Self::comment_display_lines(comment, viewport_width);
            for _ in 0..comment_lines {
                annotations.push(AnnotatedLine::LineComment {
                    file_idx,
                    line: ln,
                    comment_idx: idx,
                    side,
                });
            }
        }
    }

    /// Per-file map of `(line, side)` -> indices into `forge_review_threads`.
    /// Sides use the `RemoteCommentSide` mapping: `Right` -> `LineSide::New`,
    /// `Left` -> `LineSide::Old`.
    fn build_remote_thread_index(&self) -> RemoteThreadIndex {
        use crate::forge::remote_comments::RemoteCommentSide;
        let mut by_file: std::collections::HashMap<
            String,
            std::collections::HashMap<(u32, LineSide), Vec<usize>>,
        > = std::collections::HashMap::new();
        let visibility = self.session.remote_comments_visibility;

        for (thread_idx, thread) in self.forge_review_threads.iter().enumerate() {
            if visibility.render_decision(thread).is_none() {
                continue;
            }
            let Some(line) = thread.line else { continue };
            let side = match thread.side {
                RemoteCommentSide::Right => LineSide::New,
                RemoteCommentSide::Left => LineSide::Old,
            };
            by_file
                .entry(thread.path.clone())
                .or_default()
                .entry((line, side))
                .or_default()
                .push(thread_idx);
        }

        RemoteThreadIndex { by_file }
    }

    fn push_remote_threads(
        annotations: &mut Vec<AnnotatedLine>,
        threads: &[crate::forge::remote_comments::RemoteReviewThread],
        index: &RemoteThreadIndex,
        path: &std::path::Path,
        line: u32,
        side: LineSide,
    ) {
        let Some(file_index) = index.by_file.get(path.to_string_lossy().as_ref()) else {
            return;
        };
        let Some(thread_indices) = file_index.get(&(line, side)) else {
            return;
        };
        for thread_idx in thread_indices {
            if let Some(thread) = threads.get(*thread_idx) {
                let n = crate::forge::remote_comments::thread_display_lines(thread);
                for _ in 0..n {
                    annotations.push(AnnotatedLine::RemoteThreadLine {
                        thread_idx: *thread_idx,
                    });
                }
            }
        }
    }

    /// Build annotations for unified diff mode (one annotation per diff line)
    #[allow(clippy::too_many_arguments)]
    fn build_unified_diff_annotations(
        annotations: &mut Vec<AnnotatedLine>,
        file_idx: usize,
        hunk_idx: usize,
        lines: &[crate::model::DiffLine],
        line_comments: &std::collections::HashMap<u32, Vec<crate::model::Comment>>,
        path: &std::path::Path,
        remote_threads: &[crate::forge::remote_comments::RemoteReviewThread],
        remote_index: &RemoteThreadIndex,
        viewport_width: usize,
    ) {
        for (line_idx, diff_line) in lines.iter().enumerate() {
            annotations.push(AnnotatedLine::DiffLine {
                file_idx,
                hunk_idx,
                line_idx,
                old_lineno: diff_line.old_lineno,
                new_lineno: diff_line.new_lineno,
            });

            // Line comments on old side (delete lines)
            if let Some(old_ln) = diff_line.old_lineno {
                Self::push_comments(
                    annotations,
                    file_idx,
                    Some(old_ln),
                    line_comments,
                    LineSide::Old,
                    viewport_width,
                );
                Self::push_remote_threads(
                    annotations,
                    remote_threads,
                    remote_index,
                    path,
                    old_ln,
                    LineSide::Old,
                );
            }

            // Line comments on new side (added/context lines)
            if let Some(new_ln) = diff_line.new_lineno {
                Self::push_comments(
                    annotations,
                    file_idx,
                    Some(new_ln),
                    line_comments,
                    LineSide::New,
                    viewport_width,
                );
                Self::push_remote_threads(
                    annotations,
                    remote_threads,
                    remote_index,
                    path,
                    new_ln,
                    LineSide::New,
                );
            }
        }
    }

    /// Build annotations for side-by-side diff mode, pairing deletions and additions into aligned rows.
    #[allow(clippy::too_many_arguments)]
    fn build_side_by_side_annotations(
        annotations: &mut Vec<AnnotatedLine>,
        file_idx: usize,
        hunk_idx: usize,
        lines: &[crate::model::DiffLine],
        line_comments: &std::collections::HashMap<u32, Vec<crate::model::Comment>>,
        path: &std::path::Path,
        remote_threads: &[crate::forge::remote_comments::RemoteReviewThread],
        remote_index: &RemoteThreadIndex,
        viewport_width: usize,
    ) {
        let mut i = 0;
        while i < lines.len() {
            let diff_line = &lines[i];

            match diff_line.origin {
                LineOrigin::Context => {
                    annotations.push(AnnotatedLine::SideBySideLine {
                        file_idx,
                        hunk_idx,
                        del_line_idx: Some(i),
                        add_line_idx: Some(i),
                        old_lineno: diff_line.old_lineno,
                        new_lineno: diff_line.new_lineno,
                    });

                    Self::push_comments(
                        annotations,
                        file_idx,
                        diff_line.new_lineno,
                        line_comments,
                        LineSide::New,
                        viewport_width,
                    );
                    if let Some(new_ln) = diff_line.new_lineno {
                        Self::push_remote_threads(
                            annotations,
                            remote_threads,
                            remote_index,
                            path,
                            new_ln,
                            LineSide::New,
                        );
                    }

                    i += 1
                }

                LineOrigin::Deletion => {
                    // Find consecutive deletions
                    let del_start = i;
                    let mut del_end = i + 1;
                    while del_end < lines.len() && lines[del_end].origin == LineOrigin::Deletion {
                        del_end += 1;
                    }

                    // Find consecutive additions following deletions
                    let add_start = del_end;
                    let mut add_end = add_start;
                    while add_end < lines.len() && lines[add_end].origin == LineOrigin::Addition {
                        add_end += 1;
                    }

                    let del_count = del_end - del_start;
                    let add_count = add_end - add_start;
                    let max_lines = del_count.max(add_count);

                    for offset in 0..max_lines {
                        let del_idx = if offset < del_count {
                            Some(del_start + offset)
                        } else {
                            None
                        };
                        let add_idx = if offset < add_count {
                            Some(add_start + offset)
                        } else {
                            None
                        };

                        let old_lineno = del_idx.and_then(|idx| lines[idx].old_lineno);
                        let new_lineno = add_idx.and_then(|idx| lines[idx].new_lineno);

                        annotations.push(AnnotatedLine::SideBySideLine {
                            file_idx,
                            hunk_idx,
                            del_line_idx: del_idx,
                            add_line_idx: add_idx,
                            old_lineno,
                            new_lineno,
                        });

                        Self::push_comments(
                            annotations,
                            file_idx,
                            old_lineno,
                            line_comments,
                            LineSide::Old,
                            viewport_width,
                        );
                        if let Some(old_ln) = old_lineno {
                            Self::push_remote_threads(
                                annotations,
                                remote_threads,
                                remote_index,
                                path,
                                old_ln,
                                LineSide::Old,
                            );
                        }
                        Self::push_comments(
                            annotations,
                            file_idx,
                            new_lineno,
                            line_comments,
                            LineSide::New,
                            viewport_width,
                        );
                        if let Some(new_ln) = new_lineno {
                            Self::push_remote_threads(
                                annotations,
                                remote_threads,
                                remote_index,
                                path,
                                new_ln,
                                LineSide::New,
                            );
                        }
                    }

                    i = add_end;
                }
                LineOrigin::Addition => {
                    annotations.push(AnnotatedLine::SideBySideLine {
                        file_idx,
                        hunk_idx,
                        del_line_idx: None,
                        add_line_idx: Some(i),
                        old_lineno: None,
                        new_lineno: diff_line.new_lineno,
                    });

                    Self::push_comments(
                        annotations,
                        file_idx,
                        diff_line.new_lineno,
                        line_comments,
                        LineSide::New,
                        viewport_width,
                    );
                    if let Some(new_ln) = diff_line.new_lineno {
                        Self::push_remote_threads(
                            annotations,
                            remote_threads,
                            remote_index,
                            path,
                            new_ln,
                            LineSide::New,
                        );
                    }

                    i += 1;
                }
            }
        }
    }

    /// What the cursor is on in a gap region
    pub fn get_gap_at_cursor(&self) -> Option<GapCursorHit> {
        let target = self.diff_state.cursor_line;
        match self.line_annotations.get(target) {
            Some(AnnotatedLine::Expander { gap_id, direction }) => {
                Some(GapCursorHit::Expander(gap_id.clone(), *direction))
            }
            Some(AnnotatedLine::HiddenLines { gap_id, .. }) => {
                Some(GapCursorHit::HiddenLines(gap_id.clone()))
            }
            Some(AnnotatedLine::ExpandedContext { gap_id, .. }) => {
                Some(GapCursorHit::ExpandedContent(gap_id.clone()))
            }
            _ => None,
        }
    }

    fn ensure_valid_tree_selection(&mut self) {
        use std::path::Path;

        let visible_items = self.build_visible_items();
        if visible_items.is_empty() {
            self.file_list_state.select(0);
            return;
        }

        let current_file_idx = self.diff_state.current_file_idx;
        let file_visible = visible_items.iter().any(|item| {
            matches!(item, FileTreeItem::File { file_idx, .. } if *file_idx == current_file_idx)
        });

        if file_visible {
            if let Some(tree_idx) = self.file_idx_to_tree_idx(current_file_idx) {
                self.file_list_state.select(tree_idx);
            }
        } else {
            if let Some(file) = self.diff_files.get(current_file_idx) {
                let file_path = file.display_path();
                let mut current = file_path.parent();
                while let Some(parent) = current {
                    if parent != Path::new("") {
                        let parent_str = parent.to_string_lossy().to_string();
                        for (tree_idx, item) in visible_items.iter().enumerate() {
                            if let FileTreeItem::Directory { path, .. } = item
                                && *path == parent_str
                            {
                                self.file_list_state.select(tree_idx);
                                return;
                            }
                        }
                    }
                    current = parent.parent();
                }
            }
            self.file_list_state.select(0);
        }
    }

    pub fn build_visible_items(&self) -> Vec<FileTreeItem> {
        use std::path::Path;

        let mut items = Vec::new();
        let mut seen_dirs: HashSet<String> = HashSet::new();

        for (file_idx, file) in self.diff_files.iter().enumerate() {
            let path = file.display_path();

            let mut ancestors: Vec<String> = Vec::new();
            let mut current = path.parent();
            while let Some(parent) = current {
                if parent != Path::new("") {
                    ancestors.push(parent.to_string_lossy().to_string());
                }
                current = parent.parent();
            }
            ancestors.reverse();

            let mut visible = true;
            for (depth, dir) in ancestors.iter().enumerate() {
                if !seen_dirs.contains(dir) && visible {
                    let expanded = self.expanded_dirs.contains(dir);
                    items.push(FileTreeItem::Directory {
                        path: dir.clone(),
                        depth,
                        expanded,
                    });
                    seen_dirs.insert(dir.clone());
                }

                if !self.expanded_dirs.contains(dir) {
                    visible = false;
                }
            }

            if visible {
                items.push(FileTreeItem::File {
                    file_idx,
                    depth: ancestors.len(),
                });
            }
        }

        items
    }

    pub fn get_selected_tree_item(&self) -> Option<FileTreeItem> {
        let visible_items = self.build_visible_items();
        let selected_idx = self.file_list_state.selected();
        visible_items.get(selected_idx).cloned()
    }
}

#[cfg(test)]
mod persistence_merge_tests {
    use super::*;

    const SOME_HASH: u64 = 0xabc;

    fn test_session() -> ReviewSession {
        let mut session = ReviewSession::new(
            PathBuf::from("/repo"),
            "abc1234".to_string(),
            Some("main".to_string()),
            SessionDiffSource::WorkingTree,
        );
        session.add_file(
            PathBuf::from("src/main.rs"),
            FileStatus::Modified,
            SOME_HASH,
        );
        session
    }

    fn comment(id: &str, content: &str) -> Comment {
        let mut comment = Comment::new(content.to_string(), CommentType::Note, None);
        comment.id = id.to_string();
        comment
    }

    fn push_file_comment(session: &mut ReviewSession, id: &str, content: &str) {
        session
            .get_file_mut(&PathBuf::from("src/main.rs"))
            .unwrap()
            .file_comments
            .push(comment(id, content));
    }

    fn file_comment_ids(session: &ReviewSession) -> Vec<String> {
        session
            .files
            .get(&PathBuf::from("src/main.rs"))
            .unwrap()
            .file_comments
            .iter()
            .map(|comment| comment.id.clone())
            .collect()
    }

    #[test]
    fn should_merge_external_comment_without_losing_local_comment() {
        let base = test_session();
        let mut current = base.clone();
        let mut latest = base.clone();

        push_file_comment(&mut current, "local", "from tui");
        push_file_comment(&mut latest, "external", "from cli");

        let changed = App::merge_external_session_changes(&mut current, &base, &latest);

        assert_eq!(changed, 1);
        assert_eq!(file_comment_ids(&current), vec!["local", "external"]);
    }

    #[test]
    fn should_not_resurrect_locally_deleted_comment_when_disk_is_unchanged() {
        let mut base = test_session();
        push_file_comment(&mut base, "deleted", "old");
        let mut current = base.clone();
        current
            .get_file_mut(&PathBuf::from("src/main.rs"))
            .unwrap()
            .file_comments
            .clear();
        let latest = base.clone();

        let changed = App::merge_external_session_changes(&mut current, &base, &latest);

        assert_eq!(changed, 0);
        assert!(file_comment_ids(&current).is_empty());
    }

    #[test]
    fn should_apply_external_edit_when_comment_is_unchanged_locally() {
        let mut base = test_session();
        push_file_comment(&mut base, "same", "old");
        let mut current = base.clone();
        let mut latest = base.clone();
        latest
            .get_file_mut(&PathBuf::from("src/main.rs"))
            .unwrap()
            .file_comments[0]
            .content = "new".to_string();

        let changed = App::merge_external_session_changes(&mut current, &base, &latest);

        assert_eq!(changed, 1);
        assert_eq!(
            current
                .files
                .get(&PathBuf::from("src/main.rs"))
                .unwrap()
                .file_comments[0]
                .content,
            "new"
        );
    }
}

#[cfg(test)]
mod tree_tests {
    use super::*;
    use crate::model::{DiffFile, FileStatus};

    fn make_file(path: &str) -> DiffFile {
        DiffFile {
            old_path: None,
            new_path: Some(PathBuf::from(path)),
            status: FileStatus::Modified,
            hunks: vec![],
            is_binary: false,
            is_too_large: false,
            is_commit_message: false,
            content_hash: 0,
        }
    }

    struct TreeTestHarness {
        diff_files: Vec<DiffFile>,
        expanded_dirs: HashSet<String>,
    }

    impl TreeTestHarness {
        fn new(paths: &[&str]) -> Self {
            Self {
                diff_files: paths.iter().map(|p| make_file(p)).collect(),
                expanded_dirs: HashSet::new(),
            }
        }

        fn expand_all(&mut self) {
            use std::path::Path;
            for file in &self.diff_files {
                let path = file.display_path();
                let mut current = path.parent();
                while let Some(parent) = current {
                    if parent != Path::new("") {
                        self.expanded_dirs
                            .insert(parent.to_string_lossy().to_string());
                    }
                    current = parent.parent();
                }
            }
        }

        fn collapse_all(&mut self) {
            self.expanded_dirs.clear();
        }

        fn toggle(&mut self, dir: &str) {
            if self.expanded_dirs.contains(dir) {
                self.expanded_dirs.remove(dir);
            } else {
                self.expanded_dirs.insert(dir.to_string());
            }
        }

        fn build_visible_items(&self) -> Vec<FileTreeItem> {
            use std::path::Path;
            let mut items = Vec::new();
            let mut seen_dirs: HashSet<String> = HashSet::new();

            for (file_idx, file) in self.diff_files.iter().enumerate() {
                let path = file.display_path();
                let mut ancestors: Vec<String> = Vec::new();
                let mut current = path.parent();
                while let Some(parent) = current {
                    if parent != Path::new("") {
                        ancestors.push(parent.to_string_lossy().to_string());
                    }
                    current = parent.parent();
                }
                ancestors.reverse();

                let mut visible = true;
                for (depth, dir) in ancestors.iter().enumerate() {
                    if !seen_dirs.contains(dir) && visible {
                        let expanded = self.expanded_dirs.contains(dir);
                        items.push(FileTreeItem::Directory {
                            path: dir.clone(),
                            depth,
                            expanded,
                        });
                        seen_dirs.insert(dir.clone());
                    }
                    if !self.expanded_dirs.contains(dir) {
                        visible = false;
                    }
                }

                if visible {
                    items.push(FileTreeItem::File {
                        file_idx,
                        depth: ancestors.len(),
                    });
                }
            }
            items
        }

        fn visible_file_count(&self) -> usize {
            self.build_visible_items()
                .iter()
                .filter(|i| matches!(i, FileTreeItem::File { .. }))
                .count()
        }

        fn visible_dir_count(&self) -> usize {
            self.build_visible_items()
                .iter()
                .filter(|i| matches!(i, FileTreeItem::Directory { .. }))
                .count()
        }
    }

    #[test]
    fn test_expand_all_shows_all_files() {
        let mut h = TreeTestHarness::new(&["src/ui/app.rs", "src/ui/help.rs", "src/main.rs"]);
        h.expand_all();

        assert_eq!(h.visible_file_count(), 3);
    }

    #[test]
    fn test_collapse_all_hides_all_files() {
        let mut h = TreeTestHarness::new(&["src/ui/app.rs", "src/main.rs"]);
        h.expand_all();
        h.collapse_all();

        assert_eq!(h.visible_file_count(), 0);
        assert_eq!(h.visible_dir_count(), 1); // only "src" visible
    }

    #[test]
    fn test_collapse_parent_hides_nested_dirs() {
        let mut h = TreeTestHarness::new(&["src/ui/components/button.rs"]);
        h.expand_all();
        assert_eq!(h.visible_dir_count(), 3); // src, src/ui, src/ui/components

        h.toggle("src");
        let items = h.build_visible_items();
        assert_eq!(items.len(), 1); // only collapsed "src" dir
        assert!(matches!(
            &items[0],
            FileTreeItem::Directory {
                expanded: false,
                ..
            }
        ));
    }

    #[test]
    fn test_root_files_always_visible() {
        let mut h = TreeTestHarness::new(&["README.md", "Cargo.toml"]);
        h.collapse_all();

        assert_eq!(h.visible_file_count(), 2);
    }

    #[test]
    fn test_tree_depth_correct() {
        let mut h = TreeTestHarness::new(&["a/b/c/file.rs"]);
        h.expand_all();

        let items = h.build_visible_items();
        assert!(matches!(&items[0], FileTreeItem::Directory { depth: 0, path, .. } if path == "a"));
        assert!(
            matches!(&items[1], FileTreeItem::Directory { depth: 1, path, .. } if path == "a/b")
        );
        assert!(
            matches!(&items[2], FileTreeItem::Directory { depth: 2, path, .. } if path == "a/b/c")
        );
        assert!(matches!(&items[3], FileTreeItem::File { depth: 3, .. }));
    }

    #[test]
    fn test_toggle_expands_collapsed_dir() {
        let mut h = TreeTestHarness::new(&["src/main.rs"]);
        h.collapse_all();
        assert_eq!(h.visible_file_count(), 0);

        h.toggle("src");
        assert_eq!(h.visible_file_count(), 1);
    }

    #[test]
    fn test_sibling_dirs_independent() {
        let mut h = TreeTestHarness::new(&["src/app.rs", "tests/test.rs"]);
        h.expand_all();
        h.toggle("src"); // collapse src

        assert_eq!(h.visible_file_count(), 1); // only tests/test.rs
    }
}

#[cfg(test)]
mod commit_selection_tests {
    use super::*;
    use crate::model::FileStatus;
    use crate::vcs::traits::VcsType;

    struct DummyVcs {
        info: VcsInfo,
    }

    impl VcsBackend for DummyVcs {
        fn info(&self) -> &VcsInfo {
            &self.info
        }

        fn get_working_tree_diff(&self, _highlighter: &SyntaxHighlighter) -> Result<Vec<DiffFile>> {
            Err(TuicrError::NoChanges)
        }

        fn fetch_context_lines(
            &self,
            _file_path: &Path,
            _file_status: FileStatus,
            _ref_commit: Option<&str>,
            _start_line: u32,
            _end_line: u32,
        ) -> Result<Vec<DiffLine>> {
            Ok(Vec::new())
        }

        fn file_line_count(
            &self,
            _file_path: &Path,
            _file_status: FileStatus,
            _ref_commit: Option<&str>,
        ) -> Result<u32> {
            Ok(0)
        }
    }

    fn build_app(commit_list: Vec<CommitInfo>) -> App {
        let vcs_info = VcsInfo {
            root_path: PathBuf::from("/tmp"),
            head_commit: "head".to_string(),
            branch_name: Some("main".to_string()),
            vcs_type: VcsType::Git,
        };
        let session = ReviewSession::new(
            vcs_info.root_path.clone(),
            vcs_info.head_commit.clone(),
            vcs_info.branch_name.clone(),
            SessionDiffSource::WorkingTree,
        );

        App::build(
            Box::new(DummyVcs {
                info: vcs_info.clone(),
            }),
            vcs_info,
            Theme::dark(),
            None,
            false,
            Vec::new(),
            session,
            DiffSource::WorkingTree,
            InputMode::CommitSelect,
            commit_list,
            None,
            None,
        )
        .expect("failed to build test app")
    }

    fn normal_commit(id: &str) -> CommitInfo {
        CommitInfo {
            id: id.to_string(),
            short_id: id.to_string(),
            branch_name: None,
            summary: "Test commit".to_string(),
            body: None,
            author: "Test".to_string(),
            time: Utc::now(),
        }
    }

    #[test]
    fn special_commit_count_counts_leading_special_entries() {
        let app = build_app(vec![
            App::staged_commit_entry(),
            App::unstaged_commit_entry(),
            normal_commit("abc123"),
        ]);

        assert_eq!(app.special_commit_count(), 2);
    }

    #[test]
    fn special_commit_count_ignores_non_leading_special_entries() {
        let app = build_app(vec![normal_commit("abc123"), App::staged_commit_entry()]);

        assert_eq!(app.special_commit_count(), 0);
    }

    #[test]
    fn toggle_commit_selection_from_all_selected_selects_only_cursor() {
        for cursor in 0..3 {
            let mut app = build_app(vec![
                normal_commit("abc123"),
                normal_commit("def456"),
                normal_commit("789abc"),
            ]);
            app.commit_selection_range = Some((0, 2));
            app.commit_list_cursor = cursor;

            app.toggle_commit_selection();

            assert_eq!(app.commit_selection_range, Some((cursor, cursor)));
        }
    }

    #[test]
    fn toggle_commit_selection_keeps_partial_range_shrink_behavior() {
        let mut app = build_app(vec![
            normal_commit("abc123"),
            normal_commit("def456"),
            normal_commit("789abc"),
        ]);
        app.commit_selection_range = Some((0, 1));
        app.commit_list_cursor = 0;

        app.toggle_commit_selection();

        assert_eq!(app.commit_selection_range, Some((1, 1)));
    }
}

#[cfg(test)]
mod target_selector_tests {
    use super::*;
    use crate::forge::selector::PullRequestsTab;
    use crate::forge::traits::PullRequestSummary;
    use crate::model::FileStatus;
    use crate::vcs::traits::{VcsChangeStatus, VcsType};

    struct DummyVcs {
        info: VcsInfo,
        commits: Vec<CommitInfo>,
    }

    impl VcsBackend for DummyVcs {
        fn info(&self) -> &VcsInfo {
            &self.info
        }

        fn get_working_tree_diff(&self, _highlighter: &SyntaxHighlighter) -> Result<Vec<DiffFile>> {
            Err(TuicrError::NoChanges)
        }

        fn fetch_context_lines(
            &self,
            _file_path: &Path,
            _file_status: FileStatus,
            _ref_commit: Option<&str>,
            _start_line: u32,
            _end_line: u32,
        ) -> Result<Vec<DiffLine>> {
            Ok(Vec::new())
        }

        fn get_change_status(&self) -> Result<VcsChangeStatus> {
            Ok(VcsChangeStatus {
                staged: false,
                unstaged: false,
            })
        }

        fn get_recent_commits(&self, offset: usize, limit: usize) -> Result<Vec<CommitInfo>> {
            Ok(self
                .commits
                .iter()
                .skip(offset)
                .take(limit)
                .cloned()
                .collect())
        }

        fn file_line_count(
            &self,
            _file_path: &Path,
            _file_status: FileStatus,
            _ref_commit: Option<&str>,
        ) -> Result<u32> {
            Ok(0)
        }
    }

    fn build_app() -> App {
        build_app_with_commits(Vec::new())
    }

    fn build_app_with_commits(commits: Vec<CommitInfo>) -> App {
        let vcs_info = VcsInfo {
            root_path: PathBuf::from("/tmp"),
            head_commit: "head".to_string(),
            branch_name: Some("main".to_string()),
            vcs_type: VcsType::Git,
        };
        let session = ReviewSession::new(
            vcs_info.root_path.clone(),
            vcs_info.head_commit.clone(),
            vcs_info.branch_name.clone(),
            SessionDiffSource::WorkingTree,
        );

        App::build(
            Box::new(DummyVcs {
                info: vcs_info.clone(),
                commits,
            }),
            vcs_info,
            Theme::dark(),
            None,
            false,
            Vec::new(),
            session,
            DiffSource::WorkingTree,
            InputMode::Normal,
            Vec::new(),
            None,
            None,
        )
        .expect("failed to build test app")
    }

    #[test]
    fn should_cycle_comment_type_on_tab_action() {
        let mut app = build_app();
        app.enter_comment_mode(false, None);
        assert_eq!(app.input_mode, InputMode::Comment);
        assert_eq!(app.comment_type.id(), "note");

        app.cycle_comment_type();
        assert_eq!(app.comment_type.id(), "suggestion");

        app.cycle_comment_type();
        assert_eq!(app.comment_type.id(), "issue");

        app.cycle_comment_type();
        assert_eq!(app.comment_type.id(), "praise");

        app.cycle_comment_type();
        assert_eq!(app.comment_type.id(), "note");
    }

    fn dummy_commit(id: &str) -> CommitInfo {
        CommitInfo {
            id: id.to_string(),
            short_id: id.to_string(),
            branch_name: None,
            summary: format!("commit {id}"),
            body: None,
            author: "tester".to_string(),
            time: Utc::now(),
        }
    }

    fn test_pr_details(number: u64, title: &str) -> crate::forge::traits::PullRequestDetails {
        crate::forge::traits::PullRequestDetails {
            repository: ForgeRepository::github("github.com", "agavra", "tuicr"),
            number,
            title: title.to_string(),
            url: format!("https://github.com/agavra/tuicr/pull/{number}"),
            state: "OPEN".to_string(),
            is_draft: false,
            author: Some("alice".to_string()),
            head_ref_name: "feat".to_string(),
            base_ref_name: "main".to_string(),
            head_sha: "abcdef0123456789".to_string(),
            base_sha: "1234567890abcdef".to_string(),
            body: String::new(),
            updated_at: None,
            closed: false,
            merged_at: None,
            diff_start_sha: None,
        }
    }

    struct FakeForgeBackend {
        details: crate::forge::traits::PullRequestDetails,
        patch: String,
        commits: Vec<crate::forge::traits::PullRequestCommit>,
        range_patch: Option<String>,
    }

    impl FakeForgeBackend {
        fn open_pr_details(
            details: crate::forge::traits::PullRequestDetails,
            patch: String,
        ) -> Self {
            Self {
                details,
                patch,
                commits: Vec::new(),
                range_patch: None,
            }
        }
    }

    impl crate::forge::traits::ForgeBackend for FakeForgeBackend {
        fn list_pull_requests(
            &self,
            _query: crate::forge::traits::PullRequestListQuery,
        ) -> Result<crate::forge::traits::PagedPullRequests> {
            unimplemented!("not used in this test")
        }
        fn get_pull_request(
            &self,
            _target: crate::forge::traits::PullRequestTarget,
        ) -> Result<crate::forge::traits::PullRequestDetails> {
            Ok(self.details.clone())
        }
        fn get_pull_request_diff(
            &self,
            _pr: &crate::forge::traits::PullRequestDetails,
        ) -> Result<String> {
            Ok(self.patch.clone())
        }
        fn fetch_file_lines(
            &self,
            _request: crate::forge::traits::ForgeFileLinesRequest,
        ) -> Result<Vec<crate::model::DiffLine>> {
            Ok(Vec::new())
        }
        fn list_review_threads(
            &self,
            _pr: &crate::forge::traits::PullRequestDetails,
        ) -> Result<Vec<crate::forge::remote_comments::RemoteReviewThread>> {
            Ok(Vec::new())
        }
        fn list_pull_request_commits(
            &self,
            _pr: &crate::forge::traits::PullRequestDetails,
        ) -> Result<Vec<crate::forge::traits::PullRequestCommit>> {
            Ok(self.commits.clone())
        }
        fn get_pull_request_commit_range_diff(
            &self,
            _pr: &crate::forge::traits::PullRequestDetails,
            _start_sha: &str,
            _end_sha: &str,
        ) -> Result<String> {
            Ok(self
                .range_patch
                .clone()
                .unwrap_or_else(|| self.patch.clone()))
        }
        fn create_review(
            &self,
            _pr: &crate::forge::traits::PullRequestDetails,
            _request: crate::forge::traits::CreateReviewRequest<'_>,
        ) -> Result<crate::forge::traits::GhCreateReviewResponse> {
            unimplemented!("FakeForgeBackend does not implement create_review")
        }
    }

    fn sample_pr(number: u64, title: &str) -> PullRequestSummary {
        PullRequestSummary {
            repository: ForgeRepository::github("github.com", "agavra", "tuicr"),
            number,
            title: title.to_string(),
            author: Some("alice".to_string()),
            head_ref_name: "feat".to_string(),
            base_ref_name: "main".to_string(),
            updated_at: None,
            url: format!("https://github.com/agavra/tuicr/pull/{number}"),
            state: "OPEN".to_string(),
            is_draft: false,
        }
    }

    #[test]
    fn should_default_to_local_tab_after_build() {
        // given / when
        let app = build_app();
        // then
        assert_eq!(app.target_tab, TargetTab::Local);
        assert!(!app.pr_filter_editing());
    }

    #[test]
    fn should_cycle_between_local_and_pull_requests_on_tab_keypress() {
        // given
        let mut app = build_app();
        app.forge_repository = Some(ForgeRepository::github("github.com", "agavra", "tuicr"));
        app.pr_tab = PullRequestsTab::new(app.forge_repository.clone());
        // when
        app.cycle_target_tab(true);
        // then
        assert_eq!(app.target_tab, TargetTab::PullRequests);
        // when
        app.cycle_target_tab(false);
        // then
        assert_eq!(app.target_tab, TargetTab::Local);
    }

    #[test]
    fn should_transition_pr_tab_to_loading_on_first_visit() {
        // given
        let mut app = build_app();
        app.forge_repository = Some(ForgeRepository::github("github.com", "agavra", "tuicr"));
        app.pr_tab = PullRequestsTab::new(app.forge_repository.clone());
        // when
        app.cycle_target_tab(true);
        // then — a background fetch is in flight
        assert!(app.pr_tab.is_loading());
        assert!(app.pr_load_rx.is_some());
        // The spawned thread holds a backend that may fail without a real
        // `gh` binary; cancel by dropping the receiver to avoid touching it.
        app.pr_load_rx = None;
    }

    #[test]
    fn should_keep_pr_tab_disabled_when_no_forge_remote() {
        // given
        let mut app = build_app();
        // No forge_repository set up; default new app has None.
        // when
        app.cycle_target_tab(true);
        // then
        assert_eq!(app.target_tab, TargetTab::PullRequests);
        assert!(matches!(app.pr_tab, PullRequestsTab::Disabled { .. }));
        assert!(app.pr_load_rx.is_none());
    }

    #[test]
    fn should_set_filter_after_typing_and_committing() {
        // given
        let mut app = build_app();
        app.forge_repository = Some(ForgeRepository::github("github.com", "agavra", "tuicr"));
        app.pr_tab = PullRequestsTab::new(app.forge_repository.clone());
        app.pr_tab.start_initial_load();
        app.pr_tab.apply_initial_load(Ok((
            vec![sample_pr(125, "Forge"), sample_pr(148, "Review")],
            false,
        )));
        app.target_tab = TargetTab::PullRequests;
        // when
        app.begin_pr_filter();
        app.pr_filter_insert_char('f');
        app.pr_filter_insert_char('o');
        app.commit_pr_filter();
        // then
        assert!(!app.pr_filter_editing());
        assert_eq!(app.pr_tab.view().rows.len(), 1);
        assert_eq!(app.pr_tab.view().rows[0].summary.number, 125);
    }

    #[test]
    fn should_discard_filter_draft_on_cancel() {
        // given
        let mut app = build_app();
        app.forge_repository = Some(ForgeRepository::github("github.com", "agavra", "tuicr"));
        app.pr_tab = PullRequestsTab::new(app.forge_repository.clone());
        app.pr_tab.start_initial_load();
        app.pr_tab
            .apply_initial_load(Ok((vec![sample_pr(1, "alpha")], false)));
        app.target_tab = TargetTab::PullRequests;
        // when
        app.begin_pr_filter();
        app.pr_filter_insert_char('z');
        app.cancel_pr_filter();
        // then
        assert!(!app.pr_filter_editing());
        assert_eq!(app.pr_tab.view().filter, "");
    }

    #[test]
    fn should_enter_pr_mode_when_opening_pr_via_fake_backend() {
        // given a selector with a single PR row and a fake forge backend
        let mut app = build_app();
        app.forge_repository = Some(ForgeRepository::github("github.com", "agavra", "tuicr"));
        app.pr_tab = PullRequestsTab::new(app.forge_repository.clone());
        app.pr_tab.start_initial_load();
        let summary = sample_pr(42, "answer");
        app.pr_tab
            .apply_initial_load(Ok((vec![summary.clone()], false)));
        app.target_tab = TargetTab::PullRequests;
        let backend = Box::new(FakeForgeBackend::open_pr_details(
            test_pr_details(42, "answer"),
            crate::forge::github::gh::tests_fixture::SIMPLE_PATCH.to_string(),
        ));
        // when
        app.open_pr_with_backend(&summary, backend, None).unwrap();
        // then
        assert!(matches!(app.diff_source, DiffSource::PullRequest(_)));
        if let DiffSource::PullRequest(pr) = &app.diff_source {
            assert_eq!(pr.key.number, 42);
            assert_eq!(pr.title, "answer");
            assert_eq!(pr.head_ref_name, "feat");
            assert_eq!(pr.base_ref_name, "main");
            assert_eq!(pr.key.head_sha, "abcdef0123456789");
            assert_eq!(pr.base_sha, "1234567890abcdef");
        }
        // and the session is keyed by the PR
        assert!(app.session.pr_session_key.is_some());
        // and PR diff files were parsed
        assert_eq!(app.diff_files.len(), 1);
        // and the forge backend is wired for context expansion / submit
        assert!(app.forge_backend.is_some());
    }

    fn sample_pr_commit(oid: &str, summary: &str) -> crate::forge::traits::PullRequestCommit {
        crate::forge::traits::PullRequestCommit {
            oid: oid.to_string(),
            short_oid: oid.chars().take(7).collect(),
            summary: summary.to_string(),
            author: "Alice".to_string(),
            timestamp: None,
        }
    }

    fn two_hunk_patch() -> &'static str {
        r#"diff --git a/src/lib.rs b/src/lib.rs
index 1111111..2222222 100644
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -1 +1 @@
-old one
+new one
@@ -10 +10 @@
-old two
+new two
"#
    }

    fn first_hunk_patch() -> &'static str {
        r#"diff --git a/src/lib.rs b/src/lib.rs
index 1111111..2222222 100644
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -1 +1 @@
-old one
+new one
"#
    }

    #[test]
    fn should_populate_inline_selector_when_pr_has_multiple_commits() {
        // given a PR open path where the forge returns 3 commits
        let mut app = build_app();
        app.forge_repository = Some(ForgeRepository::github("github.com", "agavra", "tuicr"));
        app.pr_tab = PullRequestsTab::new(app.forge_repository.clone());
        app.pr_tab.start_initial_load();
        let summary = sample_pr(42, "multi-commit");
        app.pr_tab
            .apply_initial_load(Ok((vec![summary.clone()], false)));
        let mut backend = FakeForgeBackend::open_pr_details(
            test_pr_details(42, "multi-commit"),
            crate::forge::github::gh::tests_fixture::SIMPLE_PATCH.to_string(),
        );
        // Forge returns oldest-first; pr_open reverses to newest-first.
        backend.commits = vec![
            sample_pr_commit("aaaaaaa1111", "first"),
            sample_pr_commit("bbbbbbb2222", "second"),
            sample_pr_commit("ccccccc3333", "third"),
        ];
        // when
        app.open_pr_with_backend(&summary, Box::new(backend), None)
            .unwrap();
        // then — selector is visible and pr_commits is in newest-first order.
        assert!(app.show_commit_selector, "selector should be visible");
        assert_eq!(app.pr_commits.len(), 3);
        assert_eq!(app.pr_commits[0].summary, "third");
        assert_eq!(app.review_commits.len(), 3);
        // and — default selection covers all commits.
        assert_eq!(app.commit_selection_range, Some((0, 2)));
    }

    #[test]
    fn should_hide_inline_selector_for_single_commit_pr() {
        // given a PR with exactly one commit
        let mut app = build_app();
        app.forge_repository = Some(ForgeRepository::github("github.com", "agavra", "tuicr"));
        app.pr_tab = PullRequestsTab::new(app.forge_repository.clone());
        app.pr_tab.start_initial_load();
        let summary = sample_pr(42, "solo");
        app.pr_tab
            .apply_initial_load(Ok((vec![summary.clone()], false)));
        let mut backend = FakeForgeBackend::open_pr_details(
            test_pr_details(42, "solo"),
            crate::forge::github::gh::tests_fixture::SIMPLE_PATCH.to_string(),
        );
        backend.commits = vec![sample_pr_commit("aaaaaaa1111", "only")];
        // when
        app.open_pr_with_backend(&summary, Box::new(backend), None)
            .unwrap();
        // then
        assert!(!app.show_commit_selector);
        assert!(app.commit_list.is_empty());
        assert_eq!(app.commit_selection_range, None);
    }

    #[test]
    fn should_resolve_pr_range_to_parent_sha_and_head_sha() {
        // given a multi-commit PR open with the middle commit selected
        let mut app = build_app();
        app.forge_repository = Some(ForgeRepository::github("github.com", "agavra", "tuicr"));
        app.pr_tab = PullRequestsTab::new(app.forge_repository.clone());
        app.pr_tab.start_initial_load();
        let summary = sample_pr(42, "ranges");
        app.pr_tab
            .apply_initial_load(Ok((vec![summary.clone()], false)));
        let mut backend = FakeForgeBackend::open_pr_details(
            test_pr_details(42, "ranges"),
            crate::forge::github::gh::tests_fixture::SIMPLE_PATCH.to_string(),
        );
        backend.commits = vec![
            sample_pr_commit("first11", "first"),
            sample_pr_commit("middle2", "middle"),
            sample_pr_commit("last333", "last"),
        ];
        app.open_pr_with_backend(&summary, Box::new(backend), None)
            .unwrap();
        // After open: pr_commits = [last, middle, first] (newest-first).
        // Select only the middle commit (index 1).
        app.commit_selection_range = Some((1, 1));
        // when
        let pair = app.pr_range_sha_pair();
        // then — start = parent (first), end = newest selected (middle).
        assert_eq!(pair, Some(("first11".to_string(), "middle2".to_string())));
    }

    #[test]
    fn should_resolve_pr_range_to_pr_base_when_oldest_commit_selected() {
        // given a multi-commit PR with only the oldest commit selected
        let mut app = build_app();
        app.forge_repository = Some(ForgeRepository::github("github.com", "agavra", "tuicr"));
        app.pr_tab = PullRequestsTab::new(app.forge_repository.clone());
        app.pr_tab.start_initial_load();
        let summary = sample_pr(42, "base");
        app.pr_tab
            .apply_initial_load(Ok((vec![summary.clone()], false)));
        let mut backend = FakeForgeBackend::open_pr_details(
            test_pr_details(42, "base"),
            crate::forge::github::gh::tests_fixture::SIMPLE_PATCH.to_string(),
        );
        backend.commits = vec![
            sample_pr_commit("aaa", "first"),
            sample_pr_commit("bbb", "second"),
        ];
        app.open_pr_with_backend(&summary, Box::new(backend), None)
            .unwrap();
        // pr_commits = [second, first]. Select only the oldest (index 1).
        app.commit_selection_range = Some((1, 1));
        // when
        let pair = app.pr_range_sha_pair();
        // then — start falls back to the PR's base_sha.
        let expected_base = test_pr_details(42, "base").base_sha;
        assert_eq!(pair, Some((expected_base, "aaa".to_string())));
    }

    #[test]
    fn should_preserve_hunk_marks_hidden_by_pr_range_diff() {
        let mut app = build_app();
        app.forge_repository = Some(ForgeRepository::github("github.com", "agavra", "tuicr"));
        app.pr_tab = PullRequestsTab::new(app.forge_repository.clone());
        app.pr_tab.start_initial_load();
        let summary = sample_pr(42, "ranges");
        app.pr_tab
            .apply_initial_load(Ok((vec![summary.clone()], false)));
        let mut backend = FakeForgeBackend::open_pr_details(
            test_pr_details(42, "ranges"),
            two_hunk_patch().to_string(),
        );
        backend.commits = vec![
            sample_pr_commit("aaa1111", "first"),
            sample_pr_commit("bbb2222", "second"),
        ];
        app.open_pr_with_backend(&summary, Box::new(backend), None)
            .unwrap();
        let path = app.diff_files[0].display_path().clone();
        let hidden_key = app.diff_files[0].hunk_review_key(1).unwrap();
        app.session
            .get_file_mut(&path)
            .unwrap()
            .toggle_hunk_reviewed(hidden_key.clone());

        let request = PrRangeReloadRequest {
            repository: ForgeRepository::github("github.com", "agavra", "tuicr"),
            pr_number: 42,
            head_sha: "abcdef0123456789".to_string(),
            start_sha: "aaa1111".to_string(),
            end_sha: "bbb2222".to_string(),
            range: (1, 1),
            started_at: Instant::now(),
            anchor: None,
        };
        app.finish_pr_range_reload(&request, first_hunk_patch())
            .unwrap();

        assert!(app.session.is_hunk_reviewed(&path, &hidden_key));
    }

    #[test]
    fn should_warn_when_opening_closed_pr() {
        // given
        let mut app = build_app();
        let summary = sample_pr(42, "old");
        let mut details = test_pr_details(42, "old");
        details.state = "CLOSED".to_string();
        details.closed = true;
        let backend = Box::new(FakeForgeBackend::open_pr_details(
            details,
            crate::forge::github::gh::tests_fixture::SIMPLE_PATCH.to_string(),
        ));
        // when
        app.open_pr_with_backend(&summary, backend, None).unwrap();
        // then — warning message surfaces the read-only state
        let msg = app.message.as_ref().expect("expected warning message");
        assert!(msg.content.contains("closed"), "got: {:?}", msg.content);
        assert!(msg.content.contains("read-only"), "got: {:?}", msg.content);
        // and the diff source reflects the closed state
        if let DiffSource::PullRequest(pr) = &app.diff_source {
            assert!(pr.is_read_only());
            assert_eq!(pr.read_only_reason(), Some("closed"));
        } else {
            panic!("expected PullRequest diff source");
        }
    }

    #[test]
    fn should_surface_pr_open_error_into_selector_state() {
        // given a backend that fails at get_pull_request
        let mut app = build_app();
        app.forge_repository = Some(ForgeRepository::github("github.com", "agavra", "tuicr"));
        app.pr_tab = PullRequestsTab::new(app.forge_repository.clone());
        app.pr_tab.start_initial_load();
        let summary = sample_pr(42, "boom");
        app.pr_tab
            .apply_initial_load(Ok((vec![summary.clone()], false)));
        app.target_tab = TargetTab::PullRequests;
        let backend = Box::new(FailingForgeBackend);
        // when
        let result = app.open_pr_with_backend(&summary, backend, None);
        // then
        assert!(result.is_err());
        // diff source did not switch
        assert!(matches!(app.diff_source, DiffSource::WorkingTree));
    }

    #[test]
    fn should_route_context_expansion_to_forge_provider_in_pr_mode() {
        // given an app in PR mode with a counting fake backend
        let mut app = build_app();
        let summary = sample_pr(7, "ctx");
        let backend = Box::new(FakeForgeBackend::open_pr_details(
            test_pr_details(7, "ctx"),
            crate::forge::github::gh::tests_fixture::SIMPLE_PATCH.to_string(),
        ));
        app.open_pr_with_backend(&summary, backend, None).unwrap();
        // when we ask for a context provider
        // (we can't easily trigger a real gap expansion without setting up
        //  the full diff state, so instead we assert by construction)
        let provider = app.context_provider();
        // then — explicitly: the provider is the forge variant. We probe by
        // calling fetch with a Modified file and asserting the forge backend
        // recorded a head-side request via its trait method. The
        // FakeForgeBackend just returns empty; we're verifying routing.
        let res = provider
            .fetch_context_lines(
                None,
                Some(&PathBuf::from("src/lib.rs")),
                FileStatus::Modified,
                1,
                3,
            )
            .unwrap();
        // The fake forge returns empty by default — the *call* succeeded
        // (no error from a VCS backend would have meant VCS routing). The
        // key signal: this didn't go through the VCS backend (DummyVcs
        // doesn't implement fetch_context_lines and would have panicked).
        assert!(res.is_empty());
    }

    #[test]
    fn should_switch_session_when_pr_head_advances_on_reload() {
        // given an app already in PR mode at head A
        let mut app = build_app();
        let summary = sample_pr(42, "head-a");
        let mut details_a = test_pr_details(42, "head-a");
        details_a.head_sha = "aaaaaaaaaaaaaaaa".to_string();
        let backend_a = Box::new(FakeForgeBackend::open_pr_details(
            details_a.clone(),
            crate::forge::github::gh::tests_fixture::SIMPLE_PATCH.to_string(),
        ));
        app.open_pr_with_backend(&summary, backend_a, None).unwrap();
        let old_session_id = app.session.id.clone();
        // when reloading with a backend that reports head B
        let mut details_b = details_a.clone();
        details_b.head_sha = "bbbbbbbbbbbbbbbb".to_string();
        details_b.title = "head-b".to_string();
        let backend_b = Box::new(FakeForgeBackend::open_pr_details(
            details_b.clone(),
            crate::forge::github::gh::tests_fixture::SIMPLE_PATCH.to_string(),
        ));
        let head_changed = app
            .reload_pull_request_with_backend(backend_b, None)
            .unwrap();
        // then — the session swap happened
        assert!(head_changed);
        if let DiffSource::PullRequest(pr) = &app.diff_source {
            assert_eq!(pr.key.head_sha, "bbbbbbbbbbbbbbbb");
            assert_eq!(pr.title, "head-b");
        } else {
            panic!("expected PullRequest diff source");
        }
        // and the session changed (new session, not the old one)
        assert_ne!(app.session.id, old_session_id);
    }

    #[test]
    fn should_keep_session_when_pr_head_unchanged_on_reload() {
        // given an app in PR mode
        let mut app = build_app();
        let summary = sample_pr(42, "same");
        let details = test_pr_details(42, "same");
        let backend = Box::new(FakeForgeBackend::open_pr_details(
            details.clone(),
            crate::forge::github::gh::tests_fixture::SIMPLE_PATCH.to_string(),
        ));
        app.open_pr_with_backend(&summary, backend, None).unwrap();
        let session_id_before = app.session.id.clone();
        // when reloading with the same head
        let backend2 = Box::new(FakeForgeBackend::open_pr_details(
            details,
            crate::forge::github::gh::tests_fixture::SIMPLE_PATCH.to_string(),
        ));
        let changed = app
            .reload_pull_request_with_backend(backend2, None)
            .unwrap();
        // then
        assert!(!changed);
        assert_eq!(app.session.id, session_id_before);
    }

    struct FailingForgeBackend;

    impl crate::forge::traits::ForgeBackend for FailingForgeBackend {
        fn list_pull_requests(
            &self,
            _q: crate::forge::traits::PullRequestListQuery,
        ) -> Result<crate::forge::traits::PagedPullRequests> {
            unimplemented!()
        }
        fn get_pull_request(
            &self,
            _target: crate::forge::traits::PullRequestTarget,
        ) -> Result<crate::forge::traits::PullRequestDetails> {
            Err(crate::error::TuicrError::Forge(
                "simulated network failure".to_string(),
            ))
        }
        fn get_pull_request_diff(
            &self,
            _pr: &crate::forge::traits::PullRequestDetails,
        ) -> Result<String> {
            unreachable!()
        }
        fn fetch_file_lines(
            &self,
            _req: crate::forge::traits::ForgeFileLinesRequest,
        ) -> Result<Vec<crate::model::DiffLine>> {
            Ok(Vec::new())
        }
        fn list_review_threads(
            &self,
            _pr: &crate::forge::traits::PullRequestDetails,
        ) -> Result<Vec<crate::forge::remote_comments::RemoteReviewThread>> {
            Ok(Vec::new())
        }
        fn list_pull_request_commits(
            &self,
            _pr: &crate::forge::traits::PullRequestDetails,
        ) -> Result<Vec<crate::forge::traits::PullRequestCommit>> {
            Ok(Vec::new())
        }
        fn get_pull_request_commit_range_diff(
            &self,
            _pr: &crate::forge::traits::PullRequestDetails,
            _start_sha: &str,
            _end_sha: &str,
        ) -> Result<String> {
            unreachable!()
        }
        fn create_review(
            &self,
            _pr: &crate::forge::traits::PullRequestDetails,
            _request: crate::forge::traits::CreateReviewRequest<'_>,
        ) -> Result<crate::forge::traits::GhCreateReviewResponse> {
            unimplemented!()
        }
    }

    #[test]
    fn should_apply_initial_load_event_to_pr_tab() {
        // given
        let mut app = build_app();
        app.forge_repository = Some(ForgeRepository::github("github.com", "agavra", "tuicr"));
        app.pr_tab = PullRequestsTab::new(app.forge_repository.clone());
        app.pr_tab.start_initial_load();
        let (tx, rx) = std::sync::mpsc::channel();
        app.pr_load_rx = Some(rx);
        tx.send(PrLoadEvent::Initial {
            canonical: ForgeRepository::github("github.com", "agavra", "tuicr"),
            result: Ok((vec![sample_pr(7, "lucky")], false)),
        })
        .unwrap();
        drop(tx);
        // when
        app.poll_pr_load_events();
        // then
        assert!(app.pr_load_rx.is_none());
        assert_eq!(app.pr_tab.view().rows.len(), 1);
        assert_eq!(app.pr_tab.view().rows[0].summary.number, 7);
    }

    #[test]
    fn should_promote_app_forge_repository_to_canonical_on_initial_load() {
        // given — origin is a fork; canonical from the background thread is upstream
        let origin = ForgeRepository::github("github.com", "agavra", "slatedb");
        let canonical = ForgeRepository::github("github.com", "slatedb", "slatedb");
        let mut app = build_app();
        app.forge_repository = Some(origin.clone());
        app.pr_tab = PullRequestsTab::new(Some(origin));
        app.pr_tab.start_initial_load();
        assert!(!app.canonical_resolved);
        let (tx, rx) = std::sync::mpsc::channel();
        app.pr_load_rx = Some(rx);
        tx.send(PrLoadEvent::Initial {
            canonical: canonical.clone(),
            result: Ok((Vec::new(), false)),
        })
        .unwrap();
        drop(tx);
        // when
        app.poll_pr_load_events();
        // then
        assert_eq!(app.forge_repository.as_ref(), Some(&canonical));
        assert!(app.canonical_resolved);
    }

    #[test]
    fn should_quit_on_q_when_only_reviewed_files_dirty() {
        // given: a session dirtied only by a reviewed-file marker (no comments)
        let mut app = build_app();
        let path = PathBuf::from("src/main.rs");
        app.session.add_file(path.clone(), FileStatus::Modified, 0);
        app.session.get_file_mut(&path).unwrap().reviewed = true;
        app.dirty = true;
        assert!(!app.session.has_comments());
        app.command_buffer = "q".to_string();

        // when
        crate::handler::handle_command_action(&mut app, crate::input::Action::SubmitInput);

        // then: `:q` discards the reviewed-only state and quits, no `:q!` needed
        assert!(app.should_quit);
        assert!(!app.dirty);
        assert!(
            !matches!(
                app.message.as_ref().map(|m| &m.message_type),
                Some(MessageType::Error)
            ),
            "should not surface a no-write error"
        );
    }

    #[test]
    fn should_block_q_when_unsaved_comments_exist() {
        // given: a session with an unsaved comment
        let mut app = build_app();
        let path = PathBuf::from("src/main.rs");
        app.session.add_file(path.clone(), FileStatus::Modified, 0);
        app.session
            .get_file_mut(&path)
            .unwrap()
            .add_file_comment(crate::model::Comment::new(
                "needs work".to_string(),
                crate::model::CommentType::Note,
                None,
            ));
        app.dirty = true;
        assert!(app.session.has_comments());
        app.command_buffer = "q".to_string();

        // when
        crate::handler::handle_command_action(&mut app, crate::input::Action::SubmitInput);

        // then: the guard still requires `:q!`
        assert!(!app.should_quit);
        assert!(app.dirty);
        assert_eq!(
            app.message.as_ref().map(|m| m.message_type.clone()),
            Some(MessageType::Error)
        );
    }

    #[test]
    fn should_open_pr_selector_on_prs_command() {
        // given
        let mut app = build_app();
        app.forge_repository = Some(ForgeRepository::github("github.com", "agavra", "tuicr"));
        app.pr_tab = PullRequestsTab::new(app.forge_repository.clone());
        app.command_buffer = "prs".to_string();
        // when
        crate::handler::handle_command_action(&mut app, crate::input::Action::SubmitInput);
        // then
        assert_eq!(app.target_tab, TargetTab::PullRequests);
        assert_eq!(app.input_mode, InputMode::CommitSelect);
        // Cancel the background fetch handle to avoid surprising real `gh` calls.
        app.pr_load_rx = None;
    }

    #[test]
    fn should_open_local_selector_on_targets_command() {
        // given
        let mut app = build_app_with_commits(vec![dummy_commit("abc")]);
        app.command_buffer = "targets".to_string();
        // when
        crate::handler::handle_command_action(&mut app, crate::input::Action::SubmitInput);
        // then
        assert_eq!(app.target_tab, TargetTab::Local);
        assert_eq!(app.input_mode, InputMode::CommitSelect);
    }

    #[test]
    fn should_treat_commits_as_alias_for_local_target_selector() {
        // given
        let mut app = build_app_with_commits(vec![dummy_commit("abc")]);
        app.command_buffer = "commits".to_string();
        // when
        crate::handler::handle_command_action(&mut app, crate::input::Action::SubmitInput);
        // then
        assert_eq!(app.target_tab, TargetTab::Local);
        assert_eq!(app.input_mode, InputMode::CommitSelect);
    }

    #[test]
    fn should_complete_command_when_only_one_candidate_matches() {
        // given
        let mut app = build_app();
        app.input_mode = InputMode::Command;
        app.command_buffer = "vers".to_string();
        // when
        crate::handler::handle_command_action(&mut app, crate::input::Action::CompleteCommand);
        // then
        assert_eq!(app.command_buffer, "version");
        assert!(app.command_completion.is_none());
    }

    #[test]
    fn should_extend_to_common_command_prefix_before_cycling() {
        // given
        let mut app = build_app();
        app.input_mode = InputMode::Command;
        app.command_buffer = "su".to_string();
        // when
        crate::handler::handle_command_action(&mut app, crate::input::Action::CompleteCommand);
        // then
        assert_eq!(app.command_buffer, "submit");
        assert!(app.command_completion.is_none());
    }

    #[test]
    fn should_cycle_forward_through_command_matches() {
        // given
        let mut app = build_app();
        app.input_mode = InputMode::Command;
        app.command_buffer = "submit".to_string();
        // when
        crate::handler::handle_command_action(&mut app, crate::input::Action::CompleteCommand);
        // then
        assert_eq!(app.command_buffer, "submit comment");
        assert_eq!(
            app.command_completion
                .as_ref()
                .map(|completion| completion.prefix.as_str()),
            Some("submit")
        );
        // when
        crate::handler::handle_command_action(&mut app, crate::input::Action::CompleteCommand);
        // then
        assert_eq!(app.command_buffer, "submit approve");
    }

    #[test]
    fn should_cycle_backward_through_command_matches() {
        // given
        let mut app = build_app();
        app.input_mode = InputMode::Command;
        app.command_buffer = "submit".to_string();
        // when
        crate::handler::handle_command_action(
            &mut app,
            crate::input::Action::CompleteCommandReverse,
        );
        // then
        assert_eq!(app.command_buffer, "submit draft");
    }

    #[test]
    fn should_leave_unknown_command_completion_unchanged() {
        // given
        let mut app = build_app();
        app.input_mode = InputMode::Command;
        app.command_buffer = "zz".to_string();
        // when
        crate::handler::handle_command_action(&mut app, crate::input::Action::CompleteCommand);
        // then
        assert_eq!(app.command_buffer, "zz");
        assert!(app.command_completion.is_none());
    }

    #[test]
    fn should_clear_command_completion_state_after_manual_edit() {
        // given
        let mut app = build_app();
        app.input_mode = InputMode::Command;
        app.command_buffer = "set ".to_string();
        crate::handler::handle_command_action(&mut app, crate::input::Action::CompleteCommand);
        assert_eq!(app.command_buffer, "set wrap");
        assert!(app.command_completion.is_some());
        // when
        crate::handler::handle_command_action(&mut app, crate::input::Action::InsertChar('x'));
        // then
        assert_eq!(app.command_buffer, "set wrapx");
        assert!(app.command_completion.is_none());
    }

    // -- async PR open spinner tests -----------------------------------------

    fn loaded_pr_tab(pr_list: Vec<PullRequestSummary>) -> PullRequestsTab {
        let mut tab = PullRequestsTab::new(Some(ForgeRepository::github(
            "github.com",
            "agavra",
            "tuicr",
        )));
        tab.start_initial_load();
        tab.apply_initial_load(Ok((pr_list, false)));
        tab
    }

    #[test]
    fn should_set_pr_open_state_and_spawn_when_pressing_enter_on_a_pr_row() {
        // given a loaded PR tab and no in-flight open
        let mut app = build_app();
        app.forge_repository = Some(ForgeRepository::github("github.com", "agavra", "tuicr"));
        app.pr_tab = loaded_pr_tab(vec![sample_pr(42, "boom")]);
        app.target_tab = TargetTab::PullRequests;
        // when
        let handled = app.pr_tab_select();
        // then
        assert!(handled);
        assert!(app.pr_open_state.is_some());
        let state = app.pr_open_state.as_ref().unwrap();
        assert_eq!(state.pr_number, 42);
        // Drop the receiver so the spawned thread's tx send is a no-op
        // when it completes (the real `gh` call would block; this test
        // does not wait for it).
        app.pr_open_rx = None;
        app.pr_open_state = None;
    }

    #[test]
    fn should_be_a_noop_when_pressing_enter_during_an_in_flight_open() {
        // given an in-flight open marker
        let mut app = build_app();
        app.forge_repository = Some(ForgeRepository::github("github.com", "agavra", "tuicr"));
        app.pr_tab = loaded_pr_tab(vec![sample_pr(7, "ctx"), sample_pr(8, "next")]);
        app.target_tab = TargetTab::PullRequests;
        app.pr_open_state = Some(crate::app::PrOpenRequest {
            repository: ForgeRepository::github("github.com", "agavra", "tuicr"),
            pr_number: 7,
            started_at: std::time::Instant::now(),
        });
        // (no pr_open_rx is fine — the function never touches it on this path)
        // when — Enter on a different row
        if let crate::forge::selector::PullRequestsTab::Loaded { cursor, .. } = &mut app.pr_tab {
            *cursor = 1;
        }
        let handled = app.pr_tab_select();
        // then — handled but state unchanged (no new spawn for #8)
        assert!(handled);
        let state = app.pr_open_state.as_ref().unwrap();
        assert_eq!(state.pr_number, 7);
    }

    #[test]
    fn should_clear_pr_open_state_on_cancel() {
        // given
        let mut app = build_app();
        app.pr_open_state = Some(crate::app::PrOpenRequest {
            repository: ForgeRepository::github("github.com", "agavra", "tuicr"),
            pr_number: 11,
            started_at: std::time::Instant::now(),
        });
        // when
        let cancelled = app.cancel_pr_open();
        // then
        assert!(cancelled);
        assert!(app.pr_open_state.is_none());
        assert!(app.pr_open_rx.is_none());
    }

    #[test]
    fn should_return_false_when_cancelling_with_no_in_flight_open() {
        // given
        let mut app = build_app();
        // when
        let cancelled = app.cancel_pr_open();
        // then
        assert!(!cancelled);
    }

    #[test]
    fn should_surface_pr_open_error_to_message_bar_when_done_event_carries_error() {
        // given an app waiting on a synthetic open
        let mut app = build_app();
        app.forge_repository = Some(ForgeRepository::github("github.com", "agavra", "tuicr"));
        app.pr_tab = loaded_pr_tab(vec![sample_pr(42, "boom")]);
        app.target_tab = TargetTab::PullRequests;
        let request = crate::app::PrOpenRequest {
            repository: ForgeRepository::github("github.com", "agavra", "tuicr"),
            pr_number: 42,
            started_at: std::time::Instant::now(),
        };
        app.pr_open_state = Some(request.clone());
        let (tx, rx) = std::sync::mpsc::channel();
        app.pr_open_rx = Some(rx);
        tx.send(crate::app::PrOpenEvent::Done {
            request,
            result: Err("auth failed".to_string()),
        })
        .unwrap();
        // when
        app.poll_pr_open_events();
        // then — open state cleared, error surfaced to message bar, PR
        // list is intact so the user can retry / pick a different PR
        assert!(app.pr_open_state.is_none());
        assert!(app.pr_open_rx.is_none());
        assert!(matches!(app.pr_tab, PullRequestsTab::Loaded { .. }));
        let msg = app
            .message
            .as_ref()
            .expect("expected an error message on the bar");
        assert!(matches!(msg.message_type, MessageType::Error));
        assert!(msg.content.contains("auth failed"), "got {msg:?}");
    }

    #[test]
    fn should_ignore_stale_done_event_after_cancel() {
        // given an open was cancelled but the thread's send arrived anyway
        let mut app = build_app();
        app.forge_repository = Some(ForgeRepository::github("github.com", "agavra", "tuicr"));
        app.pr_tab = loaded_pr_tab(vec![sample_pr(42, "boom")]);
        app.target_tab = TargetTab::PullRequests;
        let stale_request = crate::app::PrOpenRequest {
            repository: ForgeRepository::github("github.com", "agavra", "tuicr"),
            pr_number: 42,
            started_at: std::time::Instant::now(),
        };
        let (tx, rx) = std::sync::mpsc::channel();
        app.pr_open_rx = Some(rx);
        // pr_open_state is None — the user already cancelled.
        tx.send(crate::app::PrOpenEvent::Done {
            request: stale_request,
            result: Err("would-have-failed".to_string()),
        })
        .unwrap();
        // when
        app.poll_pr_open_events();
        // then — the stale error does not produce a user-visible message
        assert!(matches!(app.pr_tab, PullRequestsTab::Loaded { .. }));
        assert!(
            app.message.is_none()
                || !app
                    .message
                    .as_ref()
                    .unwrap()
                    .content
                    .contains("would-have-failed")
        );
    }

    #[test]
    fn should_cancel_in_flight_open_when_pressing_esc_in_selector() {
        // given
        let mut app = build_app();
        app.forge_repository = Some(ForgeRepository::github("github.com", "agavra", "tuicr"));
        app.pr_tab = loaded_pr_tab(vec![sample_pr(99, "x")]);
        app.target_tab = TargetTab::PullRequests;
        app.pr_open_state = Some(crate::app::PrOpenRequest {
            repository: ForgeRepository::github("github.com", "agavra", "tuicr"),
            pr_number: 99,
            started_at: std::time::Instant::now(),
        });
        // when
        crate::handler::handle_commit_select_action(&mut app, crate::input::Action::ExitMode);
        // then
        assert!(app.pr_open_state.is_none());
    }

    // -----------------------------------------------------------------
    // Remote review threads (PR 4)
    // -----------------------------------------------------------------

    use crate::forge::remote_comments::{
        PrCommentsVisibility, RemoteCommentSide, RemoteReviewComment, RemoteReviewThread,
    };

    struct ThreadAwareForgeBackend {
        details: crate::forge::traits::PullRequestDetails,
        patch: String,
        threads: Vec<RemoteReviewThread>,
        calls: std::cell::Cell<u32>,
    }

    impl ThreadAwareForgeBackend {
        fn new(
            details: crate::forge::traits::PullRequestDetails,
            patch: String,
            threads: Vec<RemoteReviewThread>,
        ) -> Self {
            Self {
                details,
                patch,
                threads,
                calls: std::cell::Cell::new(0),
            }
        }
    }

    impl crate::forge::traits::ForgeBackend for ThreadAwareForgeBackend {
        fn list_pull_requests(
            &self,
            _q: crate::forge::traits::PullRequestListQuery,
        ) -> Result<crate::forge::traits::PagedPullRequests> {
            unimplemented!()
        }
        fn get_pull_request(
            &self,
            _t: crate::forge::traits::PullRequestTarget,
        ) -> Result<crate::forge::traits::PullRequestDetails> {
            Ok(self.details.clone())
        }
        fn get_pull_request_diff(
            &self,
            _p: &crate::forge::traits::PullRequestDetails,
        ) -> Result<String> {
            Ok(self.patch.clone())
        }
        fn fetch_file_lines(
            &self,
            _r: crate::forge::traits::ForgeFileLinesRequest,
        ) -> Result<Vec<crate::model::DiffLine>> {
            Ok(Vec::new())
        }
        fn list_review_threads(
            &self,
            _pr: &crate::forge::traits::PullRequestDetails,
        ) -> Result<Vec<RemoteReviewThread>> {
            self.calls.set(self.calls.get() + 1);
            Ok(self.threads.clone())
        }
        fn list_pull_request_commits(
            &self,
            _pr: &crate::forge::traits::PullRequestDetails,
        ) -> Result<Vec<crate::forge::traits::PullRequestCommit>> {
            Ok(Vec::new())
        }
        fn get_pull_request_commit_range_diff(
            &self,
            _pr: &crate::forge::traits::PullRequestDetails,
            _start_sha: &str,
            _end_sha: &str,
        ) -> Result<String> {
            unreachable!()
        }
        fn create_review(
            &self,
            _pr: &crate::forge::traits::PullRequestDetails,
            _request: crate::forge::traits::CreateReviewRequest<'_>,
        ) -> Result<crate::forge::traits::GhCreateReviewResponse> {
            unimplemented!()
        }
    }

    fn sample_thread(line: u32, body: &str, resolved: bool, outdated: bool) -> RemoteReviewThread {
        RemoteReviewThread {
            id: "T".to_string(),
            path: "src/lib.rs".to_string(),
            line: Some(line),
            side: RemoteCommentSide::Right,
            is_resolved: resolved,
            is_outdated: outdated,
            comments: vec![RemoteReviewComment {
                id: "C".to_string(),
                author: Some("alice".to_string()),
                body: body.to_string(),
                created_at: None,
                in_reply_to: None,
                url: "https://example.com/c".to_string(),
            }],
        }
    }

    #[test]
    fn should_populate_remote_threads_when_opening_pr_through_test_seam() {
        // given
        let mut app = build_app();
        let summary = sample_pr(42, "answer");
        let backend = Box::new(ThreadAwareForgeBackend::new(
            test_pr_details(42, "answer"),
            crate::forge::github::gh::tests_fixture::SIMPLE_PATCH.to_string(),
            vec![sample_thread(2, "remote body", false, false)],
        ));
        // when
        app.open_pr_with_backend(&summary, backend, None).unwrap();
        // then
        assert_eq!(app.forge_review_threads.len(), 1);
        assert_eq!(app.forge_review_threads[0].comments[0].body, "remote body");
        // default visibility is Unresolved on a fresh PR session
        assert_eq!(
            app.session.remote_comments_visibility,
            PrCommentsVisibility::Unresolved
        );
    }

    #[test]
    fn should_clear_remote_threads_without_refetch_when_setting_visibility_hide() {
        // given a PR open with one fetched thread
        let mut app = build_app();
        let summary = sample_pr(42, "answer");
        let backend = Box::new(ThreadAwareForgeBackend::new(
            test_pr_details(42, "answer"),
            crate::forge::github::gh::tests_fixture::SIMPLE_PATCH.to_string(),
            vec![sample_thread(2, "remote", false, false)],
        ));
        app.open_pr_with_backend(&summary, backend, None).unwrap();
        assert_eq!(app.forge_review_threads.len(), 1);
        // when — switch to hide
        let changed = app.set_remote_comments_visibility(PrCommentsVisibility::Hide);
        // then
        assert!(changed);
        assert_eq!(
            app.session.remote_comments_visibility,
            PrCommentsVisibility::Hide
        );
        // We don't drop the cache on visibility change — only filtering changes.
        // Switching back to Unresolved should restore the rendered comments
        // without making a new network call.
        assert_eq!(app.forge_review_threads.len(), 1);
    }

    #[test]
    fn should_route_comments_unresolved_command_through_command_handler() {
        use crate::handler::handle_command_action;
        use crate::input::Action;
        // given
        let mut app = build_app();
        let summary = sample_pr(42, "answer");
        let backend = Box::new(ThreadAwareForgeBackend::new(
            test_pr_details(42, "answer"),
            crate::forge::github::gh::tests_fixture::SIMPLE_PATCH.to_string(),
            vec![sample_thread(2, "remote", false, false)],
        ));
        app.open_pr_with_backend(&summary, backend, None).unwrap();
        // when — enter command mode then submit `:comments all`
        app.input_mode = crate::app::InputMode::Command;
        app.command_buffer = "comments all".to_string();
        handle_command_action(&mut app, Action::SubmitInput);
        // then
        assert_eq!(
            app.session.remote_comments_visibility,
            PrCommentsVisibility::All
        );
    }

    #[test]
    fn should_warn_when_comments_command_used_outside_pr_mode() {
        use crate::handler::handle_command_action;
        use crate::input::Action;
        // given — plain local working-tree session
        let mut app = build_app();
        app.input_mode = crate::app::InputMode::Command;
        app.command_buffer = "comments all".to_string();
        // when
        handle_command_action(&mut app, Action::SubmitInput);
        // then — visibility unchanged, a warning surfaced on the message bar
        assert_eq!(
            app.session.remote_comments_visibility,
            PrCommentsVisibility::Unresolved
        );
        let msg = app
            .message
            .as_ref()
            .expect("expected warning on message bar");
        assert!(matches!(msg.message_type, MessageType::Warning));
        assert!(
            msg.content.contains("PR mode"),
            "got message: {}",
            msg.content
        );
    }

    #[test]
    fn should_apply_remote_threads_event_when_relevant() {
        // given a PR session is open at head=`headsha`
        let mut app = build_app();
        let summary = sample_pr(42, "answer");
        let backend = Box::new(ThreadAwareForgeBackend::new(
            test_pr_details(42, "answer"),
            crate::forge::github::gh::tests_fixture::SIMPLE_PATCH.to_string(),
            // open with empty threads — we'll deliver via the channel
            Vec::new(),
        ));
        app.open_pr_with_backend(&summary, backend, None).unwrap();
        // simulate a background fetch that finished after open
        let (tx, rx) = std::sync::mpsc::channel();
        app.pr_threads_rx = Some(rx);
        app.forge_review_threads_loading = true;
        let pr_key = match &app.diff_source {
            DiffSource::PullRequest(pr) => pr.key.clone(),
            _ => panic!("expected PR mode"),
        };
        tx.send(crate::app::PrThreadsEvent::Done {
            repository: pr_key.repository.clone(),
            pr_number: pr_key.number,
            head_sha: pr_key.head_sha.clone(),
            threads: Ok(vec![sample_thread(2, "delayed", false, false)]),
            summaries: Ok(Vec::new()),
        })
        .unwrap();
        // when
        app.poll_pr_threads_events();
        // then
        assert!(!app.forge_review_threads_loading);
        assert_eq!(app.forge_review_threads.len(), 1);
        assert_eq!(app.forge_review_threads[0].comments[0].body, "delayed");
    }

    #[test]
    fn should_discard_stale_remote_threads_event_after_switching_pr() {
        // given a PR open, then user switches to a different PR while a
        // fetch is in flight
        let mut app = build_app();
        let summary = sample_pr(42, "answer");
        let backend = Box::new(ThreadAwareForgeBackend::new(
            test_pr_details(42, "answer"),
            crate::forge::github::gh::tests_fixture::SIMPLE_PATCH.to_string(),
            Vec::new(),
        ));
        app.open_pr_with_backend(&summary, backend, None).unwrap();
        // simulate a stale event from a different PR head
        let (tx, rx) = std::sync::mpsc::channel();
        app.pr_threads_rx = Some(rx);
        tx.send(crate::app::PrThreadsEvent::Done {
            repository: ForgeRepository::github("github.com", "agavra", "tuicr"),
            pr_number: 999,                         // wrong number
            head_sha: "definitely-not-this".into(), // wrong head
            threads: Ok(vec![sample_thread(2, "stale", false, false)]),
            summaries: Ok(Vec::new()),
        })
        .unwrap();
        // when
        app.poll_pr_threads_events();
        // then — stale result was dropped
        assert!(app.forge_review_threads.is_empty());
    }
}

#[cfg(test)]
mod scroll_tests {
    /// max_scroll_offset is simply total_lines - 1 (last line can be at top).
    fn calc_max_scroll(total_lines: usize) -> usize {
        total_lines.saturating_sub(1)
    }

    #[test]
    fn should_calculate_max_scroll() {
        // Last line can be scrolled to the top of the viewport
        assert_eq!(calc_max_scroll(103), 102);
        assert_eq!(calc_max_scroll(20), 19);
    }

    #[test]
    fn should_handle_small_content() {
        // Even with few lines, can scroll last line to top
        assert_eq!(calc_max_scroll(13), 12);
        assert_eq!(calc_max_scroll(1), 0);
    }

    #[test]
    fn should_handle_empty_content() {
        assert_eq!(calc_max_scroll(0), 0);
    }
}

#[cfg(test)]
mod scroll_behavior_tests {
    use super::*;
    use crate::model::FileStatus;
    use crate::vcs::traits::VcsType;

    struct DummyVcs {
        info: VcsInfo,
    }

    impl VcsBackend for DummyVcs {
        fn info(&self) -> &VcsInfo {
            &self.info
        }

        fn get_working_tree_diff(&self, _highlighter: &SyntaxHighlighter) -> Result<Vec<DiffFile>> {
            Err(TuicrError::NoChanges)
        }

        fn fetch_context_lines(
            &self,
            _file_path: &Path,
            _file_status: FileStatus,
            _ref_commit: Option<&str>,
            _start_line: u32,
            _end_line: u32,
        ) -> Result<Vec<DiffLine>> {
            Ok(Vec::new())
        }

        fn file_line_count(
            &self,
            _file_path: &Path,
            _file_status: FileStatus,
            _ref_commit: Option<&str>,
        ) -> Result<u32> {
            Ok(0)
        }
    }

    /// Build a test App with a single file containing `n` context lines.
    /// Total rendered lines = 1 (review header) + 1 (file header) + 1 (spacing)
    ///                       + 1 (hunk header) + n (diff lines) = n + 4.
    /// The viewport is set to `viewport` lines.
    fn build_scroll_app(n: usize, viewport: usize, scroll_offset_config: usize) -> App {
        let lines: Vec<DiffLine> = (1..=n)
            .map(|i| DiffLine {
                origin: crate::model::LineOrigin::Context,
                content: format!("line {i}"),
                old_lineno: Some(i as u32),
                new_lineno: Some(i as u32),
                highlighted_spans: None,
            })
            .collect();

        let hunk = DiffHunk {
            header: "@@ -1,N +1,N @@".to_string(),
            lines,
            old_start: 1,
            old_count: n as u32,
            new_start: 1,
            new_count: n as u32,
        };

        let file = DiffFile {
            old_path: None,
            new_path: Some(PathBuf::from("test.rs")),
            status: FileStatus::Modified,
            hunks: vec![hunk],
            is_binary: false,
            is_too_large: false,
            is_commit_message: false,
            content_hash: 0,
        };

        let vcs_info = VcsInfo {
            root_path: PathBuf::from("/tmp"),
            head_commit: "abc".to_string(),
            branch_name: Some("main".to_string()),
            vcs_type: VcsType::Git,
        };
        let session = ReviewSession::new(
            vcs_info.root_path.clone(),
            vcs_info.head_commit.clone(),
            vcs_info.branch_name.clone(),
            SessionDiffSource::WorkingTree,
        );

        let mut app = App::build(
            Box::new(DummyVcs {
                info: vcs_info.clone(),
            }),
            vcs_info,
            Theme::dark(),
            None,
            false,
            vec![file],
            session,
            DiffSource::WorkingTree,
            InputMode::Normal,
            Vec::new(),
            None,
            None,
        )
        .expect("failed to build test app");

        app.diff_state.viewport_height = viewport;
        app.diff_state.visible_line_count = viewport;
        app.scroll_offset = scroll_offset_config;
        app
    }

    #[test]
    fn zz_on_last_line_centers_cursor() {
        // 40 diff lines + 4 overhead = 44 total. max_cursor = 42. Viewport = 20.
        let mut app = build_scroll_app(40, 20, 5);
        assert_eq!(app.total_lines(), 44);
        let last = app.max_cursor_line(); // 42

        app.diff_state.cursor_line = last;
        app.center_cursor();

        // scroll = cursor - viewport/2 = 42 - 10 = 32
        assert_eq!(app.diff_state.scroll_offset, 32);
        assert_eq!(app.diff_state.cursor_line, 42);
    }

    #[test]
    fn after_zz_on_last_line_j_does_not_change_scroll() {
        let mut app = build_scroll_app(40, 20, 5);
        let last = app.max_cursor_line();

        app.diff_state.cursor_line = last;
        app.center_cursor();
        let scroll_after_zz = app.diff_state.scroll_offset;

        // Press j — cursor is already at max, and it's centered (not near bottom margin)
        app.cursor_down(1);

        assert_eq!(app.diff_state.cursor_line, last);
        assert_eq!(
            app.diff_state.scroll_offset, scroll_after_zz,
            "j after zz on last line should not change scroll"
        );
    }

    #[test]
    fn after_zz_on_last_line_k_does_not_change_scroll() {
        let mut app = build_scroll_app(40, 20, 5);
        let last = app.max_cursor_line();

        app.diff_state.cursor_line = last;
        app.center_cursor();
        let scroll_after_zz = app.diff_state.scroll_offset;

        // Press k — cursor moves up 1, still in free zone
        app.cursor_up(1);

        assert_eq!(app.diff_state.cursor_line, last - 1);
        assert_eq!(
            app.diff_state.scroll_offset, scroll_after_zz,
            "k after zz on last line should not change scroll"
        );
    }

    #[test]
    fn after_zz_no_oscillation_with_k_then_j() {
        let mut app = build_scroll_app(40, 20, 5);
        let last = app.max_cursor_line();

        app.diff_state.cursor_line = last;
        app.center_cursor();
        let scroll_after_zz = app.diff_state.scroll_offset;

        // k then j should return to the same state
        app.cursor_up(1);
        app.cursor_down(1);

        assert_eq!(app.diff_state.cursor_line, last);
        assert_eq!(
            app.diff_state.scroll_offset, scroll_after_zz,
            "k then j after zz should not cause oscillation"
        );
    }

    #[test]
    fn j_scrolls_one_line_at_a_time() {
        // Viewport 20, total 44. Start at the middle and scroll down.
        let mut app = build_scroll_app(40, 20, 5);

        // Position cursor and scroll in steady state near the bottom margin
        app.diff_state.cursor_line = 20;
        app.diff_state.scroll_offset = 6;
        // steady state: cursor at bottom margin = scroll + visible - margin - 1

        // Scroll down multiple times and verify single-line increments
        for _ in 0..10 {
            let prev_scroll = app.diff_state.scroll_offset;
            let prev_cursor = app.diff_state.cursor_line;
            app.cursor_down(1);
            let scroll_delta = app.diff_state.scroll_offset - prev_scroll;
            let cursor_delta = app.diff_state.cursor_line - prev_cursor;
            assert_eq!(cursor_delta, 1, "cursor should advance by exactly 1");
            assert!(
                scroll_delta <= 1,
                "scroll should advance by at most 1, got {scroll_delta}"
            );
        }
    }

    #[test]
    fn j_on_last_line_near_bottom_does_not_scroll() {
        let mut app = build_scroll_app(40, 20, 5);
        let last = app.max_cursor_line();

        // Put cursor at last line with it near the bottom of viewport
        app.diff_state.cursor_line = last;
        app.diff_state.scroll_offset = last.saturating_sub(19); // cursor at bottom of viewport

        let prev_scroll = app.diff_state.scroll_offset;
        app.cursor_down(1);

        assert_eq!(app.diff_state.cursor_line, last);
        assert_eq!(
            app.diff_state.scroll_offset, prev_scroll,
            "j on last line should never scroll the view"
        );
    }

    #[test]
    fn j_on_last_line_centered_does_not_scroll() {
        let mut app = build_scroll_app(40, 20, 5);
        let last = app.max_cursor_line();

        // Center cursor on last line
        app.diff_state.cursor_line = last;
        app.center_cursor();
        let scroll_after_center = app.diff_state.scroll_offset;

        app.cursor_down(1);

        assert_eq!(
            app.diff_state.scroll_offset, scroll_after_center,
            "j on last line when centered should not scroll"
        );
    }

    #[test]
    fn k_reclaims_empty_space_below() {
        let mut app = build_scroll_app(40, 20, 5);
        let last = app.max_cursor_line();

        // Put cursor at last line at top of view (maximum empty space below)
        app.diff_state.cursor_line = last;
        app.diff_state.scroll_offset = last; // only 1 line visible

        // Press k — should immediately reclaim space (reduce scroll)
        app.cursor_up(1);

        assert_eq!(app.diff_state.cursor_line, last - 1);
        assert!(
            app.diff_state.scroll_offset < last,
            "k should reclaim empty space below, scroll was {} expected less than {}",
            app.diff_state.scroll_offset,
            last
        );
    }

    #[test]
    fn max_scroll_allows_last_line_at_top() {
        let app = build_scroll_app(40, 20, 5);
        let total = app.total_lines();

        assert_eq!(
            app.max_scroll_offset(),
            total - 1,
            "max scroll should allow last line at top of viewport"
        );
    }

    #[test]
    fn smooth_scroll_to_end_no_jumps() {
        // Start at beginning, scroll all the way to the end with j presses
        let mut app = build_scroll_app(40, 20, 5);
        let last = app.max_cursor_line();

        app.diff_state.cursor_line = 0;
        app.diff_state.scroll_offset = 0;

        let mut max_scroll_delta = 0;
        for _ in 0..last {
            let prev_scroll = app.diff_state.scroll_offset;
            app.cursor_down(1);
            let delta = app.diff_state.scroll_offset.saturating_sub(prev_scroll);
            if delta > max_scroll_delta {
                max_scroll_delta = delta;
            }
        }

        assert_eq!(app.diff_state.cursor_line, last);
        assert!(
            max_scroll_delta <= 1,
            "scroll should never jump more than 1 line at a time, max was {max_scroll_delta}"
        );
    }

    #[test]
    fn k_below_midpoint_only_moves_cursor() {
        // After G, cursor is near the bottom of viewport. Pressing k should
        // only move the cursor, not also scroll the view (which would cause
        // a visual 2-line jump).
        let mut app = build_scroll_app(40, 20, 5);
        let last = app.max_cursor_line();

        // Simulate G: cursor at last line, scroll positions it at bottom
        app.diff_state.cursor_line = last;
        app.diff_state.scroll_offset = last.saturating_sub(19);
        let scroll_before = app.diff_state.scroll_offset;

        // k should only move cursor, not scroll
        app.cursor_up(1);
        assert_eq!(app.diff_state.cursor_line, last - 1);
        assert_eq!(
            app.diff_state.scroll_offset, scroll_before,
            "k when cursor is below midpoint should not change scroll"
        );
    }

    #[test]
    fn no_scroll_when_last_line_visible() {
        // When the last content line is visible, cursor should descend
        // to it without the view scrolling (no bottom margin near EOF).
        let mut app = build_scroll_app(40, 20, 5);
        let last = app.max_cursor_line(); // 42

        // Position so last line is visible at viewport bottom: scroll=23, shows lines 23-42
        app.diff_state.scroll_offset = last.saturating_sub(19); // 23
        app.diff_state.cursor_line = last - 5; // 37, viewport position 14

        // Descend toward the last line — scroll should not change
        for i in 0..5 {
            let scroll_before = app.diff_state.scroll_offset;
            app.cursor_down(1);
            assert_eq!(
                app.diff_state.scroll_offset, scroll_before,
                "scroll should not change on step {i} (cursor near EOF with last line visible)"
            );
        }
        assert_eq!(app.diff_state.cursor_line, last);
    }

    #[test]
    fn cursor_cannot_go_past_last_content_line() {
        let mut app = build_scroll_app(40, 20, 5);
        let last = app.max_cursor_line();
        let total = app.total_lines();

        // max_cursor should be strictly less than total_lines - 1
        // (total-1 is the trailing Spacing line)
        assert_eq!(last, total - 2);

        // cursor_down from last line should not advance
        app.diff_state.cursor_line = last;
        app.cursor_down(1);
        assert_eq!(app.diff_state.cursor_line, last);
    }

    #[test]
    fn effective_scroll_margin_prevents_oscillation() {
        // With viewport 21 (odd), margin should be at most 9 (= 21/2 - 1 = 9)
        // so that after centering at position 10 (= 21/2), there's free space
        let state = DiffState {
            visible_line_count: 21,
            viewport_height: 21,
            ..DiffState::default()
        };
        let margin = state.effective_scroll_margin(100);
        assert!(
            margin < 21 / 2,
            "margin ({margin}) must be strictly less than half viewport ({})",
            21 / 2
        );
    }

    #[test]
    fn scroll_offset_zero_means_no_margin() {
        // When scroll_offset is 0, effective margin should be 0 (no margin at file start)
        let state = DiffState {
            visible_line_count: 20,
            viewport_height: 20,
            ..DiffState::default()
        };
        let margin = state.effective_scroll_margin(0);
        assert_eq!(margin, 0, "margin should be 0 when scroll_offset is 0");
    }
}

#[cfg(test)]
mod find_source_line_tests {
    use super::*;

    fn make_diff_line(file_idx: usize, new_lineno: Option<u32>) -> AnnotatedLine {
        AnnotatedLine::DiffLine {
            file_idx,
            hunk_idx: 0,
            line_idx: 0,
            old_lineno: None,
            new_lineno,
        }
    }

    fn make_diff_line_with_old(
        file_idx: usize,
        old_lineno: Option<u32>,
        new_lineno: Option<u32>,
    ) -> AnnotatedLine {
        AnnotatedLine::DiffLine {
            file_idx,
            hunk_idx: 0,
            line_idx: 0,
            old_lineno,
            new_lineno,
        }
    }

    fn make_sbs_line(file_idx: usize, new_lineno: Option<u32>) -> AnnotatedLine {
        AnnotatedLine::SideBySideLine {
            file_idx,
            hunk_idx: 0,
            del_line_idx: None,
            add_line_idx: None,
            old_lineno: None,
            new_lineno,
        }
    }

    #[test]
    fn should_find_exact_match() {
        let annotations = vec![
            AnnotatedLine::FileHeader { file_idx: 0 },
            make_diff_line(0, Some(10)),
            make_diff_line(0, Some(11)),
            make_diff_line(0, Some(12)),
        ];

        let result = find_source_line(&annotations, 0, 11, LineSide::New);
        assert_eq!(result, FindSourceLineResult::Exact(2));
    }

    #[test]
    fn should_find_nearest_when_no_exact_match() {
        let annotations = vec![
            make_diff_line(0, Some(10)),
            make_diff_line(0, Some(15)),
            make_diff_line(0, Some(20)),
        ];

        // Target 12 is closest to line 10 (dist=2) vs 15 (dist=3) vs 20 (dist=8)
        let result = find_source_line(&annotations, 0, 12, LineSide::New);
        assert_eq!(result, FindSourceLineResult::Nearest(0));
    }

    #[test]
    fn should_find_nearest_above_target() {
        let annotations = vec![
            make_diff_line(0, Some(10)),
            make_diff_line(0, Some(15)),
            make_diff_line(0, Some(20)),
        ];

        // Target 18 is closest to line 20 (dist=2) vs 15 (dist=3) vs 10 (dist=8)
        let result = find_source_line(&annotations, 0, 18, LineSide::New);
        assert_eq!(result, FindSourceLineResult::Nearest(2));
    }

    #[test]
    fn should_return_not_found_for_empty_annotations() {
        let annotations: Vec<AnnotatedLine> = vec![];
        let result = find_source_line(&annotations, 0, 42, LineSide::New);
        assert_eq!(result, FindSourceLineResult::NotFound);
    }

    #[test]
    fn should_return_not_found_when_no_lines_in_current_file() {
        let annotations = vec![make_diff_line(1, Some(10)), make_diff_line(1, Some(20))];

        // File 0 has no lines
        let result = find_source_line(&annotations, 0, 10, LineSide::New);
        assert_eq!(result, FindSourceLineResult::NotFound);
    }

    #[test]
    fn should_skip_lines_from_other_files() {
        let annotations = vec![
            make_diff_line(0, Some(100)), // file 0, line 100
            make_diff_line(1, Some(42)),  // file 1, exact match but wrong file
            make_diff_line(0, Some(50)),  // file 0, line 50
        ];

        // Searching file 0 for line 42 — should find nearest (50, dist=8) not file 1's exact match
        let result = find_source_line(&annotations, 0, 42, LineSide::New);
        assert_eq!(result, FindSourceLineResult::Nearest(2));
    }

    #[test]
    fn should_skip_non_diff_line_annotations() {
        let annotations = vec![
            AnnotatedLine::FileHeader { file_idx: 0 },
            AnnotatedLine::HunkHeader {
                file_idx: 0,
                hunk_idx: 0,
            },
            AnnotatedLine::Spacing,
            make_diff_line(0, Some(42)),
        ];

        let result = find_source_line(&annotations, 0, 42, LineSide::New);
        assert_eq!(result, FindSourceLineResult::Exact(3));
    }

    #[test]
    fn should_skip_diff_lines_with_no_new_lineno() {
        // Deletion-only lines have new_lineno = None
        let annotations = vec![make_diff_line(0, None), make_diff_line(0, Some(20))];

        let result = find_source_line(&annotations, 0, 5, LineSide::New);
        assert_eq!(result, FindSourceLineResult::Nearest(1));
    }

    #[test]
    fn should_work_with_side_by_side_lines() {
        let annotations = vec![
            make_sbs_line(0, Some(10)),
            make_sbs_line(0, Some(20)),
            make_sbs_line(0, Some(30)),
        ];

        let result = find_source_line(&annotations, 0, 20, LineSide::New);
        assert_eq!(result, FindSourceLineResult::Exact(1));
    }

    #[test]
    fn should_handle_mixed_diff_and_sbs_lines() {
        let annotations = vec![
            make_diff_line(0, Some(10)),
            make_sbs_line(0, Some(20)),
            make_diff_line(0, Some(30)),
        ];

        let result = find_source_line(&annotations, 0, 25, LineSide::New);
        // Nearest is line 20 (dist=5) or line 30 (dist=5), first match wins
        assert_eq!(result, FindSourceLineResult::Nearest(1));
    }

    #[test]
    fn should_return_not_found_when_only_non_line_annotations() {
        let annotations = vec![
            AnnotatedLine::FileHeader { file_idx: 0 },
            AnnotatedLine::Spacing,
            AnnotatedLine::HunkHeader {
                file_idx: 0,
                hunk_idx: 0,
            },
        ];

        let result = find_source_line(&annotations, 0, 42, LineSide::New);
        assert_eq!(result, FindSourceLineResult::NotFound);
    }

    #[test]
    fn should_prefer_exact_match_over_earlier_nearest() {
        let annotations = vec![
            make_diff_line(0, Some(41)), // dist=1 from target 42
            make_diff_line(0, Some(42)), // exact match
            make_diff_line(0, Some(43)), // dist=1 from target 42
        ];

        let result = find_source_line(&annotations, 0, 42, LineSide::New);
        assert_eq!(result, FindSourceLineResult::Exact(1));
    }

    #[test]
    fn should_find_nearest_for_target_zero() {
        // target_lineno = 0 is out-of-range (lines are 1-indexed) but should
        // still return the nearest line rather than panicking.
        let annotations = vec![make_diff_line(0, Some(1)), make_diff_line(0, Some(5))];

        let result = find_source_line(&annotations, 0, 0, LineSide::New);
        assert_eq!(result, FindSourceLineResult::Nearest(0));
    }

    #[test]
    fn should_tie_break_nearest_by_iteration_order() {
        // When two lines are equidistant, the first one encountered wins.
        // Here lines are in descending order; line 30 (idx 0) and line 10 (idx 2)
        // are both dist=10 from target 20, so idx 0 should win.
        let annotations = vec![
            make_diff_line(0, Some(30)),
            make_diff_line(0, Some(50)),
            make_diff_line(0, Some(10)),
        ];

        let result = find_source_line(&annotations, 0, 20, LineSide::New);
        assert_eq!(result, FindSourceLineResult::Nearest(0));
    }

    #[test]
    fn should_match_old_lineno_when_side_is_old() {
        // Deletion-only lines carry old_lineno but no new_lineno. `:o<n>`
        // must match those.
        let annotations = vec![
            make_diff_line_with_old(0, Some(5), None),
            make_diff_line_with_old(0, Some(10), None),
            make_diff_line(0, Some(50)), // new-side line — should be ignored when side=Old
        ];

        let exact = find_source_line(&annotations, 0, 10, LineSide::Old);
        assert_eq!(exact, FindSourceLineResult::Exact(1));

        let nearest = find_source_line(&annotations, 0, 7, LineSide::Old);
        assert_eq!(nearest, FindSourceLineResult::Nearest(0));
    }

    #[test]
    fn should_not_match_new_lineno_when_side_is_old() {
        // A pure-addition line has no old_lineno; searching old-side should
        // not fall back to its new_lineno.
        let annotations = vec![make_diff_line(0, Some(42))];

        let result = find_source_line(&annotations, 0, 42, LineSide::Old);
        assert_eq!(result, FindSourceLineResult::NotFound);
    }
}

#[cfg(test)]
mod change_status_tests {
    use std::fs;

    use tempfile::tempdir;

    use super::*;
    use crate::vcs::traits::VcsType;

    struct StatusProbeMock {
        info: VcsInfo,
        status: VcsChangeStatus,
        staged_files: Vec<DiffFile>,
        unstaged_files: Vec<DiffFile>,
    }

    impl VcsBackend for StatusProbeMock {
        fn info(&self) -> &VcsInfo {
            &self.info
        }

        fn get_working_tree_diff(&self, _highlighter: &SyntaxHighlighter) -> Result<Vec<DiffFile>> {
            Err(TuicrError::NoChanges)
        }

        fn get_staged_diff(&self, _highlighter: &SyntaxHighlighter) -> Result<Vec<DiffFile>> {
            if self.staged_files.is_empty() {
                Err(TuicrError::NoChanges)
            } else {
                Ok(self.staged_files.clone())
            }
        }

        fn get_unstaged_diff(&self, _highlighter: &SyntaxHighlighter) -> Result<Vec<DiffFile>> {
            if self.unstaged_files.is_empty() {
                Err(TuicrError::NoChanges)
            } else {
                Ok(self.unstaged_files.clone())
            }
        }

        fn get_change_status(&self) -> Result<VcsChangeStatus> {
            Ok(self.status)
        }

        fn fetch_context_lines(
            &self,
            _file_path: &Path,
            _file_status: FileStatus,
            _ref_commit: Option<&str>,
            _start_line: u32,
            _end_line: u32,
        ) -> Result<Vec<DiffLine>> {
            Ok(Vec::new())
        }

        fn file_line_count(
            &self,
            _file_path: &Path,
            _file_status: FileStatus,
            _ref_commit: Option<&str>,
        ) -> Result<u32> {
            Ok(0)
        }
    }

    fn diff_file(path: &str) -> DiffFile {
        DiffFile {
            old_path: None,
            new_path: Some(PathBuf::from(path)),
            status: FileStatus::Modified,
            hunks: Vec::new(),
            is_binary: false,
            is_too_large: false,
            is_commit_message: false,
            content_hash: 0,
        }
    }

    fn mock_vcs(root_path: PathBuf) -> StatusProbeMock {
        StatusProbeMock {
            info: VcsInfo {
                root_path,
                head_commit: "HEAD".to_string(),
                branch_name: Some("main".to_string()),
                vcs_type: VcsType::Git,
            },
            status: VcsChangeStatus {
                staged: true,
                unstaged: true,
            },
            staged_files: Vec::new(),
            unstaged_files: Vec::new(),
        }
    }

    #[test]
    fn status_probe_rechecks_positive_rows_when_ignore_rules_exist() {
        let dir = tempdir().expect("failed to create temp dir");
        fs::write(dir.path().join(".tuicrignore"), "ignored/\n")
            .expect("failed to write .tuicrignore");
        let mut vcs = mock_vcs(dir.path().to_path_buf());
        vcs.staged_files = vec![diff_file("ignored/generated.rs")];
        vcs.unstaged_files = vec![diff_file("src/lib.rs")];

        let status = App::get_change_status_with_ignore(
            &vcs,
            dir.path(),
            &SyntaxHighlighter::default(),
            None,
        )
        .expect("failed to get change status");

        assert_eq!(
            status,
            VcsChangeStatus {
                staged: false,
                unstaged: true,
            }
        );
        // The mock backend's full-diff path was used (no list_changed_paths
        // override), so it returned the unstaged file. With a real backend
        // that implements list_changed_paths the same path-filter logic runs
        // without ever materializing the diff.
    }

    #[test]
    fn status_probe_does_not_load_diffs_without_ignore_rules() {
        let dir = tempdir().expect("failed to create temp dir");
        let vcs = mock_vcs(dir.path().to_path_buf());

        let status = App::get_change_status_with_ignore(
            &vcs,
            dir.path(),
            &SyntaxHighlighter::default(),
            None,
        )
        .expect("failed to get change status");

        assert_eq!(
            status,
            VcsChangeStatus {
                staged: true,
                unstaged: true,
            }
        );
        // No ignore rules → no diffs were loaded by the probe (the mock's
        // get_X_diff methods would have errored if hit, since staged_files
        // and unstaged_files are empty).
    }

    #[test]
    fn list_changed_paths_used_to_verify_ignore_rules() {
        use std::cell::Cell;

        struct PathProbeMock {
            inner: StatusProbeMock,
            list_calls: Cell<u32>,
            staged_paths: Vec<PathBuf>,
            unstaged_paths: Vec<PathBuf>,
        }

        impl VcsBackend for PathProbeMock {
            fn info(&self) -> &VcsInfo {
                self.inner.info()
            }
            fn get_working_tree_diff(
                &self,
                _highlighter: &SyntaxHighlighter,
            ) -> Result<Vec<DiffFile>> {
                Err(TuicrError::UnsupportedOperation(
                    "should not be called".into(),
                ))
            }
            fn get_staged_diff(&self, _highlighter: &SyntaxHighlighter) -> Result<Vec<DiffFile>> {
                Err(TuicrError::UnsupportedOperation(
                    "should not be called".into(),
                ))
            }
            fn get_unstaged_diff(&self, _highlighter: &SyntaxHighlighter) -> Result<Vec<DiffFile>> {
                Err(TuicrError::UnsupportedOperation(
                    "should not be called".into(),
                ))
            }
            fn get_change_status(&self) -> Result<VcsChangeStatus> {
                self.inner.get_change_status()
            }
            fn list_changed_paths(&self, kind: ChangeKind) -> Result<Vec<PathBuf>> {
                self.list_calls.set(self.list_calls.get() + 1);
                Ok(match kind {
                    ChangeKind::Staged => self.staged_paths.clone(),
                    ChangeKind::Unstaged => self.unstaged_paths.clone(),
                })
            }
            fn fetch_context_lines(
                &self,
                _file_path: &Path,
                _file_status: FileStatus,
                _ref_commit: Option<&str>,
                _start_line: u32,
                _end_line: u32,
            ) -> Result<Vec<DiffLine>> {
                Ok(Vec::new())
            }
            fn file_line_count(
                &self,
                _file_path: &Path,
                _file_status: FileStatus,
                _ref_commit: Option<&str>,
            ) -> Result<u32> {
                Ok(0)
            }
        }

        let dir = tempdir().expect("failed to create temp dir");
        fs::write(dir.path().join(".tuicrignore"), "ignored/\n")
            .expect("failed to write .tuicrignore");

        let vcs = PathProbeMock {
            inner: mock_vcs(dir.path().to_path_buf()),
            list_calls: Cell::new(0),
            staged_paths: vec![PathBuf::from("ignored/generated.rs")],
            unstaged_paths: vec![PathBuf::from("src/lib.rs")],
        };

        let status = App::get_change_status_with_ignore(
            &vcs,
            dir.path(),
            &SyntaxHighlighter::default(),
            None,
        )
        .expect("failed to get change status");

        assert_eq!(
            status,
            VcsChangeStatus {
                staged: false,
                unstaged: true,
            }
        );
        // Both sides probed via cheap path listing, not via full diff parsing.
        assert_eq!(vcs.list_calls.get(), 2);
    }
}

#[cfg(test)]
mod expand_gap_tests {
    use super::*;
    use crate::model::{DiffHunk, DiffLine, FileStatus, LineOrigin};
    use crate::vcs::traits::VcsType;

    struct MockVcs {
        info: VcsInfo,
        /// Total lines available in the "file" (1-indexed)
        total_lines: u32,
    }

    impl VcsBackend for MockVcs {
        fn info(&self) -> &VcsInfo {
            &self.info
        }

        fn get_working_tree_diff(&self, _highlighter: &SyntaxHighlighter) -> Result<Vec<DiffFile>> {
            Err(TuicrError::NoChanges)
        }

        fn fetch_context_lines(
            &self,
            _file_path: &Path,
            _file_status: FileStatus,
            _ref_commit: Option<&str>,
            start_line: u32,
            end_line: u32,
        ) -> Result<Vec<DiffLine>> {
            let mut result = Vec::new();
            for line_num in start_line..=end_line.min(self.total_lines) {
                result.push(DiffLine {
                    origin: LineOrigin::Context,
                    content: format!("line {line_num}"),
                    old_lineno: Some(line_num),
                    new_lineno: Some(line_num),
                    highlighted_spans: None,
                });
            }
            Ok(result)
        }

        fn file_line_count(
            &self,
            _file_path: &Path,
            _file_status: FileStatus,
            _ref_commit: Option<&str>,
        ) -> Result<u32> {
            Ok(self.total_lines)
        }
    }

    fn make_hunk(new_start: u32, new_count: u32) -> DiffHunk {
        let mut lines = Vec::new();
        for i in 0..new_count {
            lines.push(DiffLine {
                origin: LineOrigin::Context,
                content: format!("hunk line {}", new_start + i),
                old_lineno: Some(new_start + i),
                new_lineno: Some(new_start + i),
                highlighted_spans: None,
            });
        }
        DiffHunk {
            header: format!("@@ -{new_start},{new_count} +{new_start},{new_count} @@"),
            lines,
            old_start: new_start,
            old_count: new_count,
            new_start,
            new_count,
        }
    }

    fn build_app_with_files(files: Vec<DiffFile>, total_lines: u32) -> App {
        let vcs_info = VcsInfo {
            root_path: PathBuf::from("/tmp"),
            head_commit: "abc123".to_string(),
            branch_name: Some("main".to_string()),
            vcs_type: VcsType::Git,
        };
        let session = ReviewSession::new(
            vcs_info.root_path.clone(),
            vcs_info.head_commit.clone(),
            vcs_info.branch_name.clone(),
            SessionDiffSource::WorkingTree,
        );

        App::build(
            Box::new(MockVcs {
                info: vcs_info.clone(),
                total_lines,
            }),
            vcs_info,
            Theme::dark(),
            None,
            false,
            files,
            session,
            DiffSource::WorkingTree,
            InputMode::Normal,
            Vec::new(),
            None,
            None,
        )
        .expect("failed to build test app")
    }

    fn make_file_with_hunks(path: &str, hunks: Vec<DiffHunk>) -> DiffFile {
        let content_hash = DiffFile::compute_content_hash(&hunks);
        DiffFile {
            old_path: None,
            new_path: Some(PathBuf::from(path)),
            status: FileStatus::Modified,
            hunks,
            is_binary: false,
            is_too_large: false,
            is_commit_message: false,
            content_hash,
        }
    }

    fn hunk_diff_line(app: &App, file_idx: usize, hunk_idx: usize) -> usize {
        app.line_annotations
            .iter()
            .position(|line| {
                matches!(
                    line,
                    AnnotatedLine::DiffLine {
                        file_idx: candidate_file_idx,
                        hunk_idx: candidate_hunk_idx,
                        ..
                    } if *candidate_file_idx == file_idx && *candidate_hunk_idx == hunk_idx
                )
            })
            .expect("missing hunk diff annotation")
    }

    #[test]
    fn should_toggle_hunk_reviewed_from_header() {
        let file = make_file_with_hunks("test.rs", vec![make_hunk(1, 3)]);
        let mut app = build_app_with_files(vec![file], 20);
        let path = app.diff_files[0].display_path().clone();
        let key = app.diff_files[0].hunk_review_key(0).unwrap();

        app.diff_state.cursor_line = app.hunk_header_line(0, 0).expect("missing hunk header");
        app.toggle_hunk_reviewed();

        assert!(app.session.is_hunk_reviewed(&path, &key));
        assert_eq!(
            app.message.as_ref().unwrap().content,
            "Hunk marked reviewed"
        );
        assert!(matches!(
            app.line_annotations[app.diff_state.cursor_line],
            AnnotatedLine::HunkHeader {
                file_idx: 0,
                hunk_idx: 0
            }
        ));
    }

    #[test]
    fn should_toggle_hunk_reviewed_from_diff_line() {
        let file = make_file_with_hunks("test.rs", vec![make_hunk(1, 3)]);
        let mut app = build_app_with_files(vec![file], 20);
        let path = app.diff_files[0].display_path().clone();
        let key = app.diff_files[0].hunk_review_key(0).unwrap();

        app.diff_state.cursor_line = hunk_diff_line(&app, 0, 0);
        app.toggle_hunk_reviewed();

        assert!(app.session.is_hunk_reviewed(&path, &key));
    }

    #[test]
    fn should_warn_when_toggling_hunk_outside_hunk() {
        let file = make_file_with_hunks("test.rs", vec![make_hunk(1, 3)]);
        let mut app = build_app_with_files(vec![file], 20);
        let path = app.diff_files[0].display_path().clone();
        let key = app.diff_files[0].hunk_review_key(0).unwrap();

        app.diff_state.cursor_line = 0;
        app.toggle_hunk_reviewed();

        assert!(!app.session.is_hunk_reviewed(&path, &key));
        assert_eq!(
            app.message.as_ref().unwrap().content,
            "Move cursor to a hunk to toggle reviewed"
        );
        assert_eq!(
            app.message.as_ref().unwrap().message_type,
            MessageType::Warning
        );
    }

    #[test]
    fn should_fold_reviewed_hunk_body() {
        let file = make_file_with_hunks("test.rs", vec![make_hunk(1, 3), make_hunk(10, 2)]);
        let mut app = build_app_with_files(vec![file], 20);
        let before = app.total_lines();

        app.diff_state.cursor_line = app.hunk_header_line(0, 0).expect("missing hunk header");
        app.toggle_hunk_reviewed();

        assert_eq!(app.total_lines(), before - 4);
        assert!(!app.line_annotations.iter().any(|line| {
            matches!(
                line,
                AnnotatedLine::DiffLine {
                    file_idx: 0,
                    hunk_idx: 0,
                    ..
                }
            )
        }));
        assert!(!app.line_annotations.iter().any(|line| {
            matches!(
                line,
                AnnotatedLine::Expander {
                    gap_id: GapId {
                        file_idx: 0,
                        hunk_idx: 1
                    },
                    ..
                } | AnnotatedLine::HiddenLines {
                    gap_id: GapId {
                        file_idx: 0,
                        hunk_idx: 1
                    },
                    ..
                }
            )
        }));
        assert!(app.line_annotations.iter().any(|line| {
            matches!(
                line,
                AnnotatedLine::DiffLine {
                    file_idx: 0,
                    hunk_idx: 1,
                    ..
                }
            )
        }));
    }

    #[test]
    fn should_keep_file_and_hunk_reviewed_state_independent() {
        let file = make_file_with_hunks("test.rs", vec![make_hunk(1, 3)]);
        let mut app = build_app_with_files(vec![file], 20);
        let path = app.diff_files[0].display_path().clone();
        let key = app.diff_files[0].hunk_review_key(0).unwrap();

        app.diff_state.cursor_line = app.hunk_header_line(0, 0).expect("missing hunk header");
        app.toggle_hunk_reviewed();
        app.toggle_reviewed_for_file_idx(0, false);

        assert!(app.session.is_file_reviewed(&path));
        assert!(app.session.is_hunk_reviewed(&path, &key));
    }

    #[test]
    fn should_expand_up_from_first_hunk() {
        // given: file with 50-line gap before first hunk (hunk starts at line 51)
        let file = make_file_with_hunks("test.rs", vec![make_hunk(51, 5)]);
        let mut app = build_app_with_files(vec![file], 100);
        let gap_id = GapId {
            file_idx: 0,
            hunk_idx: 0,
        };

        // when: expand Up with limit 20 (reveals lines closest to hunk)
        app.expand_gap(gap_id.clone(), ExpandDirection::Up, Some(20))
            .unwrap();

        // then: 20 lines expanded from the bottom of the gap (lines 31-50)
        let content = app.expanded_bottom.get(&gap_id).unwrap();
        assert_eq!(content.len(), 20);
        assert_eq!(content[0].new_lineno, Some(31));
        assert_eq!(content[19].new_lineno, Some(50));
    }

    #[test]
    fn should_expand_all_lines_with_both_direction() {
        // given: file with 50-line gap before first hunk
        let file = make_file_with_hunks("test.rs", vec![make_hunk(51, 5)]);
        let mut app = build_app_with_files(vec![file], 100);
        let gap_id = GapId {
            file_idx: 0,
            hunk_idx: 0,
        };

        // when: expand Both (all remaining)
        app.expand_gap(gap_id.clone(), ExpandDirection::Both, None)
            .unwrap();

        // then: all 50 lines in expanded_top
        let content = app.expanded_top.get(&gap_id).unwrap();
        assert_eq!(content.len(), 50);
        assert_eq!(content[0].new_lineno, Some(1));
        assert_eq!(content[49].new_lineno, Some(50));
    }

    #[test]
    fn should_expand_down_from_upper_hunk() {
        // given: file with two hunks, gap of 24 lines (6..29) between them
        let file = make_file_with_hunks("test.rs", vec![make_hunk(1, 5), make_hunk(30, 5)]);
        let mut app = build_app_with_files(vec![file], 100);
        let gap_id = GapId {
            file_idx: 0,
            hunk_idx: 1,
        };

        // when: expand Down with limit 10
        app.expand_gap(gap_id.clone(), ExpandDirection::Down, Some(10))
            .unwrap();

        // then: 10 lines from top of gap (lines 6-15)
        let content = app.expanded_top.get(&gap_id).unwrap();
        assert_eq!(content.len(), 10);
        assert_eq!(content[0].new_lineno, Some(6));
        assert_eq!(content[9].new_lineno, Some(15));
    }

    #[test]
    fn should_expand_up_from_lower_hunk() {
        // given: file with two hunks, gap of 24 lines (6..29) between them
        let file = make_file_with_hunks("test.rs", vec![make_hunk(1, 5), make_hunk(30, 5)]);
        let mut app = build_app_with_files(vec![file], 100);
        let gap_id = GapId {
            file_idx: 0,
            hunk_idx: 1,
        };

        // when: expand Up with limit 10
        app.expand_gap(gap_id.clone(), ExpandDirection::Up, Some(10))
            .unwrap();

        // then: 10 lines from bottom of gap (lines 20-29)
        let content = app.expanded_bottom.get(&gap_id).unwrap();
        assert_eq!(content.len(), 10);
        assert_eq!(content[0].new_lineno, Some(20));
        assert_eq!(content[9].new_lineno, Some(29));
    }

    #[test]
    fn should_append_on_subsequent_down_expand() {
        // given: already expanded 20 lines down
        let file = make_file_with_hunks("test.rs", vec![make_hunk(1, 5), make_hunk(50, 5)]);
        let mut app = build_app_with_files(vec![file], 100);
        let gap_id = GapId {
            file_idx: 0,
            hunk_idx: 1,
        };
        app.expand_gap(gap_id.clone(), ExpandDirection::Down, Some(20))
            .unwrap();

        // when: expand Down 20 more
        app.expand_gap(gap_id.clone(), ExpandDirection::Down, Some(20))
            .unwrap();

        // then: 40 lines total in top
        let content = app.expanded_top.get(&gap_id).unwrap();
        assert_eq!(content.len(), 40);
        assert_eq!(content[0].new_lineno, Some(6));
        assert_eq!(content[39].new_lineno, Some(45));
    }

    #[test]
    fn should_prepend_on_subsequent_up_expand() {
        // given: already expanded 10 lines up from bottom
        let file = make_file_with_hunks("test.rs", vec![make_hunk(1, 5), make_hunk(50, 5)]);
        let mut app = build_app_with_files(vec![file], 100);
        let gap_id = GapId {
            file_idx: 0,
            hunk_idx: 1,
        };
        app.expand_gap(gap_id.clone(), ExpandDirection::Up, Some(10))
            .unwrap();

        // when: expand Up 10 more
        app.expand_gap(gap_id.clone(), ExpandDirection::Up, Some(10))
            .unwrap();

        // then: 20 lines total in bottom, in ascending order
        let content = app.expanded_bottom.get(&gap_id).unwrap();
        assert_eq!(content.len(), 20);
        assert_eq!(content[0].new_lineno, Some(30));
        assert_eq!(content[19].new_lineno, Some(49));
    }

    #[test]
    fn should_cap_at_gap_boundaries() {
        // given: file with 50-line gap, already expanded 40 up
        let file = make_file_with_hunks("test.rs", vec![make_hunk(51, 5)]);
        let mut app = build_app_with_files(vec![file], 100);
        let gap_id = GapId {
            file_idx: 0,
            hunk_idx: 0,
        };
        app.expand_gap(gap_id.clone(), ExpandDirection::Up, Some(40))
            .unwrap();

        // when: expand Up 20 more (only 10 remain)
        app.expand_gap(gap_id.clone(), ExpandDirection::Up, Some(20))
            .unwrap();

        // then: all 50 lines in bottom
        let content = app.expanded_bottom.get(&gap_id).unwrap();
        assert_eq!(content.len(), 50);
        assert_eq!(content[0].new_lineno, Some(1));
    }

    #[test]
    fn should_show_up_expander_for_top_of_file_partial() {
        // given: file with 50-line gap, expanded 20 lines up
        let file = make_file_with_hunks("test.rs", vec![make_hunk(51, 5)]);
        let mut app = build_app_with_files(vec![file], 100);
        let gap_id = GapId {
            file_idx: 0,
            hunk_idx: 0,
        };
        app.expand_gap(gap_id.clone(), ExpandDirection::Up, Some(20))
            .unwrap();

        // then: should have ↑ expander + hidden lines annotation
        let expander_count = app
            .line_annotations
            .iter()
            .filter(|a| matches!(a, AnnotatedLine::Expander { gap_id: g, direction: ExpandDirection::Up } if *g == gap_id))
            .count();
        assert_eq!(expander_count, 1);

        let hidden_count = app
            .line_annotations
            .iter()
            .filter(|a| matches!(a, AnnotatedLine::HiddenLines { gap_id: g, .. } if *g == gap_id))
            .count();
        assert_eq!(hidden_count, 1, "should show hidden lines count");

        let expanded_count = app
            .line_annotations
            .iter()
            .filter(
                |a| matches!(a, AnnotatedLine::ExpandedContext { gap_id: g, .. } if *g == gap_id),
            )
            .count();
        assert_eq!(expanded_count, 20);
    }

    #[test]
    fn should_not_show_expander_when_fully_expanded() {
        // given: file with 50-line gap, fully expanded
        let file = make_file_with_hunks("test.rs", vec![make_hunk(51, 5)]);
        let mut app = build_app_with_files(vec![file], 100);
        let gap_id = GapId {
            file_idx: 0,
            hunk_idx: 0,
        };
        app.expand_gap(gap_id.clone(), ExpandDirection::Both, None)
            .unwrap();

        // then: no expander or hidden lines
        let expander_count = app
            .line_annotations
            .iter()
            .filter(|a| matches!(a, AnnotatedLine::Expander { gap_id: g, .. } if *g == gap_id))
            .count();
        assert_eq!(expander_count, 0);
    }

    #[test]
    fn should_show_merged_expander_for_small_between_hunk_gap() {
        // given: file with two hunks and a 15-line gap between them
        let file = make_file_with_hunks("test.rs", vec![make_hunk(1, 5), make_hunk(21, 5)]);
        let app = build_app_with_files(vec![file], 100);
        let gap_id = GapId {
            file_idx: 0,
            hunk_idx: 1,
        };

        // then: should show single ↕ expander (gap=15, < 20)
        let both_count = app
            .line_annotations
            .iter()
            .filter(|a| matches!(a, AnnotatedLine::Expander { gap_id: g, direction: ExpandDirection::Both } if *g == gap_id))
            .count();
        assert_eq!(both_count, 1, "small gap should show merged ↕ expander");
    }

    #[test]
    fn should_show_split_expanders_for_large_between_hunk_gap() {
        // given: file with two hunks and a 30-line gap between them
        let file = make_file_with_hunks("test.rs", vec![make_hunk(1, 5), make_hunk(36, 5)]);
        let app = build_app_with_files(vec![file], 100);
        let gap_id = GapId {
            file_idx: 0,
            hunk_idx: 1,
        };

        // then: should show ↓ + hidden + ↑
        let down_count = app
            .line_annotations
            .iter()
            .filter(|a| matches!(a, AnnotatedLine::Expander { gap_id: g, direction: ExpandDirection::Down } if *g == gap_id))
            .count();
        let up_count = app
            .line_annotations
            .iter()
            .filter(|a| matches!(a, AnnotatedLine::Expander { gap_id: g, direction: ExpandDirection::Up } if *g == gap_id))
            .count();
        let hidden_count = app
            .line_annotations
            .iter()
            .filter(|a| matches!(a, AnnotatedLine::HiddenLines { gap_id: g, .. } if *g == gap_id))
            .count();
        assert_eq!(down_count, 1);
        assert_eq!(up_count, 1);
        assert_eq!(hidden_count, 1);
    }

    #[test]
    fn should_expand_gap_in_correct_file_not_adjacent_file() {
        // given: two files, each with a gap before the first hunk
        let file0 = make_file_with_hunks("a.rs", vec![make_hunk(31, 5)]);
        let file1 = make_file_with_hunks("b.rs", vec![make_hunk(21, 5)]);
        let mut app = build_app_with_files(vec![file0, file1], 100);

        let gap_id_file1 = GapId {
            file_idx: 1,
            hunk_idx: 0,
        };

        // when: expand gap in file1
        app.expand_gap(gap_id_file1.clone(), ExpandDirection::Up, Some(10))
            .unwrap();

        // then: expanded content is for file1's gap (10 lines from bottom)
        let content = app.expanded_bottom.get(&gap_id_file1).unwrap();
        assert_eq!(content.len(), 10);
        assert_eq!(content[9].new_lineno, Some(20));

        // and file0's gap should not be expanded
        let gap_id_file0 = GapId {
            file_idx: 0,
            hunk_idx: 0,
        };
        assert!(
            !app.expanded_top.contains_key(&gap_id_file0)
                && !app.expanded_bottom.contains_key(&gap_id_file0)
        );
    }

    #[test]
    fn should_noop_when_already_fully_expanded() {
        // given: file with 10-line gap, fully expanded
        let file = make_file_with_hunks("test.rs", vec![make_hunk(11, 5)]);
        let mut app = build_app_with_files(vec![file], 100);
        let gap_id = GapId {
            file_idx: 0,
            hunk_idx: 0,
        };
        app.expand_gap(gap_id.clone(), ExpandDirection::Both, None)
            .unwrap();
        let len_before = app.expanded_top.get(&gap_id).unwrap().len();

        // when: try to expand again
        app.expand_gap(gap_id.clone(), ExpandDirection::Up, Some(20))
            .unwrap();

        // then: no change
        let len_after = app.expanded_top.get(&gap_id).unwrap().len();
        assert_eq!(len_before, len_after);
    }

    #[test]
    fn should_expand_small_gap_fully_even_with_large_limit() {
        // given: file with 5-line gap
        let file = make_file_with_hunks("test.rs", vec![make_hunk(6, 5)]);
        let mut app = build_app_with_files(vec![file], 100);
        let gap_id = GapId {
            file_idx: 0,
            hunk_idx: 0,
        };

        // when: expand Up with limit 20 (gap is only 5 lines)
        app.expand_gap(gap_id.clone(), ExpandDirection::Up, Some(20))
            .unwrap();

        // then: all 5 lines expanded, no expander remaining
        let content = app.expanded_bottom.get(&gap_id).unwrap();
        assert_eq!(content.len(), 5);

        let expander_count = app
            .line_annotations
            .iter()
            .filter(|a| matches!(a, AnnotatedLine::Expander { gap_id: g, .. } if *g == gap_id))
            .count();
        assert_eq!(expander_count, 0);
    }

    #[test]
    fn should_merge_to_both_when_remaining_drops_below_batch() {
        // given: 30-line between-hunk gap, expand 20 down => 10 remaining
        let file = make_file_with_hunks("test.rs", vec![make_hunk(1, 5), make_hunk(36, 5)]);
        let mut app = build_app_with_files(vec![file], 100);
        let gap_id = GapId {
            file_idx: 0,
            hunk_idx: 1,
        };
        app.expand_gap(gap_id.clone(), ExpandDirection::Down, Some(20))
            .unwrap();

        // then: remaining=10, should show ↕ merged expander
        let both_count = app
            .line_annotations
            .iter()
            .filter(|a| matches!(a, AnnotatedLine::Expander { gap_id: g, direction: ExpandDirection::Both } if *g == gap_id))
            .count();
        assert_eq!(both_count, 1, "should merge to ↕ when <20 remaining");
    }

    fn cursor_new_lineno(app: &App) -> Option<u32> {
        match &app.line_annotations[app.diff_state.cursor_line] {
            AnnotatedLine::DiffLine { new_lineno, .. }
            | AnnotatedLine::SideBySideLine { new_lineno, .. } => *new_lineno,
            AnnotatedLine::ExpandedContext { gap_id, line_idx } => app
                .get_expanded_line(gap_id, *line_idx)
                .and_then(|l| l.new_lineno),
            _ => None,
        }
    }

    fn cursor_old_lineno(app: &App) -> Option<u32> {
        match &app.line_annotations[app.diff_state.cursor_line] {
            AnnotatedLine::DiffLine { old_lineno, .. }
            | AnnotatedLine::SideBySideLine { old_lineno, .. } => *old_lineno,
            AnnotatedLine::ExpandedContext { gap_id, line_idx } => app
                .get_expanded_line(gap_id, *line_idx)
                .and_then(|l| l.old_lineno),
            _ => None,
        }
    }

    #[test]
    fn should_expand_collapsed_gap_when_jumping_into_it() {
        // given: file with a 50-line gap before the first hunk (hunk @ line 51)
        let file = make_file_with_hunks("test.rs", vec![make_hunk(51, 5)]);
        let mut app = build_app_with_files(vec![file], 100);
        let gap_id = GapId {
            file_idx: 0,
            hunk_idx: 0,
        };
        assert!(!app.expanded_top.contains_key(&gap_id));
        assert!(!app.expanded_bottom.contains_key(&gap_id));

        // when: jump to line 30, which lives inside the collapsed gap
        app.go_to_source_line(30, LineSide::New);

        // then: the gap was expanded and the cursor sits on the line whose
        // new_lineno is exactly 30
        assert_eq!(cursor_new_lineno(&app), Some(30));
    }

    #[test]
    fn should_not_expand_when_line_is_already_in_a_hunk() {
        // Line 52 lives inside the hunk's own range, not in a gap — no
        // expansion should occur.
        let file = make_file_with_hunks("test.rs", vec![make_hunk(51, 5)]);
        let mut app = build_app_with_files(vec![file], 100);
        let gap_id = GapId {
            file_idx: 0,
            hunk_idx: 0,
        };

        app.go_to_source_line(52, LineSide::New);

        assert!(!app.expanded_top.contains_key(&gap_id));
        assert!(!app.expanded_bottom.contains_key(&gap_id));
    }

    #[test]
    fn should_expand_old_side_gap_when_jumping_with_o_prefix() {
        // Symmetric gap (offset = 0): `make_hunk` keeps old_start == new_start
        // and the mock VCS returns context lines with old_lineno == new_lineno.
        // We verify the side=Old path of go_to_source_line works end-to-end:
        // the gap auto-expands and the cursor lands on old line 30.
        let file = make_file_with_hunks("test.rs", vec![make_hunk(51, 5)]);
        let mut app = build_app_with_files(vec![file], 100);

        app.go_to_source_line(30, LineSide::Old);

        assert_eq!(cursor_old_lineno(&app), Some(30));
    }

    #[test]
    fn should_expand_up_when_cursor_is_below_the_gap() {
        // Two hunks: hunk0 at lines 1-5, hunk1 at lines 50-54. Gap between
        // them spans new lines 6..=49. Move the cursor onto hunk1, then
        // jump to line 30 — expansion should come from the bottom of the
        // gap (Up) since the cursor sits below it.
        let file = make_file_with_hunks("test.rs", vec![make_hunk(1, 5), make_hunk(50, 5)]);
        let mut app = build_app_with_files(vec![file], 100);
        let gap_id = GapId {
            file_idx: 0,
            hunk_idx: 1,
        };

        let hunk1_header_idx = app
            .line_annotations
            .iter()
            .enumerate()
            .find_map(|(i, a)| match a {
                AnnotatedLine::HunkHeader { hunk_idx: 1, .. } => Some(i),
                _ => None,
            })
            .expect("hunk 1 header should exist");
        app.diff_state.cursor_line = hunk1_header_idx + 1;

        app.go_to_source_line(30, LineSide::New);

        assert_eq!(cursor_new_lineno(&app), Some(30));
        assert!(
            !app.expanded_top.contains_key(&gap_id),
            "no top expansion when cursor is below the gap"
        );
        // Gap covers new lines 6..=49 (44 lines). Up expansion to reach
        // line 30 reveals lines 30..=49 = 20 lines.
        let bot_len = app.expanded_bottom.get(&gap_id).map_or(0, |v| v.len());
        assert_eq!(bot_len, 20);
        let has_down_expander = app.line_annotations.iter().any(|a| {
            matches!(
                a,
                AnnotatedLine::Expander {
                    gap_id: g,
                    direction: ExpandDirection::Down,
                } if *g == gap_id
            )
        });
        assert!(
            has_down_expander,
            "remaining hidden lines need a `↓` expander above the cursor"
        );
    }

    #[test]
    fn should_expand_only_up_to_target_line_not_full_gap() {
        // Gap before hunk spans new lines 1..=50. Jumping to line 20 should
        // reveal lines 1..=20 and leave 21..=50 collapsed behind an
        // `↑` expander.
        let file = make_file_with_hunks("test.rs", vec![make_hunk(51, 5)]);
        let mut app = build_app_with_files(vec![file], 100);
        let gap_id = GapId {
            file_idx: 0,
            hunk_idx: 0,
        };

        app.go_to_source_line(20, LineSide::New);

        assert_eq!(cursor_new_lineno(&app), Some(20));
        let top_len = app.expanded_top.get(&gap_id).map_or(0, |v| v.len());
        assert_eq!(top_len, 20, "only the lines up to the target should expand");
        assert!(
            !app.expanded_bottom.contains_key(&gap_id),
            "no bottom expansion should happen for a downward jump"
        );
        // The unexpanded remainder (30 lines) should still be reachable through
        // an `↑` expander between the cursor and the next hunk.
        let has_up_expander = app.line_annotations.iter().any(|a| {
            matches!(
                a,
                AnnotatedLine::Expander {
                    gap_id: g,
                    direction: ExpandDirection::Up,
                } if *g == gap_id
            )
        });
        assert!(
            has_up_expander,
            "remaining hidden lines need an `↑` expander"
        );
    }

    #[test]
    fn should_show_end_of_file_expander() {
        // given: file with hunk at lines 1-5 and total 100 lines (95 lines after hunk)
        let file = make_file_with_hunks("test.rs", vec![make_hunk(1, 5)]);
        let app = build_app_with_files(vec![file], 100);
        let eof_gap_id = GapId {
            file_idx: 0,
            hunk_idx: 1, // hunks.len() == 1
        };

        // then: should have ↓ expander for end-of-file gap
        let expander_count = app
            .line_annotations
            .iter()
            .filter(|a| matches!(a, AnnotatedLine::Expander { gap_id: g, direction: ExpandDirection::Down } if *g == eof_gap_id))
            .count();
        assert_eq!(expander_count, 1, "should show ↓ expander at end of file");

        // and: should have HiddenLines (95 lines > 20)
        let hidden_count = app
            .line_annotations
            .iter()
            .filter(|a| matches!(a, AnnotatedLine::HiddenLines { gap_id: g, count } if *g == eof_gap_id && *count == 95))
            .count();
        assert_eq!(hidden_count, 1, "should show hidden lines count");

        // and: total_lines() must match line_annotations.len()
        assert_eq!(
            app.total_lines(),
            app.line_annotations.len(),
            "file_render_height sum must match annotation count"
        );
    }

    #[test]
    fn should_expand_down_at_end_of_file() {
        // given: file with hunk at lines 1-5, total 100 lines
        let file = make_file_with_hunks("test.rs", vec![make_hunk(1, 5)]);
        let mut app = build_app_with_files(vec![file], 100);
        let eof_gap_id = GapId {
            file_idx: 0,
            hunk_idx: 1,
        };

        // when: expand Down 20 lines
        app.expand_gap(eof_gap_id.clone(), ExpandDirection::Down, Some(20))
            .unwrap();

        // then: 20 lines expanded from start of EOF gap (lines 6-25)
        let content = app.expanded_top.get(&eof_gap_id).unwrap();
        assert_eq!(content.len(), 20);
        assert_eq!(content[0].new_lineno, Some(6));
        assert_eq!(content[19].new_lineno, Some(25));
    }

    #[test]
    fn should_not_show_eof_gap_when_hunk_ends_at_file_end() {
        // given: file with hunk at lines 1-100 and total 100 lines (no gap)
        let file = make_file_with_hunks("test.rs", vec![make_hunk(1, 100)]);
        let app = build_app_with_files(vec![file], 100);
        let eof_gap_id = GapId {
            file_idx: 0,
            hunk_idx: 1,
        };

        // then: no expander for end-of-file
        let expander_count = app
            .line_annotations
            .iter()
            .filter(|a| matches!(a, AnnotatedLine::Expander { gap_id: g, .. } if *g == eof_gap_id))
            .count();
        assert_eq!(
            expander_count, 0,
            "no EOF expander when hunk covers entire file"
        );
    }

    #[test]
    fn should_handle_subsequent_eof_expansions() {
        // given: file with hunk at lines 1-5, total 50 lines, already expanded 20
        let file = make_file_with_hunks("test.rs", vec![make_hunk(1, 5)]);
        let mut app = build_app_with_files(vec![file], 50);
        let eof_gap_id = GapId {
            file_idx: 0,
            hunk_idx: 1,
        };
        app.expand_gap(eof_gap_id.clone(), ExpandDirection::Down, Some(20))
            .unwrap();

        // when: expand Down 20 more
        app.expand_gap(eof_gap_id.clone(), ExpandDirection::Down, Some(20))
            .unwrap();

        // then: 40 lines total (lines 6-45)
        let content = app.expanded_top.get(&eof_gap_id).unwrap();
        assert_eq!(content.len(), 40);
        assert_eq!(content[0].new_lineno, Some(6));
        assert_eq!(content[39].new_lineno, Some(45));
    }

    #[test]
    fn should_show_small_eof_gap_expander_without_hidden_lines() {
        // given: file with hunk at lines 1-90 and total 100 lines (10-line gap)
        let file = make_file_with_hunks("test.rs", vec![make_hunk(1, 90)]);
        let app = build_app_with_files(vec![file], 100);
        let eof_gap_id = GapId {
            file_idx: 0,
            hunk_idx: 1,
        };

        // then: should have ↓ expander
        let expander_count = app
            .line_annotations
            .iter()
            .filter(|a| matches!(a, AnnotatedLine::Expander { gap_id: g, direction: ExpandDirection::Down } if *g == eof_gap_id))
            .count();
        assert_eq!(expander_count, 1);

        // and: should NOT have HiddenLines (10 <= 20)
        let hidden_count = app
            .line_annotations
            .iter()
            .filter(
                |a| matches!(a, AnnotatedLine::HiddenLines { gap_id: g, .. } if *g == eof_gap_id),
            )
            .count();
        assert_eq!(
            hidden_count, 0,
            "small EOF gap should not show hidden lines"
        );
    }

    #[test]
    fn total_lines_must_match_annotations_with_remote_threads() {
        // Regression: file_render_height previously ignored RemoteThreadLine
        // rows pushed by rebuild_annotations, so max_scroll_offset clamped
        // above the tail of a remote thread on the last line of the last file.
        use crate::forge::remote_comments::{
            RemoteCommentSide, RemoteReviewComment, RemoteReviewThread,
        };

        let files = vec![
            make_file_with_hunks("a.rs", vec![make_hunk(1, 5)]),
            make_file_with_hunks("b.rs", vec![make_hunk(1, 1)]),
        ];
        let mut app = build_app_with_files(files, 1);
        app.sync_viewport_width(80);

        app.forge_review_threads = vec![RemoteReviewThread {
            id: "T1".into(),
            path: "b.rs".into(),
            line: Some(1),
            side: RemoteCommentSide::Right,
            is_resolved: false,
            is_outdated: false,
            comments: vec![RemoteReviewComment {
                id: "C1".into(),
                author: Some("alice".into()),
                body: "first\nsecond\nthird\nfourth".into(),
                created_at: None,
                in_reply_to: None,
                url: "https://example.com/c1".into(),
            }],
        }];
        app.rebuild_annotations();

        assert_eq!(app.total_lines(), app.line_annotations.len());
    }

    #[test]
    fn comment_navigator_items_follow_rendered_comment_order() {
        use crate::forge::remote_comments::{
            RemoteCommentSide, RemoteReviewComment, RemoteReviewThread,
        };

        let files = vec![
            make_file_with_hunks("a.rs", vec![make_hunk(1, 1)]),
            make_file_with_hunks("b.rs", vec![make_hunk(1, 2)]),
        ];
        let mut app = build_app_with_files(files, 10);
        app.sync_viewport_width(80);

        app.session.review_comments.push(Comment::new(
            "review-level".to_string(),
            CommentType::Note,
            None,
        ));
        app.session
            .files
            .get_mut(&PathBuf::from("a.rs"))
            .unwrap()
            .file_comments
            .push(Comment::new(
                "file-level".to_string(),
                CommentType::Suggestion,
                None,
            ));
        app.session
            .files
            .get_mut(&PathBuf::from("b.rs"))
            .unwrap()
            .line_comments
            .entry(2)
            .or_default()
            .push(Comment::new(
                "line-level".to_string(),
                CommentType::Issue,
                Some(LineSide::New),
            ));
        app.forge_review_threads = vec![RemoteReviewThread {
            id: "T1".into(),
            path: "b.rs".into(),
            line: Some(2),
            side: RemoteCommentSide::Right,
            is_resolved: false,
            is_outdated: false,
            comments: vec![RemoteReviewComment {
                id: "C1".into(),
                author: Some("alice".into()),
                body: "remote-thread".into(),
                created_at: None,
                in_reply_to: None,
                url: "https://example.com/c1".into(),
            }],
        }];
        app.rebuild_annotations();

        let items = app.build_comment_navigator_items();

        assert_eq!(items.len(), 4);
        assert!(matches!(
            items[0].key,
            CommentNavigatorKey::Review { comment_idx: 0 }
        ));
        assert!(matches!(
            items[1].key,
            CommentNavigatorKey::File {
                file_idx: 0,
                comment_idx: 0
            }
        ));
        assert!(matches!(
            items[2].key,
            CommentNavigatorKey::Line {
                file_idx: 1,
                line: 2,
                side: LineSide::New,
                comment_idx: 0
            }
        ));
        assert!(matches!(
            items[3].key,
            CommentNavigatorKey::Remote { thread_idx: 0 }
        ));
        assert_eq!(items[3].author.as_deref(), Some("alice"));
    }

    #[test]
    fn comment_navigator_includes_remote_review_summary() {
        use crate::forge::remote_comments::{RemoteReviewState, RemoteReviewSummary};

        let file = make_file_with_hunks("a.rs", vec![make_hunk(1, 1)]);
        let mut app = build_app_with_files(vec![file], 10);
        app.sync_viewport_width(80);

        app.forge_review_summaries = vec![RemoteReviewSummary {
            id: "PRR_1".into(),
            author: Some("alice".into()),
            body: "Overall LGTM".into(),
            state: RemoteReviewState::Commented,
            created_at: None,
            url: "https://example.com/r/1".into(),
        }];
        app.rebuild_annotations();

        let items = app.build_comment_navigator_items();

        // Summary appears as the first navigator item — review-scope, before files.
        assert!(matches!(
            items[0].key,
            CommentNavigatorKey::RemoteReview { summary_idx: 0 }
        ));
        assert_eq!(items[0].author.as_deref(), Some("alice"));
        assert!(items[0].path.is_none());
        assert!(items[0].line.is_none());

        // Scroll math stays consistent: annotation count matches total_lines.
        assert_eq!(app.total_lines(), app.line_annotations.len());
    }

    #[test]
    fn jump_to_selected_comment_uses_comment_annotation_target() {
        let file = make_file_with_hunks("a.rs", vec![make_hunk(1, 2)]);
        let mut app = build_app_with_files(vec![file], 10);
        app.sync_viewport_width(80);
        app.diff_state.viewport_height = 5;
        app.session
            .files
            .get_mut(&PathBuf::from("a.rs"))
            .unwrap()
            .line_comments
            .entry(2)
            .or_default()
            .push(Comment::new(
                "line-level".to_string(),
                CommentType::Issue,
                Some(LineSide::New),
            ));
        app.rebuild_annotations();
        let items = app.build_comment_navigator_items();
        let target = items[0].target_annotation;
        let expected_scroll = target
            .saturating_sub(app.diff_state.viewport_height / 2)
            .min(app.max_scroll_offset());

        app.focused_panel = FocusedPanel::Comments;
        app.comment_navigator_state.select(0);

        assert!(app.jump_to_selected_comment());
        assert_eq!(app.diff_state.cursor_line, target);
        assert_eq!(app.diff_state.scroll_offset, expected_scroll);
        assert_eq!(app.focused_panel, FocusedPanel::Diff);
    }

    #[test]
    fn total_lines_must_match_annotations_with_eof_gaps() {
        // Multiple files, each with EOF gaps of different sizes
        let files = vec![
            make_file_with_hunks("a.rs", vec![make_hunk(10, 5)]), // gap before (9) + gap after
            make_file_with_hunks("b.rs", vec![make_hunk(1, 5), make_hunk(30, 5)]), // between gap + EOF gap
            make_file_with_hunks("c.rs", vec![make_hunk(1, 100)]), // no EOF gap (hunk covers all)
        ];
        let app = build_app_with_files(files, 100);

        assert_eq!(
            app.total_lines(),
            app.line_annotations.len(),
            "total_lines() must equal line_annotations.len()\n\
             total_lines={}, annotations={}",
            app.total_lines(),
            app.line_annotations.len()
        );
    }

    #[test]
    fn should_not_show_eof_gap_for_deleted_files() {
        // given: a deleted file with hunks (old-side content)
        let hunks = vec![make_hunk(1, 5)];
        let content_hash = DiffFile::compute_content_hash(&hunks);
        let file = DiffFile {
            old_path: Some(PathBuf::from("deleted.rs")),
            new_path: None,
            status: FileStatus::Deleted,
            hunks,
            is_binary: false,
            is_too_large: false,
            is_commit_message: false,
            content_hash,
        };
        let app = build_app_with_files(vec![file], 100);
        let eof_gap_id = GapId {
            file_idx: 0,
            hunk_idx: 1,
        };

        // then: no EOF expander for deleted files
        let expander_count = app
            .line_annotations
            .iter()
            .filter(|a| matches!(a, AnnotatedLine::Expander { gap_id: g, .. } if *g == eof_gap_id))
            .count();
        assert_eq!(
            expander_count, 0,
            "deleted files should not have EOF expander"
        );

        // and: total_lines must match annotations
        assert_eq!(app.total_lines(), app.line_annotations.len());
    }
}

#[cfg(test)]
mod visual_selection_tests {
    use super::*;

    fn p(idx: usize, off: usize) -> SelPoint {
        SelPoint {
            annotation_idx: idx,
            char_offset: off,
            side: LineSide::New,
        }
    }

    #[test]
    fn collapsed_starts_at_point() {
        let sel = VisualSelection::collapsed(p(5, 3));
        assert_eq!(sel.anchor, p(5, 3));
        assert_eq!(sel.head, p(5, 3));
    }

    #[test]
    fn ordered_returns_anchor_head_when_already_in_order() {
        let sel = VisualSelection {
            anchor: p(1, 0),
            head: p(4, 8),
        };
        let (start, end) = sel.ordered();
        assert_eq!(start, p(1, 0));
        assert_eq!(end, p(4, 8));
    }

    #[test]
    fn ordered_swaps_when_head_before_anchor_by_idx() {
        let sel = VisualSelection {
            anchor: p(4, 0),
            head: p(1, 0),
        };
        let (start, end) = sel.ordered();
        assert_eq!(start, p(1, 0));
        assert_eq!(end, p(4, 0));
    }

    #[test]
    fn ordered_breaks_ties_on_idx_by_char_offset() {
        let sel = VisualSelection {
            anchor: p(7, 20),
            head: p(7, 5),
        };
        let (start, end) = sel.ordered();
        assert_eq!(start, p(7, 5));
        assert_eq!(end, p(7, 20));
    }
}

#[cfg(test)]
mod submit_flow_tests {
    //! Tests for the `:submit*` preflight / resolver / confirmation
    //! orchestration. Driven through the App methods rather than the key
    //! handlers so we exercise the state machine directly.
    use super::*;
    use crate::forge::submit::{ResolverAction, SubmitEvent, UnmappableReason};
    use crate::forge::traits::{ForgeRepository, PrSessionKey};
    use crate::model::comment::{Comment, CommentLifecycleState, CommentType, LineContext};
    use crate::model::diff_types::{DiffHunk, DiffLine, FileStatus, LineOrigin};
    use crate::vcs::traits::{VcsChangeStatus, VcsType};

    struct DummyVcs {
        info: VcsInfo,
    }

    impl VcsBackend for DummyVcs {
        fn info(&self) -> &VcsInfo {
            &self.info
        }
        fn get_working_tree_diff(&self, _h: &SyntaxHighlighter) -> Result<Vec<DiffFile>> {
            Err(TuicrError::NoChanges)
        }
        fn fetch_context_lines(
            &self,
            _p: &Path,
            _s: FileStatus,
            _ref_commit: Option<&str>,
            _start: u32,
            _end: u32,
        ) -> Result<Vec<DiffLine>> {
            Ok(Vec::new())
        }
        fn get_change_status(&self) -> Result<VcsChangeStatus> {
            Ok(VcsChangeStatus {
                staged: false,
                unstaged: false,
            })
        }
        fn file_line_count(
            &self,
            _p: &Path,
            _s: FileStatus,
            _ref_commit: Option<&str>,
        ) -> Result<u32> {
            Ok(0)
        }
    }

    fn make_pr_app_with_single_modified_file(file_path: &str) -> App {
        let vcs_info = VcsInfo {
            root_path: PathBuf::from("/tmp/repo"),
            head_commit: "abcdef0123".to_string(),
            branch_name: Some("feat".to_string()),
            vcs_type: VcsType::File,
        };
        let session = ReviewSession::new(
            vcs_info.root_path.clone(),
            vcs_info.head_commit.clone(),
            vcs_info.branch_name.clone(),
            SessionDiffSource::PullRequest,
        );
        let diff_file = DiffFile {
            old_path: Some(PathBuf::from(file_path)),
            new_path: Some(PathBuf::from(file_path)),
            status: FileStatus::Modified,
            hunks: vec![DiffHunk {
                header: "@@".to_string(),
                old_start: 1,
                old_count: 0,
                new_start: 1,
                new_count: 0,
                lines: vec![
                    DiffLine {
                        origin: LineOrigin::Context,
                        content: "a".to_string(),
                        old_lineno: Some(10),
                        new_lineno: Some(10),
                        highlighted_spans: None,
                    },
                    DiffLine {
                        origin: LineOrigin::Addition,
                        content: "b".to_string(),
                        old_lineno: None,
                        new_lineno: Some(11),
                        highlighted_spans: None,
                    },
                ],
            }],
            is_binary: false,
            is_too_large: false,
            is_commit_message: false,
            content_hash: 0,
        };
        let pr_source = PullRequestDiffSource {
            key: PrSessionKey::new(
                ForgeRepository::github("github.com", "agavra", "tuicr"),
                125,
                "abcdef0123".to_string(),
            ),
            base_sha: "0000".to_string(),
            title: "test pr".to_string(),
            url: "https://github.com/agavra/tuicr/pull/125".to_string(),
            head_ref_name: "feat".to_string(),
            base_ref_name: "main".to_string(),
            state: "OPEN".to_string(),
            closed: false,
            merged: false,
        };
        let mut app = App::build(
            Box::new(DummyVcs {
                info: vcs_info.clone(),
            }),
            vcs_info,
            Theme::dark(),
            None,
            false,
            vec![diff_file],
            session,
            DiffSource::PullRequest(Box::new(pr_source)),
            InputMode::Normal,
            Vec::new(),
            None,
            None,
        )
        .expect("build app");
        app.current_pr_head = Some("abcdef0123".to_string());
        app
    }

    fn line_comment(side: LineSide, new: Option<u32>, old: Option<u32>) -> Comment {
        let mut c = Comment::new("body".to_string(), CommentType::Issue, Some(side));
        c.line_context = Some(LineContext {
            new_line: new,
            old_line: old,
            content: String::new(),
        });
        c
    }

    fn add_line_comment(app: &mut App, path: &str, line: u32, comment: Comment) {
        let pb = PathBuf::from(path);
        let review = app.session.get_file_mut(&pb).expect("file in session");
        review.line_comments.entry(line).or_default().push(comment);
    }

    fn deliver_matching_pr_threads_event(
        app: &mut App,
        threads: std::result::Result<
            Vec<crate::forge::remote_comments::RemoteReviewThread>,
            String,
        >,
        summaries: std::result::Result<
            Vec<crate::forge::remote_comments::RemoteReviewSummary>,
            String,
        >,
    ) {
        let (tx, rx) = std::sync::mpsc::channel();
        app.pr_threads_rx = Some(rx);
        app.forge_review_threads_loading = true;
        let pr_key = match &app.diff_source {
            DiffSource::PullRequest(pr) => pr.key.clone(),
            _ => panic!("expected PR mode"),
        };
        tx.send(PrThreadsEvent::Done {
            repository: pr_key.repository,
            pr_number: pr_key.number,
            head_sha: pr_key.head_sha,
            threads,
            summaries,
        })
        .unwrap();
    }

    #[test]
    fn should_use_subset_head_sha_as_commit_id_when_inline_selector_is_strict_subset() {
        // Regression for HTTP 422 when reviewing a subset of commits: the
        // payload's `commit_id` must match the SHA the displayed diff was
        // computed against — using the cumulative PR head causes GitHub to
        // reject inline comments whose lines aren't in that diff.
        use crate::forge::traits::PullRequestCommit;

        let mut app = make_pr_app_with_single_modified_file("src/lib.rs");
        // Newest-first: [C3, C2, C1]. PR head SHA is C3 ("abcdef0123").
        app.pr_commits = vec![
            PullRequestCommit {
                oid: "abcdef0123".to_string(),
                short_oid: "abcdef0".to_string(),
                summary: "C3".to_string(),
                author: "me".to_string(),
                timestamp: None,
            },
            PullRequestCommit {
                oid: "deadbeef02".to_string(),
                short_oid: "deadbee".to_string(),
                summary: "C2".to_string(),
                author: "me".to_string(),
                timestamp: None,
            },
            PullRequestCommit {
                oid: "facecafe01".to_string(),
                short_oid: "facecaf".to_string(),
                summary: "C1".to_string(),
                author: "me".to_string(),
                timestamp: None,
            },
        ];
        // Strict subset: only middle commit C2 selected (start=1, end=1).
        app.commit_selection_range = Some((1, 1));
        add_line_comment(
            &mut app,
            "src/lib.rs",
            11,
            Comment::new(
                "comment on C2".to_string(),
                CommentType::Issue,
                Some(LineSide::New),
            ),
        );

        app.start_submit(SubmitEvent::Comment);

        let state = app.submit_state.as_ref().expect("submit state");
        assert_eq!(
            state.commit_id, "deadbeef02",
            "subset → commit_id should be the newest selected commit (start_idx), not the PR head",
        );
    }

    #[test]
    fn should_use_pr_head_sha_as_commit_id_when_full_commit_range_selected() {
        // Counterpart to the subset regression: full-range selection should
        // continue to use the cumulative PR head SHA.
        use crate::forge::traits::PullRequestCommit;

        let mut app = make_pr_app_with_single_modified_file("src/lib.rs");
        app.pr_commits = vec![
            PullRequestCommit {
                oid: "abcdef0123".to_string(),
                short_oid: "abcdef0".to_string(),
                summary: "C2".to_string(),
                author: "me".to_string(),
                timestamp: None,
            },
            PullRequestCommit {
                oid: "facecafe01".to_string(),
                short_oid: "facecaf".to_string(),
                summary: "C1".to_string(),
                author: "me".to_string(),
                timestamp: None,
            },
        ];
        app.commit_selection_range = Some((0, 1));
        add_line_comment(
            &mut app,
            "src/lib.rs",
            11,
            Comment::new(
                "comment".to_string(),
                CommentType::Issue,
                Some(LineSide::New),
            ),
        );

        app.start_submit(SubmitEvent::Comment);

        let state = app.submit_state.as_ref().expect("submit state");
        assert_eq!(state.commit_id, "abcdef0123");
    }

    #[test]
    fn should_anchor_line_comments_via_hashmap_key_when_line_context_missing() {
        // Regression: in production, comments are created via Comment::new
        // which does NOT populate line_context — the line lives only in the
        // line_comments HashMap key. Before the CommentAnchor refactor, the
        // mapper treated these as file-level and posted everything with
        // position 1 plus the "File-level:" body prefix.
        use crate::forge::submit::GhSide;

        let mut app = make_pr_app_with_single_modified_file("src/lib.rs");
        let bare = Comment::new(
            "real line comment".to_string(),
            CommentType::Issue,
            Some(LineSide::New),
        );
        assert!(bare.line_context.is_none(), "fixture contract");
        add_line_comment(&mut app, "src/lib.rs", 11, bare);

        app.start_submit(SubmitEvent::Comment);

        assert_eq!(app.input_mode, InputMode::SubmitConfirm);
        let state = app.submit_state.as_ref().expect("submit state");
        assert_eq!(state.mappable.len(), 1);
        let inline = &state.mappable[0];
        assert_eq!(inline.line, 11);
        assert_eq!(inline.side, GhSide::Right);
        assert!(
            !inline.body.contains("File-level:"),
            "regression: body should not be prefixed File-level (got: {})",
            inline.body
        );
    }

    #[test]
    fn should_open_confirm_directly_when_all_comments_map() {
        // given a PR session with one mappable line comment
        let mut app = make_pr_app_with_single_modified_file("src/lib.rs");
        add_line_comment(
            &mut app,
            "src/lib.rs",
            11,
            line_comment(LineSide::New, Some(11), None),
        );
        // when
        app.start_submit(SubmitEvent::Comment);
        // then — went straight to confirmation, no resolver
        assert_eq!(app.input_mode, InputMode::SubmitConfirm);
        let state = app.submit_state.as_ref().expect("submit state");
        assert_eq!(state.mappable.len(), 1);
        assert!(state.unmappable.is_empty());
        assert_eq!(state.commit_id, "abcdef0123");
        assert_eq!(state.event, SubmitEvent::Comment);
    }

    #[test]
    fn should_open_resolver_when_any_comment_is_unmappable() {
        // given a PR session with one mappable + one file-level on a
        // binary file (unmappable).
        let mut app = make_pr_app_with_single_modified_file("img.png");
        // mark file binary in diff_files
        app.diff_files[0].is_binary = true;
        // file-level comment in session
        let pb = PathBuf::from("img.png");
        let review = app.session.get_file_mut(&pb).expect("file in session");
        review
            .file_comments
            .push(Comment::new("oof".to_string(), CommentType::Note, None));
        // when
        app.start_submit(SubmitEvent::Comment);
        // then — resolver entered with one unmappable
        assert_eq!(app.input_mode, InputMode::SubmitResolver);
        let state = app.submit_state.as_ref().expect("submit state");
        assert_eq!(state.unmappable.len(), 1);
        assert_eq!(state.unmappable[0].reason, UnmappableReason::BinaryFile);
        assert_eq!(state.resolver_choices.len(), 1);
        // Default action is MoveToSummary per spec
        assert_eq!(state.resolver_choices[0], ResolverAction::MoveToSummary);
    }

    #[test]
    fn should_skip_locked_comments_during_preflight() {
        // given a single locked comment
        let mut app = make_pr_app_with_single_modified_file("src/lib.rs");
        let mut c = line_comment(LineSide::New, Some(11), None);
        c.lifecycle_state = CommentLifecycleState::Submitted;
        add_line_comment(&mut app, "src/lib.rs", 11, c);
        // when
        app.start_submit(SubmitEvent::Comment);
        // then — preflight aborted with the "nothing to submit" warning
        assert_eq!(app.input_mode, InputMode::Normal);
        assert!(app.submit_state.is_none());
    }

    #[test]
    fn should_warn_when_no_local_drafts_exist() {
        // given a PR session with zero comments
        let mut app = make_pr_app_with_single_modified_file("src/lib.rs");
        // when
        app.start_submit(SubmitEvent::Comment);
        // then
        assert_eq!(app.input_mode, InputMode::Normal);
        assert!(app.submit_state.is_none());
    }

    #[test]
    fn should_allow_bare_approve_through_action_picker_with_no_comments() {
        // Regression: bare `:submit` → picker → cursor on Approve → Enter
        // should NOT warn "Nothing to submit". Approve is the one event
        // meaningful with no comments. Picker uses skip_confirm = true, so
        // the flow goes straight to network dispatch.
        let mut app = make_pr_app_with_single_modified_file("src/lib.rs");
        app.start_submit_action_picker();
        // Walk cursor to the Approve row (index 1).
        app.submit_picker_cursor_down();
        assert_eq!(app.submit_picker_cursor, 1);
        app.submit_picker_confirm();
        // No warning was emitted; the network call was dispatched.
        assert_eq!(app.input_mode, InputMode::Normal);
        assert!(app.submit_state.is_none());
        assert!(
            app.pr_submit_state.is_some(),
            "picker-confirm should dispatch when Approve is bare-allowed"
        );
    }

    #[test]
    fn should_allow_bare_approve_without_any_comments() {
        // Approve is the one event meaningful with no comments (a plain
        // LGTM). Preflight should proceed; the user lands in the confirm
        // modal with zero mappable + zero unmappable.
        let mut app = make_pr_app_with_single_modified_file("src/lib.rs");
        app.start_submit(SubmitEvent::Approve);
        assert_eq!(app.input_mode, InputMode::SubmitConfirm);
        let state = app.submit_state.as_ref().expect("submit state");
        assert!(state.mappable.is_empty());
        assert!(state.unmappable.is_empty());
        assert_eq!(state.event, SubmitEvent::Approve);
    }

    #[test]
    fn should_warn_when_submitting_without_pr_mode() {
        // given an app NOT in PR mode
        let vcs_info = VcsInfo {
            root_path: PathBuf::from("/tmp"),
            head_commit: "head".to_string(),
            branch_name: Some("main".to_string()),
            vcs_type: VcsType::Git,
        };
        let session = ReviewSession::new(
            vcs_info.root_path.clone(),
            vcs_info.head_commit.clone(),
            vcs_info.branch_name.clone(),
            SessionDiffSource::WorkingTree,
        );
        let mut app = App::build(
            Box::new(DummyVcs {
                info: vcs_info.clone(),
            }),
            vcs_info,
            Theme::dark(),
            None,
            false,
            Vec::new(),
            session,
            DiffSource::WorkingTree,
            InputMode::Normal,
            Vec::new(),
            None,
            None,
        )
        .expect("build app");
        // when
        app.start_submit(SubmitEvent::Comment);
        // then
        assert_eq!(app.input_mode, InputMode::Normal);
        assert!(app.submit_state.is_none());
    }

    #[test]
    fn should_warn_when_pr_is_closed_or_merged() {
        // given a closed PR
        let mut app = make_pr_app_with_single_modified_file("src/lib.rs");
        if let DiffSource::PullRequest(pr) = &mut app.diff_source {
            pr.closed = true;
        }
        add_line_comment(
            &mut app,
            "src/lib.rs",
            11,
            line_comment(LineSide::New, Some(11), None),
        );
        // when
        app.start_submit(SubmitEvent::Comment);
        // then
        assert_eq!(app.input_mode, InputMode::Normal);
        assert!(app.submit_state.is_none());
    }

    #[test]
    fn should_cancel_submit_clears_state_and_returns_to_normal() {
        let mut app = make_pr_app_with_single_modified_file("src/lib.rs");
        add_line_comment(
            &mut app,
            "src/lib.rs",
            11,
            line_comment(LineSide::New, Some(11), None),
        );
        app.start_submit(SubmitEvent::Comment);
        // when
        app.cancel_submit();
        // then
        assert_eq!(app.input_mode, InputMode::Normal);
        assert!(app.submit_state.is_none());
    }

    #[test]
    fn should_toggle_resolver_action_between_move_and_omit() {
        let mut app = make_pr_app_with_single_modified_file("img.png");
        app.diff_files[0].is_binary = true;
        let pb = PathBuf::from("img.png");
        let review = app.session.get_file_mut(&pb).expect("file in session");
        review
            .file_comments
            .push(Comment::new("a".to_string(), CommentType::Note, None));
        review
            .file_comments
            .push(Comment::new("b".to_string(), CommentType::Note, None));
        app.start_submit(SubmitEvent::Comment);
        // when — toggle row 0
        app.submit_resolver_toggle();
        // then
        let state = app.submit_state.as_ref().unwrap();
        assert_eq!(state.resolver_choices[0], ResolverAction::Omit);
        assert_eq!(state.resolver_choices[1], ResolverAction::MoveToSummary);
        // when toggle again
        app.submit_resolver_toggle();
        let state = app.submit_state.as_ref().unwrap();
        assert_eq!(state.resolver_choices[0], ResolverAction::MoveToSummary);
    }

    #[test]
    fn should_advance_from_resolver_to_confirm() {
        let mut app = make_pr_app_with_single_modified_file("img.png");
        app.diff_files[0].is_binary = true;
        let pb = PathBuf::from("img.png");
        let review = app.session.get_file_mut(&pb).expect("file in session");
        review
            .file_comments
            .push(Comment::new("a".to_string(), CommentType::Note, None));
        app.start_submit(SubmitEvent::Comment);
        assert_eq!(app.input_mode, InputMode::SubmitResolver);
        // when
        app.submit_resolver_advance();
        // then
        assert_eq!(app.input_mode, InputMode::SubmitConfirm);
    }

    #[test]
    fn should_skip_confirm_modal_when_action_picker_dispatches_with_no_unmappable() {
        // Bare `:submit` → action picker → pick Comment → directly dispatch
        // the network call without SubmitConfirm. submit_state is cleared
        // and pr_submit_state populated, same end state as the explicit
        // `:submit comment` + [y] flow.
        let mut app = make_pr_app_with_single_modified_file("src/lib.rs");
        add_line_comment(
            &mut app,
            "src/lib.rs",
            11,
            line_comment(LineSide::New, Some(11), None),
        );

        app.start_submit_action_picker();
        assert_eq!(app.input_mode, InputMode::SubmitActionPicker);
        app.submit_picker_confirm();

        assert_eq!(app.input_mode, InputMode::Normal);
        assert!(app.submit_state.is_none());
        assert!(app.pr_submit_state.is_some());
    }

    #[test]
    fn should_route_picker_through_resolver_then_skip_confirm() {
        // Bare `:submit` picker with one unmappable comment → resolver
        // appears → `s` advances and dispatches the network call directly,
        // bypassing SubmitConfirm. skip_confirm is the flag that makes
        // submit_resolver_advance bypass the confirm modal.
        let mut app = make_pr_app_with_single_modified_file("img.png");
        app.diff_files[0].is_binary = true;
        let pb = PathBuf::from("img.png");
        let review = app.session.get_file_mut(&pb).expect("file in session");
        review.file_comments.push(Comment::new(
            "binary art".to_string(),
            CommentType::Note,
            None,
        ));

        app.start_submit_action_picker();
        app.submit_picker_cursor = 0; // Comment
        app.submit_picker_confirm();

        // Picker dispatched → resolver visible because the comment is
        // unmappable, but the underlying skip_confirm flag is set.
        assert_eq!(app.input_mode, InputMode::SubmitResolver);
        let state = app.submit_state.as_ref().expect("submit state");
        assert!(state.skip_confirm);

        // Advance from resolver → goes straight to network call, no
        // SubmitConfirm.
        app.submit_resolver_advance();
        assert_eq!(app.input_mode, InputMode::Normal);
        assert!(app.submit_state.is_none());
        assert!(app.pr_submit_state.is_some());
    }

    #[test]
    fn should_dispatch_async_submit_on_confirm_and_clear_modal_state() {
        // given a PR session with one mappable line comment
        let mut app = make_pr_app_with_single_modified_file("src/lib.rs");
        add_line_comment(
            &mut app,
            "src/lib.rs",
            11,
            line_comment(LineSide::New, Some(11), None),
        );
        app.start_submit(SubmitEvent::Comment);
        // when
        app.confirm_submit();
        // then — PR 6: confirmation modal disappears immediately; the
        // background thread is running with state captured on
        // pr_submit_state so the spinner has something to render.
        assert_eq!(app.input_mode, InputMode::Normal);
        assert!(app.submit_state.is_none());
        assert!(
            app.pr_submit_state.is_some(),
            "spawn_pr_submit should populate pr_submit_state"
        );
        assert!(app.pr_submit_rx.is_some(), "rx must be present in-flight");
    }

    fn make_in_flight(
        event: SubmitEvent,
        comment_ids: &[&str],
        head_sha: &str,
        moved_to_summary_count: usize,
    ) -> SubmitInFlightState {
        let mappable = comment_ids
            .iter()
            .enumerate()
            .map(|(i, id)| crate::forge::submit::InlineComment {
                path: PathBuf::from("src/lib.rs"),
                line: 11 + i as u32,
                side: crate::forge::submit::GhSide::Right,
                counterpart_line: None,
                start_line: None,
                start_side: None,
                old_path: None,
                body: "x".to_string(),
                comment_id: (*id).to_string(),
            })
            .collect();
        SubmitInFlightState {
            event,
            mappable,
            summary_comment_ids: Vec::new(),
            review_comment_ids: Vec::new(),
            moved_to_summary_count,
            head_sha_snapshot: head_sha.to_string(),
            repository: ForgeRepository::github("github.com", "agavra", "tuicr"),
            pr_number: 125,
            started_at: Instant::now(),
        }
    }

    fn make_response(
        id: u64,
        html_url: &str,
        state: &str,
    ) -> crate::forge::traits::GhCreateReviewResponse {
        crate::forge::traits::GhCreateReviewResponse {
            id,
            html_url: html_url.to_string(),
            state: state.to_string(),
        }
    }

    #[test]
    fn should_flip_comments_to_submitted_and_stamp_review_id_on_success() {
        // given an app with one line comment that we'll claim got submitted
        let mut app = make_pr_app_with_single_modified_file("src/lib.rs");
        let comment = line_comment(LineSide::New, Some(11), None);
        let comment_id = comment.id.clone();
        add_line_comment(&mut app, "src/lib.rs", 11, comment);
        let in_flight = make_in_flight(
            SubmitEvent::Comment,
            &[comment_id.as_str()],
            "abcdef0123",
            0,
        );
        let response = make_response(987654, "https://example.com/r", "COMMENTED");
        // when
        app.apply_submit_success(&in_flight, &response);
        // then — the comment stays visible but is locked; remote_review_id set
        let review = app.session.files.get(&PathBuf::from("src/lib.rs")).unwrap();
        let saved = &review.line_comments.get(&11).unwrap()[0];
        assert_eq!(saved.lifecycle_state, CommentLifecycleState::Submitted);
        assert_eq!(saved.remote_review_id.as_deref(), Some("987654"));
    }

    #[test]
    fn should_flip_comments_to_pushed_draft_for_draft_submission() {
        // given
        let mut app = make_pr_app_with_single_modified_file("src/lib.rs");
        let comment = line_comment(LineSide::New, Some(11), None);
        let comment_id = comment.id.clone();
        add_line_comment(&mut app, "src/lib.rs", 11, comment);
        let in_flight = make_in_flight(SubmitEvent::Draft, &[comment_id.as_str()], "abcdef0123", 0);
        let response = make_response(42, "https://example.com/r", "PENDING");
        // when
        app.apply_submit_success(&in_flight, &response);
        // then
        let review = app.session.files.get(&PathBuf::from("src/lib.rs")).unwrap();
        let saved = &review.line_comments.get(&11).unwrap()[0];
        assert_eq!(saved.lifecycle_state, CommentLifecycleState::PushedDraft);
        assert_eq!(saved.remote_review_id.as_deref(), Some("42"));
    }

    #[test]
    fn should_only_flip_comments_whose_ids_were_submitted() {
        // given — one matching id and one stray local-draft that wasn't in the submit
        let mut app = make_pr_app_with_single_modified_file("src/lib.rs");
        let comment = line_comment(LineSide::New, Some(11), None);
        let target_id = comment.id.clone();
        add_line_comment(&mut app, "src/lib.rs", 11, comment);
        let untouched = line_comment(LineSide::New, Some(11), None);
        let untouched_id = untouched.id.clone();
        add_line_comment(&mut app, "src/lib.rs", 11, untouched);
        let in_flight =
            make_in_flight(SubmitEvent::Comment, &[target_id.as_str()], "abcdef0123", 0);
        let response = make_response(1, "u", "COMMENTED");
        // when
        app.apply_submit_success(&in_flight, &response);
        // then — only the target id moved to Submitted; the other stays a LocalDraft
        let review = app.session.files.get(&PathBuf::from("src/lib.rs")).unwrap();
        let comments = review.line_comments.get(&11).unwrap();
        let target = comments.iter().find(|c| c.id == target_id).unwrap();
        let other = comments.iter().find(|c| c.id == untouched_id).unwrap();
        assert_eq!(target.lifecycle_state, CommentLifecycleState::Submitted);
        assert_eq!(other.lifecycle_state, CommentLifecycleState::LocalDraft);
        assert!(other.remote_review_id.is_none());
    }

    #[test]
    fn should_flip_review_level_comments_on_submit_success() {
        // given a PR session with a review-level comment included in the
        // review body. Its id is tracked on the in-flight state so we know
        // exactly which review-level entries went out.
        let mut app = make_pr_app_with_single_modified_file("src/lib.rs");
        let review_comment = Comment::new("nice work".to_string(), CommentType::Note, None);
        let review_comment_id = review_comment.id.clone();
        app.session.review_comments.push(review_comment);
        let mut in_flight = make_in_flight(SubmitEvent::Comment, &[], "abcdef0123", 0);
        in_flight.review_comment_ids = vec![review_comment_id.clone()];
        let response = make_response(55, "u", "COMMENTED");
        // when
        app.apply_submit_success(&in_flight, &response);
        // then — review-level comments locked too, since they went out in the body
        let saved = app
            .session
            .review_comments
            .iter()
            .find(|c| c.id == review_comment_id)
            .unwrap();
        assert_eq!(saved.lifecycle_state, CommentLifecycleState::Submitted);
        assert_eq!(saved.remote_review_id.as_deref(), Some("55"));
    }

    #[test]
    fn should_flip_summary_bound_unmappable_comments_on_submit_success() {
        // given an unmappable line comment the user chose to "move to summary"
        // — it was embedded in the review body, so the local copy is now stale.
        let mut app = make_pr_app_with_single_modified_file("src/lib.rs");
        let comment = line_comment(LineSide::New, Some(11), None);
        let summary_id = comment.id.clone();
        add_line_comment(&mut app, "src/lib.rs", 11, comment);
        let mut in_flight = make_in_flight(SubmitEvent::Comment, &[], "abcdef0123", 1);
        in_flight.summary_comment_ids = vec![summary_id.clone()];
        let response = make_response(77, "u", "COMMENTED");
        // when
        app.apply_submit_success(&in_flight, &response);
        // then
        let review = app.session.files.get(&PathBuf::from("src/lib.rs")).unwrap();
        let saved = &review.line_comments.get(&11).unwrap()[0];
        assert_eq!(saved.lifecycle_state, CommentLifecycleState::Submitted);
        assert_eq!(saved.remote_review_id.as_deref(), Some("77"));
    }

    #[test]
    fn should_prune_locked_comments_across_all_buckets() {
        // given a session populated with one locked comment in each bucket
        // (review-level, file-level, line) plus an unlocked draft that must
        // survive the prune.
        let mut app = make_pr_app_with_single_modified_file("src/lib.rs");

        let mut review_locked = Comment::new("body item".to_string(), CommentType::Note, None);
        review_locked.lifecycle_state = CommentLifecycleState::Submitted;
        app.session.review_comments.push(review_locked);

        let mut file_locked = Comment::new("file-level".to_string(), CommentType::Note, None);
        file_locked.lifecycle_state = CommentLifecycleState::PushedDraft;
        let mut line_locked = line_comment(LineSide::New, Some(11), None);
        line_locked.lifecycle_state = CommentLifecycleState::Submitted;
        let line_unlocked = line_comment(LineSide::New, Some(12), None);
        let unlocked_id = line_unlocked.id.clone();
        {
            let review = app
                .session
                .get_file_mut(&PathBuf::from("src/lib.rs"))
                .unwrap();
            review.file_comments.push(file_locked);
        }
        add_line_comment(&mut app, "src/lib.rs", 11, line_locked);
        add_line_comment(&mut app, "src/lib.rs", 12, line_unlocked);

        // when remote threads come back and pruning runs
        app.prune_locked_comments();

        // then — every locked entry is gone; the unlocked draft survives
        assert!(app.session.review_comments.is_empty());
        let review = app.session.files.get(&PathBuf::from("src/lib.rs")).unwrap();
        assert!(review.file_comments.is_empty());
        assert!(!review.line_comments.contains_key(&11));
        let surviving = review.line_comments.get(&12).unwrap();
        assert_eq!(surviving.len(), 1);
        assert_eq!(surviving[0].id, unlocked_id);
    }

    #[test]
    fn should_keep_locked_line_comments_when_remote_thread_refetch_fails() {
        let mut app = make_pr_app_with_single_modified_file("src/lib.rs");
        let mut line_locked = line_comment(LineSide::New, Some(11), None);
        line_locked.lifecycle_state = CommentLifecycleState::Submitted;
        add_line_comment(&mut app, "src/lib.rs", 11, line_locked);

        deliver_matching_pr_threads_event(
            &mut app,
            Err("network unavailable".to_string()),
            Ok(Vec::new()),
        );

        app.poll_pr_threads_events();

        let review = app.session.files.get(&PathBuf::from("src/lib.rs")).unwrap();
        let saved = review.line_comments.get(&11).unwrap();
        assert_eq!(saved.len(), 1);
        assert_eq!(saved[0].lifecycle_state, CommentLifecycleState::Submitted);
    }

    #[test]
    fn should_keep_locked_review_comments_when_remote_summary_refetch_fails() {
        let mut app = make_pr_app_with_single_modified_file("src/lib.rs");
        let mut review_locked = Comment::new("summary".to_string(), CommentType::Note, None);
        review_locked.lifecycle_state = CommentLifecycleState::Submitted;
        app.session.review_comments.push(review_locked);

        deliver_matching_pr_threads_event(
            &mut app,
            Ok(Vec::new()),
            Err("graphql unavailable".to_string()),
        );

        app.poll_pr_threads_events();

        assert_eq!(app.session.review_comments.len(), 1);
        assert_eq!(
            app.session.review_comments[0].lifecycle_state,
            CommentLifecycleState::Submitted
        );
    }

    #[test]
    fn should_emit_success_message_with_review_id_and_counts_for_published_submit() {
        // given
        let mut app = make_pr_app_with_single_modified_file("src/lib.rs");
        let comment = line_comment(LineSide::New, Some(11), None);
        let comment_id = comment.id.clone();
        add_line_comment(&mut app, "src/lib.rs", 11, comment);
        let in_flight = make_in_flight(
            SubmitEvent::Comment,
            &[comment_id.as_str()],
            "abcdef0123",
            2,
        );
        let response = make_response(123456, "https://example.com/r", "COMMENTED");
        // when
        app.finish_pr_submit(in_flight, Ok(response));
        // then
        let msg = app.message.as_ref().expect("info message");
        assert_eq!(msg.message_type, MessageType::Info);
        assert!(msg.content.contains("Submitted GitHub review #123456"));
        assert!(msg.content.contains("1 inline"));
        assert!(msg.content.contains("2 moved to summary"));
    }

    #[test]
    fn should_emit_draft_message_with_pr_url_for_draft_submit() {
        // given
        let mut app = make_pr_app_with_single_modified_file("src/lib.rs");
        let comment = line_comment(LineSide::New, Some(11), None);
        let comment_id = comment.id.clone();
        add_line_comment(&mut app, "src/lib.rs", 11, comment);
        let in_flight = make_in_flight(SubmitEvent::Draft, &[comment_id.as_str()], "abcdef0123", 0);
        let response = make_response(
            999,
            "https://github.com/agavra/tuicr/pull/125#pullrequestreview-999",
            "PENDING",
        );
        // when
        app.finish_pr_submit(in_flight, Ok(response));
        // then
        let msg = app.message.as_ref().expect("info message");
        assert!(msg.content.contains("Pushed pending GitHub review #999"));
        assert!(
            msg.content
                .contains("https://github.com/agavra/tuicr/pull/125"),
            "draft message should include the PR URL — got: {}",
            msg.content
        );
    }

    #[test]
    fn should_keep_comments_as_local_draft_on_submit_failure() {
        // given a local-draft comment in the session
        let mut app = make_pr_app_with_single_modified_file("src/lib.rs");
        let comment = line_comment(LineSide::New, Some(11), None);
        let comment_id = comment.id.clone();
        add_line_comment(&mut app, "src/lib.rs", 11, comment);
        let in_flight = make_in_flight(
            SubmitEvent::Comment,
            &[comment_id.as_str()],
            "abcdef0123",
            0,
        );
        // when — network failure
        app.finish_pr_submit(
            in_flight,
            Err(
                "Cannot submit review: GitHub token lacks pull request write permission."
                    .to_string(),
            ),
        );
        // then — the comment is still LocalDraft and editable
        let review = app.session.files.get(&PathBuf::from("src/lib.rs")).unwrap();
        let saved = &review.line_comments.get(&11).unwrap()[0];
        assert_eq!(saved.lifecycle_state, CommentLifecycleState::LocalDraft);
        assert!(saved.remote_review_id.is_none());
        // and — a sticky error message is set
        let msg = app.message.as_ref().expect("error message");
        assert_eq!(msg.message_type, MessageType::Error);
        assert!(msg.content.contains("Submit failed"));
        assert!(msg.content.contains("pull request write permission"));
    }

    #[test]
    fn should_discard_stale_submit_result_when_head_sha_changed() {
        // given a PR session pretending we already swapped heads after submit dispatched.
        let mut app = make_pr_app_with_single_modified_file("src/lib.rs");
        let comment = line_comment(LineSide::New, Some(11), None);
        let comment_id = comment.id.clone();
        add_line_comment(&mut app, "src/lib.rs", 11, comment);
        // record that a submit was in flight for an *older* head.
        let in_flight = make_in_flight(SubmitEvent::Comment, &[comment_id.as_str()], "OLD_HEAD", 0);
        app.pr_submit_state = Some(in_flight);
        let (tx, rx) = std::sync::mpsc::channel();
        app.pr_submit_rx = Some(rx);
        // simulate the bg thread coming back for the OLD head, but with
        // a fresh-head session note: poll path checks repository/pr_number/head_sha.
        tx.send(PrSubmitEvent::Done {
            repository: ForgeRepository::github("github.com", "agavra", "tuicr"),
            pr_number: 125,
            head_sha: "DIFFERENT_HEAD".to_string(),
            result: Ok(make_response(42, "u", "COMMENTED")),
        })
        .unwrap();
        drop(tx);
        // when
        app.poll_pr_submit_events();
        // then — comment lifecycle untouched, info message about discarded result.
        let review = app.session.files.get(&PathBuf::from("src/lib.rs")).unwrap();
        let saved = &review.line_comments.get(&11).unwrap()[0];
        assert_eq!(saved.lifecycle_state, CommentLifecycleState::LocalDraft);
        let msg = app.message.as_ref().expect("info message");
        assert!(
            msg.content.contains("Discarded stale submit result"),
            "got: {}",
            msg.content
        );
    }

    #[test]
    fn should_apply_result_via_poll_when_head_sha_matches() {
        // given a session with a local-draft comment ready to be locked
        let mut app = make_pr_app_with_single_modified_file("src/lib.rs");
        let comment = line_comment(LineSide::New, Some(11), None);
        let comment_id = comment.id.clone();
        add_line_comment(&mut app, "src/lib.rs", 11, comment);
        let in_flight = make_in_flight(
            SubmitEvent::Comment,
            &[comment_id.as_str()],
            "abcdef0123",
            0,
        );
        app.pr_submit_state = Some(in_flight);
        let (tx, rx) = std::sync::mpsc::channel();
        app.pr_submit_rx = Some(rx);
        tx.send(PrSubmitEvent::Done {
            repository: ForgeRepository::github("github.com", "agavra", "tuicr"),
            pr_number: 125,
            head_sha: "abcdef0123".to_string(),
            result: Ok(make_response(123, "u", "COMMENTED")),
        })
        .unwrap();
        drop(tx);
        // when
        app.poll_pr_submit_events();
        // then — lifecycle moved and the spinner state is cleared.
        assert!(app.pr_submit_state.is_none());
        assert!(app.pr_submit_rx.is_none());
        let review = app.session.files.get(&PathBuf::from("src/lib.rs")).unwrap();
        let saved = &review.line_comments.get(&11).unwrap()[0];
        assert_eq!(saved.lifecycle_state, CommentLifecycleState::Submitted);
        assert_eq!(saved.remote_review_id.as_deref(), Some("123"));
    }

    #[test]
    fn should_lock_file_level_comment_via_submit_success() {
        // given — a file-level comment lives in `file_comments`, not `line_comments`
        let mut app = make_pr_app_with_single_modified_file("src/lib.rs");
        let comment = Comment::new("file-level".to_string(), CommentType::Note, None);
        let comment_id = comment.id.clone();
        {
            let review = app
                .session
                .get_file_mut(&PathBuf::from("src/lib.rs"))
                .unwrap();
            review.file_comments.push(comment);
        }
        let in_flight = make_in_flight(
            SubmitEvent::Comment,
            &[comment_id.as_str()],
            "abcdef0123",
            0,
        );
        let response = make_response(7, "u", "COMMENTED");
        // when
        app.apply_submit_success(&in_flight, &response);
        // then
        let review = app.session.files.get(&PathBuf::from("src/lib.rs")).unwrap();
        assert_eq!(
            review.file_comments[0].lifecycle_state,
            CommentLifecycleState::Submitted
        );
        assert_eq!(
            review.file_comments[0].remote_review_id.as_deref(),
            Some("7")
        );
    }

    #[test]
    fn should_report_stale_head_when_current_differs_from_session_head() {
        let mut app = make_pr_app_with_single_modified_file("src/lib.rs");
        add_line_comment(
            &mut app,
            "src/lib.rs",
            11,
            line_comment(LineSide::New, Some(11), None),
        );
        // simulate a refresh having spotted a newer remote head
        app.current_pr_head = Some("ffff5678".to_string());
        app.start_submit(SubmitEvent::Comment);
        assert!(app.submit_head_is_stale());
    }

    #[test]
    fn should_report_head_not_stale_when_current_matches_session_head() {
        let mut app = make_pr_app_with_single_modified_file("src/lib.rs");
        add_line_comment(
            &mut app,
            "src/lib.rs",
            11,
            line_comment(LineSide::New, Some(11), None),
        );
        app.start_submit(SubmitEvent::Comment);
        assert!(!app.submit_head_is_stale());
    }

    #[test]
    fn should_detect_locked_comment_under_cursor_for_dd_path() {
        // given an app with a locked line comment registered against the
        // diff. We just verify the App helper sees the lock — exercising
        // the handler keypath itself is covered in integration tests.
        let mut app = make_pr_app_with_single_modified_file("src/lib.rs");
        let mut c = line_comment(LineSide::New, Some(11), None);
        c.lifecycle_state = CommentLifecycleState::PushedDraft;
        add_line_comment(&mut app, "src/lib.rs", 11, c);
        // No cursor positioning here — `cursor_on_locked_comment` resolves
        // through `find_comment_at_cursor` which depends on annotations.
        // The annotation indices use 0..N; with a single line comment on
        // line 11 there's exactly one LineComment annotation. We point the
        // cursor at it via diff_state.
        app.rebuild_annotations();
        // Find the LineComment annotation index.
        let idx = app
            .line_annotations
            .iter()
            .position(|a| matches!(a, AnnotatedLine::LineComment { .. }))
            .expect("expected a LineComment annotation");
        app.diff_state.cursor_line = idx;
        assert!(app.cursor_on_locked_comment());
    }
}

#[cfg(test)]
mod single_file_view_tests {
    use super::*;
    use crate::model::{DiffFile, DiffHunk, DiffLine, FileStatus, LineOrigin};
    use crate::vcs::traits::{VcsBackend, VcsInfo, VcsType};
    use std::fs;
    use std::path::PathBuf;

    struct StubVcs(VcsInfo);
    impl VcsBackend for StubVcs {
        fn info(&self) -> &VcsInfo {
            &self.0
        }
        fn get_working_tree_diff(
            &self,
            _hl: &crate::syntax::SyntaxHighlighter,
        ) -> crate::error::Result<Vec<DiffFile>> {
            Ok(Vec::new())
        }
        fn fetch_context_lines(
            &self,
            _path: &std::path::Path,
            _status: FileStatus,
            _ref_commit: Option<&str>,
            _start: u32,
            _end: u32,
        ) -> crate::error::Result<Vec<DiffLine>> {
            Ok(Vec::new())
        }
        fn file_line_count(
            &self,
            _path: &std::path::Path,
            _status: FileStatus,
            _ref_commit: Option<&str>,
        ) -> crate::error::Result<u32> {
            Ok(0)
        }
    }

    fn hunk(start: u32, count: u32) -> DiffHunk {
        let lines = (0..count)
            .map(|i| DiffLine {
                origin: LineOrigin::Context,
                content: format!("line {}", start + i),
                old_lineno: Some(start + i),
                new_lineno: Some(start + i),
                highlighted_spans: None,
            })
            .collect();
        DiffHunk {
            header: format!("@@ -{start},{count} +{start},{count} @@"),
            lines,
            old_start: start,
            old_count: count,
            new_start: start,
            new_count: count,
        }
    }

    fn file(path: &str, hunks: Vec<DiffHunk>) -> DiffFile {
        let content_hash = DiffFile::compute_content_hash(&hunks);
        DiffFile {
            old_path: None,
            new_path: Some(PathBuf::from(path)),
            status: FileStatus::Modified,
            hunks,
            is_binary: false,
            is_too_large: false,
            is_commit_message: false,
            content_hash,
        }
    }

    fn app_with(files: Vec<DiffFile>) -> App {
        app_with_root(PathBuf::from("/tmp"), files)
    }

    fn app_with_root(root_path: PathBuf, files: Vec<DiffFile>) -> App {
        let vcs_info = VcsInfo {
            root_path,
            head_commit: "head".into(),
            branch_name: Some("main".into()),
            vcs_type: VcsType::Git,
        };
        let session = ReviewSession::new(
            vcs_info.root_path.clone(),
            vcs_info.head_commit.clone(),
            vcs_info.branch_name.clone(),
            SessionDiffSource::WorkingTree,
        );
        App::build(
            Box::new(StubVcs(vcs_info.clone())),
            vcs_info,
            crate::theme::Theme::dark(),
            None,
            false,
            files,
            session,
            DiffSource::WorkingTree,
            InputMode::Normal,
            Vec::new(),
            None,
            None,
        )
        .expect("build app")
    }

    #[test]
    fn editor_target_uses_selected_file_list_row() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("main.rs");
        fs::write(&path, "fn main() {}\n").expect("write file");

        let mut app = app_with_root(
            dir.path().to_path_buf(),
            vec![file("main.rs", vec![hunk(1, 1)])],
        );
        app.focused_panel = FocusedPanel::FileList;
        app.queue_editor_for_focused_item();

        let target = app.take_pending_editor_target().expect("editor target");
        assert_eq!(target.path, path);
        assert_eq!(target.line, None);
    }

    #[test]
    fn edit_command_uses_selected_file_list_row() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("main.rs");
        fs::write(&path, "fn main() {}\n").expect("write file");

        let mut app = app_with_root(
            dir.path().to_path_buf(),
            vec![file("main.rs", vec![hunk(1, 1)])],
        );
        app.focused_panel = FocusedPanel::FileList;
        app.enter_command_mode();
        app.command_buffer = "edit".to_string();

        crate::handler::handle_command_action(&mut app, crate::input::Action::SubmitInput);

        let target = app.take_pending_editor_target().expect("editor target");
        assert_eq!(target.path, path);
        assert_eq!(target.line, None);
        assert_eq!(app.input_mode, InputMode::Normal);
    }

    #[test]
    fn editor_target_uses_diff_cursor_line() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("main.rs");
        fs::write(&path, "line 1\nline 2\nline 3\n").expect("write file");

        let mut app = app_with_root(
            dir.path().to_path_buf(),
            vec![file("main.rs", vec![hunk(1, 3)])],
        );
        app.focused_panel = FocusedPanel::Diff;
        app.diff_state.cursor_line = app
            .line_annotations
            .iter()
            .position(|annotation| {
                matches!(
                    annotation,
                    AnnotatedLine::DiffLine {
                        new_lineno: Some(2),
                        ..
                    }
                )
            })
            .expect("diff line annotation");
        app.queue_editor_for_focused_item();

        let target = app.take_pending_editor_target().expect("editor target");
        assert_eq!(target.path, path);
        assert_eq!(target.line, Some(2));
    }

    #[test]
    fn editor_target_warns_for_missing_local_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app_with_root(
            dir.path().to_path_buf(),
            vec![file("missing.rs", vec![hunk(1, 1)])],
        );
        app.focused_panel = FocusedPanel::Diff;

        app.queue_editor_for_focused_item();

        assert!(app.take_pending_editor_target().is_none());
        assert!(
            app.message
                .as_ref()
                .expect("warning")
                .content
                .contains("file does not exist")
        );
    }

    #[test]
    fn toggle_preserves_file_position() {
        let files = vec![
            file("a.rs", vec![hunk(1, 3)]),
            file("b.rs", vec![hunk(1, 3)]),
            file("c.rs", vec![hunk(1, 3)]),
        ];
        let mut app = app_with(files);
        app.diff_state.current_file_idx = 1;
        let expected_multi = app.calculate_file_scroll_offset(1);
        app.diff_state.scroll_offset = expected_multi;

        app.toggle_single_file_view();
        assert!(app.is_single_file_view);
        let expected_single = app.calculate_file_scroll_offset(1);
        assert_eq!(app.diff_state.scroll_offset, expected_single);
        assert_eq!(app.diff_state.cursor_line, expected_single);

        app.toggle_single_file_view();
        assert!(!app.is_single_file_view);
        let expected_back = app.calculate_file_scroll_offset(1);
        assert_eq!(app.diff_state.scroll_offset, expected_back);
        assert_eq!(app.diff_state.cursor_line, expected_back);
    }

    #[test]
    fn cursor_down_requires_two_presses_to_walk_to_next_file() {
        let files = vec![
            file("a.rs", vec![hunk(1, 3)]),
            file("b.rs", vec![hunk(1, 3)]),
        ];
        let mut app = app_with(files);
        app.is_single_file_view = true;
        app.diff_state.current_file_idx = 0;
        app.diff_state.cursor_line = app.max_cursor_line();
        let max_a = app.max_cursor_line();

        // First press at file end arms primed_walk_next and stays on max.
        app.cursor_down(1);
        assert_eq!(app.diff_state.current_file_idx, 0);
        assert_eq!(app.diff_state.cursor_line, max_a);
        assert!(app.primed_walk_next);

        // Second press consumes the prime and walks.
        app.cursor_down(1);
        assert_eq!(app.diff_state.current_file_idx, 1);
        assert!(!app.primed_walk_next);
    }

    #[test]
    fn cursor_up_requires_two_presses_to_walk_to_prev_file() {
        let files = vec![
            file("a.rs", vec![hunk(1, 3)]),
            file("b.rs", vec![hunk(1, 3)]),
        ];
        let mut app = app_with(files);
        app.is_single_file_view = true;
        app.diff_state.current_file_idx = 1;
        let file_top = app.calculate_file_scroll_offset(1);
        app.diff_state.cursor_line = file_top;

        // First press at file top arms primed_walk_prev and stays.
        app.cursor_up(1);
        assert_eq!(app.diff_state.current_file_idx, 1);
        assert_eq!(app.diff_state.cursor_line, file_top);
        assert!(app.primed_walk_prev);

        // Second press walks to the previous file.
        app.cursor_up(1);
        assert_eq!(app.diff_state.current_file_idx, 0);
        assert!(!app.primed_walk_prev);
    }

    #[test]
    fn primed_walk_clears_on_non_overflow_cursor_move() {
        let files = vec![
            file("a.rs", vec![hunk(1, 5)]),
            file("b.rs", vec![hunk(1, 5)]),
        ];
        let mut app = app_with(files);
        app.is_single_file_view = true;
        app.diff_state.current_file_idx = 0;
        app.diff_state.cursor_line = app.max_cursor_line();
        app.cursor_down(1); // arms
        assert!(app.primed_walk_next);

        // A non-overflow move (cursor up within file) clears the next prime.
        app.cursor_up(1);
        assert!(!app.primed_walk_next);
        assert_eq!(app.diff_state.current_file_idx, 0);
    }

    #[test]
    fn next_hunk_crosses_into_next_file_in_single_file_view() {
        let files = vec![
            file("a.rs", vec![hunk(1, 3), hunk(10, 3)]),
            file("b.rs", vec![hunk(1, 3), hunk(10, 3)]),
        ];
        let mut app = app_with(files);
        app.is_single_file_view = true;
        app.diff_state.current_file_idx = 0;
        app.rebuild_annotations();
        let positions = app.hunk_positions();
        let last_hunk = *positions.last().expect("a.rs has two hunks");
        app.diff_state.cursor_line = last_hunk;

        // From a.rs's last hunk, `]` should land on b.rs's first hunk.
        app.next_hunk();
        assert_eq!(app.diff_state.current_file_idx, 1);
        let new_first = *app.hunk_positions().first().expect("b.rs has hunks");
        assert_eq!(app.diff_state.cursor_line, new_first);
    }

    #[test]
    fn prev_hunk_crosses_into_prev_file_in_single_file_view() {
        let files = vec![
            file("a.rs", vec![hunk(1, 3), hunk(10, 3)]),
            file("b.rs", vec![hunk(1, 3), hunk(10, 3)]),
        ];
        let mut app = app_with(files);
        app.is_single_file_view = true;
        app.diff_state.current_file_idx = 1;
        app.rebuild_annotations();
        let first_hunk = *app.hunk_positions().first().expect("b.rs has hunks");
        app.diff_state.cursor_line = first_hunk;

        // From b.rs's first hunk, `[` should land on a.rs's last hunk.
        app.prev_hunk();
        assert_eq!(app.diff_state.current_file_idx, 0);
        let new_last = *app.hunk_positions().last().expect("a.rs has hunks");
        assert_eq!(app.diff_state.cursor_line, new_last);
    }

    #[test]
    fn held_key_does_not_walk_when_keyboard_enhancement_supported() {
        // Simulates kitty REPORT_EVENT_TYPES: held-j auto-repeats arm the
        // prime but the release flag never trips, so consecutive
        // cursor_down calls park on max forever.
        let files = vec![
            file("a.rs", vec![hunk(1, 3)]),
            file("b.rs", vec![hunk(1, 3)]),
        ];
        let mut app = app_with(files);
        app.supports_keyboard_enhancement = true;
        app.is_single_file_view = true;
        app.diff_state.current_file_idx = 0;
        app.diff_state.cursor_line = app.max_cursor_line();
        let max_a = app.max_cursor_line();

        // 10 consecutive presses (no release) stay parked on max.
        for _ in 0..10 {
            app.cursor_down(1);
        }
        assert_eq!(app.diff_state.current_file_idx, 0);
        assert_eq!(app.diff_state.cursor_line, max_a);
        assert!(app.primed_walk_next);
        assert!(!app.down_released_since_arm);
    }

    #[test]
    fn release_then_press_walks_when_keyboard_enhancement_supported() {
        let files = vec![
            file("a.rs", vec![hunk(1, 3)]),
            file("b.rs", vec![hunk(1, 3)]),
        ];
        let mut app = app_with(files);
        app.supports_keyboard_enhancement = true;
        app.is_single_file_view = true;
        app.diff_state.current_file_idx = 0;
        app.diff_state.cursor_line = app.max_cursor_line();

        // First press: arm.
        app.cursor_down(1);
        assert!(app.primed_walk_next);
        assert_eq!(app.diff_state.current_file_idx, 0);

        // Simulate a Down-Release event from the main loop.
        app.down_released_since_arm = true;

        // Second press: walks.
        app.cursor_down(1);
        assert_eq!(app.diff_state.current_file_idx, 1);
        assert!(!app.primed_walk_next);
        assert!(!app.down_released_since_arm);
    }

    #[test]
    fn effective_file_height_is_zero_for_non_current_in_single_file_view() {
        let files = vec![
            file("a.rs", vec![hunk(1, 3)]),
            file("b.rs", vec![hunk(1, 3)]),
        ];
        let mut app = app_with(files);
        app.is_single_file_view = true;
        app.diff_state.current_file_idx = 0;
        let other = &app.diff_files[1].clone();
        assert_eq!(app.effective_file_height(1, other), 0);
        let current = &app.diff_files[0].clone();
        assert!(app.effective_file_height(0, current) > 0);
    }
}
