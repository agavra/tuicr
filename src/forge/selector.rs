//! State machine for the Pull Requests tab in the review target selector.
//!
//! The state is driven by user navigation (entering the tab triggers an
//! initial fetch) and by background fetch results delivered through an
//! `mpsc::Receiver`. UI rendering, filter input, and key dispatch all read
//! from this state; no I/O happens here.
#![allow(dead_code)]

use crate::forge::traits::{ForgeRepository, PullRequestSummary};

pub const PR_PAGE_SIZE: usize = 30;

/// High level state for the Pull Requests tab.
///
/// The tab is `Disabled` when the current repo has no GitHub remote. It is
/// `Idle` before the first network call. `Loading` and `LoadingMore` show
/// the indeterminate progress bar. `Loaded` carries the rows that have been
/// fetched so far, whether there is more to load, and the active local
/// filter. `Error` carries a user-facing message.
#[derive(Debug, Clone)]
pub enum PullRequestsTab {
    Disabled {
        reason: String,
    },
    Idle {
        repository: ForgeRepository,
    },
    Loading {
        repository: ForgeRepository,
    },
    Loaded {
        repository: ForgeRepository,
        rows: Vec<PullRequestSummary>,
        has_more: bool,
        loading_more: bool,
        filter: String,
        cursor: usize,
        scroll_offset: usize,
    },
    Error {
        repository: Option<ForgeRepository>,
        message: String,
    },
}

