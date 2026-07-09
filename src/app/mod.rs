use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use chrono::Utc;
use ratatui::style::Color;

use crate::comment_vim::CommentVimEditor;
use crate::config::CommentTypeConfig;
use crate::editor::EditorTarget;
use crate::error::{Result, TuicrError};
use crate::forge::context::{ContextProvider, ForgeContextProvider, VcsContextProvider};
use crate::forge::selector::PullRequestsTab;
use crate::forge::traits::{ForgeBackend, ForgeRepository};
use crate::model::review::FileReview;
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct SinceLastReviewSelection {
    range: Option<(usize, usize)>,
    reviewed_index: usize,
    message: String,
}

fn commits_since_last_review_selection(
    commits_newest_first: &[crate::forge::traits::PullRequestCommit],
    review_metadata: &crate::forge::traits::PullRequestReviewMetadata,
) -> Option<SinceLastReviewSelection> {
    let viewer = review_metadata.viewer_login.as_deref()?;
    let last_review = review_metadata
        .reviews
        .iter()
        .filter(|review| {
            review
                .author
                .as_deref()
                .is_some_and(|author| author.eq_ignore_ascii_case(viewer))
        })
        .filter(|review| review.submitted_at.is_some() && review.commit_oid.is_some())
        .max_by(|a, b| a.submitted_at.cmp(&b.submitted_at))?;

    let reviewed_commit = last_review.commit_oid.as_deref()?;
    let reviewed_index = commits_newest_first
        .iter()
        .position(|commit| commit.oid == reviewed_commit)?;

    if reviewed_index == 0 {
        return Some(SinceLastReviewSelection {
            range: None,
            reviewed_index,
            message: "No commits since your last review".to_string(),
        });
    }

    let count = reviewed_index;
    let noun = if count == 1 { "commit" } else { "commits" };
    Some(SinceLastReviewSelection {
        range: Some((0, reviewed_index - 1)),
        reviewed_index,
        message: format!("Showing {count} {noun} since your last review — press Enter to see all"),
    })
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

/// True for rendered lines the cursor should never rest on — spacing between
/// files and file header rows.
fn is_decoration(annotation: &AnnotatedLine) -> bool {
    matches!(
        annotation,
        AnnotatedLine::Spacing | AnnotatedLine::FileHeader { .. }
    )
}

/// Walk `start` forward (capped at `max_line`) to the nearest non-decoration
/// annotation so scroll and jump motions land on actionable content.
fn skip_decoration_forward(annotations: &[AnnotatedLine], start: usize, max_line: usize) -> usize {
    let mut line = start;
    while line < max_line && annotations.get(line).is_some_and(is_decoration) {
        line += 1;
    }
    line
}

/// Walk `start` backward to the nearest non-decoration annotation.
fn skip_decoration_backward(annotations: &[AnnotatedLine], start: usize) -> usize {
    let mut line = start;
    while line > 0 && annotations.get(line).is_some_and(is_decoration) {
        line -= 1;
    }
    line
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
                crate::forge::traits::PullRequestReviewMetadata,
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
                crate::forge::traits::PullRequestReviewMetadata,
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

/// Pending "press again to confirm" state for the vim comment box. A first
/// plain `Enter`/`Esc` in Normal mode arms `Save`/`Cancel` and shows a header
/// hint; a second consecutive press performs it. Any other key resets to `None`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CommentVimPending {
    #[default]
    None,
    Save,
    Cancel,
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
    /// Config `comment_vim`: vim modal editing in the comment box.
    pub comment_vim_enabled: bool,
    /// Spaces inserted by Tab while typing in the vim comment box (config
    /// `comment_tab_width`, default 4).
    pub comment_tab_width: usize,
    /// Active vim overlay (only while in comment mode with vim on); synced into
    /// `comment_buffer`/`comment_cursor`, which stay canonical for rendering.
    pub comment_vim_editor: Option<CommentVimEditor>,
    /// In-progress `:` command-line in vim Normal mode (without the leading
    /// `:`); `Some("")` right after `:` is pressed. `:w` saves, `:q` cancels.
    pub comment_vim_command: Option<String>,
    /// Pending double-press confirm in vim Normal mode: a first plain
    /// `Enter`/`Esc` arms Save/Cancel (with a header hint), the second performs
    /// it (`:w`/`:q`). Any other key resets it.
    pub comment_vim_pending: CommentVimPending,
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
    /// Index in `pr_commits`/`review_commits` of the newest commit covered
    /// by the viewer's latest submitted review. Commits at this index and
    /// older get a reviewed marker in the inline selector.
    pub pr_last_reviewed_commit_index: Option<usize>,
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

mod annotations;
mod comment_vim;
mod comments;
mod commits;
mod diff_load;
mod gaps;
mod init;
mod modes;
mod navigation;
mod pr;
mod reviewed;
mod search;
mod session;
mod submit;
mod tree;
mod visual;

#[cfg(test)]
mod tests;