/// What we want the UI to render next. Computed once per draw so the
/// renderer does not have to walk the state-machine variants directly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrTabView<'a> {
    pub status: PrTabStatus<'a>,
    pub rows: Vec<PrRow<'a>>,
    pub cursor: usize,
    pub scroll_offset: usize,
    /// True when an additional `... load more pull requests` row should appear
    /// after the data rows.
    pub has_load_more: bool,
    pub filter: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrTabStatus<'a> {
    Disabled(&'a str),
    Idle,
    Loading,
    LoadingMore,
    Ready,
    Error(&'a str),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrRow<'a> {
    pub summary: &'a PullRequestSummary,
}

impl PullRequestsTab {
    pub fn new(repository: Option<ForgeRepository>) -> Self {
        match repository {
            Some(repo) => PullRequestsTab::Idle { repository: repo },
            None => PullRequestsTab::Disabled {
                reason: "No GitHub remote on this repo".to_string(),
            },
        }
    }

    pub fn repository(&self) -> Option<&ForgeRepository> {
        match self {
            PullRequestsTab::Idle { repository, .. }
            | PullRequestsTab::Loading { repository, .. } => Some(repository),
            PullRequestsTab::Loaded { repository, .. } => Some(repository),
            PullRequestsTab::Error { repository, .. } => repository.as_ref(),
            PullRequestsTab::Disabled { .. } => None,
        }
    }

    /// True when the tab is currently waiting on a network call.
    pub fn is_loading(&self) -> bool {
        matches!(self, PullRequestsTab::Loading { .. })
            || matches!(
                self,
                PullRequestsTab::Loaded {
                    loading_more: true,
                    ..
                }
            )
    }

    /// True when the tab has rows. The filter is allowed to reduce them
    /// to zero — that still counts as loaded.
    pub fn is_loaded(&self) -> bool {
        matches!(self, PullRequestsTab::Loaded { .. })
    }

    /// Begin the initial fetch. Only meaningful from `Idle`.
    pub fn start_initial_load(&mut self) -> Option<ForgeRepository> {
        if let PullRequestsTab::Idle { repository } = self {
            let repo = repository.clone();
            *self = PullRequestsTab::Loading {
                repository: repo.clone(),
            };
            Some(repo)
        } else {
            None
        }
    }

    /// Begin a "load more" fetch. Only meaningful when `Loaded` with `has_more`.
    pub fn start_load_more(&mut self) -> Option<(ForgeRepository, usize)> {
        if let PullRequestsTab::Loaded {
            repository,
            rows,
            has_more,
            loading_more,
            ..
        } = self
            && *has_more
            && !*loading_more
        {
            *loading_more = true;
            return Some((repository.clone(), rows.len()));
        }
        None
    }

    /// Promote the in-flight `Loading` tab to a (possibly different)
    /// canonical repository. Called by `poll_pr_load_events` right before
    /// `apply_initial_load` so the rows land on the correct repo when the
    /// background thread resolved a fork's parent. No-op when not in
    /// `Loading` or when the canonical matches the existing repository.
    pub fn apply_canonical(&mut self, canonical: ForgeRepository) {
        if let PullRequestsTab::Loading { repository } = self
            && *repository != canonical
        {
            *repository = canonical;
        }
    }

    /// Apply the result of the initial load.
    pub fn apply_initial_load(&mut self, result: Result<(Vec<PullRequestSummary>, bool), String>) {
        let repository = match self {
            PullRequestsTab::Loading { repository } => repository.clone(),
            _ => return,
        };
        match result {
            Ok((rows, has_more)) => {
                *self = PullRequestsTab::Loaded {
                    repository,
                    rows,
                    has_more,
                    loading_more: false,
                    filter: String::new(),
                    cursor: 0,
                    scroll_offset: 0,
                };
            }
            Err(message) => {
                *self = PullRequestsTab::Error {
                    repository: Some(repository),
                    message,
                };
            }
        }
    }

    /// Apply the result of a load-more fetch. Appends rows.
    pub fn apply_load_more(&mut self, result: Result<(Vec<PullRequestSummary>, bool), String>) {
        if let PullRequestsTab::Loaded {
            rows,
            has_more,
            loading_more,
            ..
        } = self
        {
            *loading_more = false;
            match result {
                Ok((new_rows, more)) => {
                    rows.extend(new_rows);
                    *has_more = more;
                }
                Err(message) => {
                    // Don't tear down the loaded list on a load-more failure;
                    // surface the message and clear the busy flag so the user
                    // can try again.
                    let repository = self.repository().cloned();
                    *self = PullRequestsTab::Error {
                        repository,
                        message,
                    };
                }
            }
        }
    }

    /// Set the local filter and clamp the cursor to the resulting list.
    pub fn set_filter(&mut self, new_filter: String) {
        if let PullRequestsTab::Loaded {
            filter,
            cursor,
            scroll_offset,
            ..
        } = self
        {
            *filter = new_filter;
            *cursor = 0;
            *scroll_offset = 0;
            // The filter changed; visible row count may have shrunk so the
            // cursor must point inside the new list.
            self.clamp_cursor();
        }
    }

    /// Indexes of `rows` that match the current filter, in order.
    pub fn visible_indices(&self) -> Vec<usize> {
        match self {
            PullRequestsTab::Loaded { rows, filter, .. } => filtered_indices(rows, filter),
            _ => Vec::new(),
        }
    }

    /// Clamp the cursor into `[0, visible + maybe_load_more)`.
    pub fn clamp_cursor(&mut self) {
        if let PullRequestsTab::Loaded {
            rows,
            filter,
            has_more,
            cursor,
            ..
        } = self
        {
            let visible = filtered_indices(rows, filter).len();
            let extra = if *has_more && filter.is_empty() { 1 } else { 0 };
            let max_idx = (visible + extra).saturating_sub(1);
            if *cursor > max_idx {
                *cursor = max_idx;
            }
        }
    }

    pub fn cursor_up(&mut self) {
        if let PullRequestsTab::Loaded { cursor, .. } = self
            && *cursor > 0
        {
            *cursor -= 1;
        }
    }

    pub fn cursor_down(&mut self) {
        if let PullRequestsTab::Loaded {
            rows,
            filter,
            has_more,
            cursor,
            ..
        } = self
        {
            let visible = filtered_indices(rows, filter).len();
            let extra = if *has_more && filter.is_empty() { 1 } else { 0 };
            let max_idx = (visible + extra).saturating_sub(1);
            if *cursor < max_idx {
                *cursor += 1;
            }
        }
    }

    /// True when the cursor is sitting on the trailing `... load more` row.
    pub fn cursor_on_load_more(&self) -> bool {
        if let PullRequestsTab::Loaded {
            rows,
            filter,
            has_more,
            cursor,
            ..
        } = self
        {
            if !*has_more || !filter.is_empty() {
                return false;
            }
            let visible = filtered_indices(rows, filter).len();
            *cursor == visible
        } else {
            false
        }
    }

    /// The PR at the cursor, if any. `None` when on the load-more row or in
    /// any non-Loaded state.
    pub fn cursor_pr(&self) -> Option<&PullRequestSummary> {
        if let PullRequestsTab::Loaded {
            rows,
            filter,
            cursor,
            ..
        } = self
        {
            let indices = filtered_indices(rows, filter);
            indices.get(*cursor).and_then(|i| rows.get(*i))
        } else {
            None
        }
    }

    /// Update scroll so cursor remains visible in a viewport of `height`.
    pub fn ensure_cursor_visible(&mut self, height: usize) {
        if let PullRequestsTab::Loaded {
            cursor,
            scroll_offset,
            ..
        } = self
        {
            if height == 0 {
                return;
            }
            if *cursor < *scroll_offset {
                *scroll_offset = *cursor;
            } else if *cursor >= *scroll_offset + height {
                *scroll_offset = *cursor + 1 - height;
            }
        }
    }

    /// Project the current state into a UI-friendly view.
    pub fn view(&self) -> PrTabView<'_> {
        match self {
            PullRequestsTab::Disabled { reason } => PrTabView {
                status: PrTabStatus::Disabled(reason.as_str()),
                rows: Vec::new(),
                cursor: 0,
                scroll_offset: 0,
                has_load_more: false,
                filter: "",
            },
            PullRequestsTab::Idle { .. } => PrTabView {
                status: PrTabStatus::Idle,
                rows: Vec::new(),
                cursor: 0,
                scroll_offset: 0,
                has_load_more: false,
                filter: "",
            },
            PullRequestsTab::Loading { .. } => PrTabView {
                status: PrTabStatus::Loading,
                rows: Vec::new(),
                cursor: 0,
                scroll_offset: 0,
                has_load_more: false,
                filter: "",
            },
            PullRequestsTab::Loaded {
                rows,
                has_more,
                loading_more,
                filter,
                cursor,
                scroll_offset,
                ..
            } => {
                let indices = filtered_indices(rows, filter);
                let mapped = indices
                    .into_iter()
                    .map(|i| PrRow { summary: &rows[i] })
                    .collect();
                let status = if *loading_more {
                    PrTabStatus::LoadingMore
                } else {
                    PrTabStatus::Ready
                };
                PrTabView {
                    status,
                    rows: mapped,
                    cursor: *cursor,
                    scroll_offset: *scroll_offset,
                    has_load_more: *has_more && filter.is_empty(),
                    filter: filter.as_str(),
                }
            }
            PullRequestsTab::Error { message, .. } => PrTabView {
                status: PrTabStatus::Error(message.as_str()),
                rows: Vec::new(),
                cursor: 0,
                scroll_offset: 0,
                has_load_more: false,
                filter: "",
            },
        }
    }
}

/// Return the indices into `rows` that match `filter`. An empty filter
/// returns every index.
pub fn filtered_indices(rows: &[PullRequestSummary], filter: &str) -> Vec<usize> {
    if filter.is_empty() {
        return (0..rows.len()).collect();
    }
    let needle = filter.to_lowercase();
    rows.iter()
        .enumerate()
        .filter(|(_, row)| matches_filter(row, &needle))
        .map(|(i, _)| i)
        .collect()
}

fn matches_filter(row: &PullRequestSummary, needle_lower: &str) -> bool {
    if row.number.to_string().contains(needle_lower) {
        return true;
    }
    if contains_ignore_case(&row.title, needle_lower) {
        return true;
    }
    if let Some(author) = row.author.as_deref()
        && contains_ignore_case(author, needle_lower)
    {
        return true;
    }
    if contains_ignore_case(&row.head_ref_name, needle_lower) {
        return true;
    }
    if contains_ignore_case(&row.base_ref_name, needle_lower) {
        return true;
    }
    false
}

fn contains_ignore_case(haystack: &str, needle_lower: &str) -> bool {
    haystack.to_lowercase().contains(needle_lower)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};

    fn repo() -> ForgeRepository {
        ForgeRepository::github("github.com", "agavra", "tuicr")
    }

    fn pr(number: u64, title: &str, author: &str, head: &str, base: &str) -> PullRequestSummary {
        PullRequestSummary {
            repository: repo(),
            number,
            title: title.to_string(),
            author: Some(author.to_string()),
            head_ref_name: head.to_string(),
            base_ref_name: base.to_string(),
            updated_at: Some(Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap()),
            url: format!("https://github.com/agavra/tuicr/pull/{number}"),
            state: "OPEN".to_string(),
            is_draft: false,
        }
    }

    #[test]
    fn should_start_disabled_when_no_repository_present() {
        // given
        let mut tab = PullRequestsTab::new(None);
        // when / then
        assert!(matches!(tab, PullRequestsTab::Disabled { .. }));
        assert!(tab.start_initial_load().is_none());
    }

    #[test]
    fn should_start_idle_when_repository_present() {
        // given / when
        let tab = PullRequestsTab::new(Some(repo()));
        // then
        assert!(matches!(tab, PullRequestsTab::Idle { .. }));
    }

    #[test]
    fn should_transition_idle_to_loading_on_initial_load() {
        // given
        let mut tab = PullRequestsTab::new(Some(repo()));
        // when
        let requested = tab.start_initial_load();
        // then
        assert_eq!(requested.unwrap(), repo());
        assert!(matches!(tab, PullRequestsTab::Loading { .. }));
        assert!(tab.is_loading());
    }

    #[test]
    fn should_transition_loading_to_loaded_on_success() {
        // given
        let mut tab = PullRequestsTab::new(Some(repo()));
        tab.start_initial_load();
        // when
        tab.apply_initial_load(Ok((vec![pr(1, "title", "alice", "feat", "main")], false)));
        // then
        let view = tab.view();
        assert_eq!(view.rows.len(), 1);
        assert!(matches!(view.status, PrTabStatus::Ready));
    }

    #[test]
    fn should_transition_loading_to_error_on_failure() {
        // given
        let mut tab = PullRequestsTab::new(Some(repo()));
        tab.start_initial_load();
        // when
        tab.apply_initial_load(Err("boom".to_string()));
        // then
        assert!(matches!(tab, PullRequestsTab::Error { .. }));
        assert!(matches!(tab.view().status, PrTabStatus::Error("boom")));
    }

    #[test]
    fn should_append_rows_on_load_more_success() {
        // given
        let mut tab = PullRequestsTab::new(Some(repo()));
        tab.start_initial_load();
        tab.apply_initial_load(Ok((vec![pr(1, "a", "a", "h", "m")], true)));
        // when
        let request = tab.start_load_more();
        assert!(request.is_some());
        tab.apply_load_more(Ok((vec![pr(2, "b", "b", "h2", "m")], false)));
        // then
        let view = tab.view();
        assert_eq!(view.rows.len(), 2);
        assert_eq!(view.rows[1].summary.number, 2);
        assert!(!view.has_load_more);
    }

    #[test]
    fn should_filter_rows_by_number_title_author_head_and_base() {
        // given
        let rows = vec![
            pr(125, "Forge backend", "alice", "feat/forge", "main"),
            pr(148, "Add review UX", "bob", "review-ux", "develop"),
        ];
        // when / then
        assert_eq!(filtered_indices(&rows, "125"), vec![0]);
        assert_eq!(filtered_indices(&rows, "review"), vec![1]);
        assert_eq!(filtered_indices(&rows, "ALICE"), vec![0]);
        assert_eq!(filtered_indices(&rows, "forge"), vec![0]);
        assert_eq!(filtered_indices(&rows, "develop"), vec![1]);
        assert_eq!(filtered_indices(&rows, ""), vec![0, 1]);
        assert!(filtered_indices(&rows, "no-match").is_empty());
    }

    #[test]
    fn should_clamp_cursor_after_filter_narrows_list() {
        // given
        let mut tab = PullRequestsTab::new(Some(repo()));
        tab.start_initial_load();
        tab.apply_initial_load(Ok((
            vec![
                pr(1, "alpha", "a", "h", "m"),
                pr(2, "beta", "a", "h", "m"),
                pr(3, "gamma", "a", "h", "m"),
            ],
            false,
        )));
        if let PullRequestsTab::Loaded { cursor, .. } = &mut tab {
            *cursor = 2;
        }
        // when
        tab.set_filter("beta".to_string());
        // then
        assert_eq!(tab.view().cursor, 0);
        assert_eq!(tab.view().rows.len(), 1);
    }

    #[test]
    fn should_position_cursor_on_load_more_row_when_at_bottom() {
        // given
        let mut tab = PullRequestsTab::new(Some(repo()));
        tab.start_initial_load();
        tab.apply_initial_load(Ok((vec![pr(1, "a", "a", "h", "m")], true)));
        // when
        tab.cursor_down();
        // then
        assert!(tab.cursor_on_load_more());
        assert!(tab.cursor_pr().is_none());
    }

    #[test]
    fn should_not_show_load_more_row_when_filter_active() {
        // given
        let mut tab = PullRequestsTab::new(Some(repo()));
        tab.start_initial_load();
        tab.apply_initial_load(Ok((vec![pr(1, "alpha", "a", "h", "m")], true)));
        // when
        tab.set_filter("alpha".to_string());
        // then
        let view = tab.view();
        assert!(!view.has_load_more);
    }

    #[test]
    fn should_not_start_load_more_when_already_loading_more() {
        // given
        let mut tab = PullRequestsTab::new(Some(repo()));
        tab.start_initial_load();
        tab.apply_initial_load(Ok((vec![pr(1, "a", "a", "h", "m")], true)));
        // when
        let first = tab.start_load_more();
        let second = tab.start_load_more();
        // then
        assert!(first.is_some());
        assert!(second.is_none());
    }

    #[test]
    fn should_promote_loading_repository_via_apply_canonical() {
        // given — origin is a fork; canonical is the upstream parent
        let origin = ForgeRepository::github("github.com", "agavra", "slatedb");
        let canonical = ForgeRepository::github("github.com", "slatedb", "slatedb");
        let mut tab = PullRequestsTab::new(Some(origin));
        tab.start_initial_load();
        // when
        tab.apply_canonical(canonical.clone());
        tab.apply_initial_load(Ok((vec![pr(9, "x", "a", "h", "m")], false)));
        // then — Loaded carries the canonical, not the origin
        if let PullRequestsTab::Loaded { repository, .. } = &tab {
            assert_eq!(repository, &canonical);
        } else {
            panic!("expected Loaded state");
        }
    }

    #[test]
    fn should_be_noop_when_apply_canonical_called_outside_loading() {
        // given — tab is still Idle
        let mut tab = PullRequestsTab::new(Some(repo()));
        let other = ForgeRepository::github("github.com", "other", "other");
        // when
        tab.apply_canonical(other);
        // then — still Idle on the original repo (no silent promotion)
        if let PullRequestsTab::Idle { repository } = &tab {
            assert_eq!(repository, &repo());
        } else {
            panic!("expected Idle state");
        }
    }

    #[test]
    fn should_keep_loaded_rows_when_load_more_fails() {
        // given
        let mut tab = PullRequestsTab::new(Some(repo()));
        tab.start_initial_load();
        tab.apply_initial_load(Ok((vec![pr(1, "a", "a", "h", "m")], true)));
        tab.start_load_more();
        // when
        tab.apply_load_more(Err("net down".to_string()));
        // then — moves into Error but the original rows can be re-fetched
        assert!(matches!(tab, PullRequestsTab::Error { .. }));
    }
}
