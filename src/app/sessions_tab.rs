//! State for the Sessions tab in the review target selector.
//!
//! The tab lists persisted review sessions for the current checkout so a
//! review can be resumed by picking it, rather than by remembering the commit
//! range or working-tree flags it was opened with. Rows come from
//! [`ReviewStore::list_sessions_for_repo`], which is already newest-first.

use crate::review_store::{SessionKind, SessionSummary};

/// Rows for the Sessions tab, plus cursor and viewport state.
///
/// Filled when the tab opens; the listing reads only the local manifest.
#[derive(Debug, Default)]
pub struct SessionsTab {
    rows: Vec<SessionSummary>,
    error: Option<String>,
    /// When true the listing also shows sessions with no comments and no
    /// reviewed files. They hold nothing to resume, so they are hidden by
    /// default; revealing them is what makes them reachable for `dd`.
    show_empty: bool,
    /// Checkout the listing is scoped to, resolved once at load. Rendering
    /// reads this every frame, so it must not re-derive it: the lookup reaches
    /// git2 repository discovery and the `origin` remote.
    scope: String,
    cursor: usize,
    scroll: usize,
}

impl SessionsTab {
    /// Whether empty sessions are currently listed.
    pub fn show_empty(&self) -> bool {
        self.show_empty
    }

    /// Flip the empty-session filter. The caller reloads the listing, since
    /// the filter is applied while rows are gathered, not while they render.
    pub fn toggle_show_empty(&mut self) {
        self.show_empty = !self.show_empty;
    }

    /// Drop the row at `index`, keeping the cursor on a valid neighbour.
    ///
    /// Used after a session file is deleted so the list reflects the removal
    /// without a full reload, which would reset the cursor to the top.
    pub fn remove_row(&mut self, index: usize) {
        if index >= self.rows.len() {
            return;
        }
        self.rows.remove(index);
        self.cursor = self.cursor.min(self.rows.len().saturating_sub(1));
    }

    /// Replace the rows with a fresh listing, resetting the cursor.
    pub fn apply_load(&mut self, result: std::result::Result<Vec<SessionSummary>, String>) {
        self.cursor = 0;
        self.scroll = 0;
        match result {
            Ok(rows) => {
                self.rows = rows;
                self.error = None;
            }
            Err(message) => {
                self.rows = Vec::new();
                self.error = Some(message);
            }
        }
    }

    /// Name the listing is scoped to, for the panel title.
    pub fn scope(&self) -> &str {
        &self.scope
    }

    pub fn set_scope(&mut self, scope: String) {
        self.scope = scope;
    }

    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    pub fn rows(&self) -> &[SessionSummary] {
        &self.rows
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn scroll(&self) -> usize {
        self.scroll
    }

    /// The session under the cursor, if any.
    pub fn cursor_session(&self) -> Option<&SessionSummary> {
        self.rows.get(self.cursor)
    }

    pub fn cursor_up(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    pub fn cursor_down(&mut self) {
        if !self.rows.is_empty() {
            self.cursor = (self.cursor + 1).min(self.rows.len() - 1);
        }
    }

    /// Keep the cursor inside the viewport after a move or a resize.
    pub fn ensure_cursor_visible(&mut self, height: usize) {
        if height == 0 {
            return;
        }
        if self.cursor < self.scroll {
            self.scroll = self.cursor;
        } else if self.cursor >= self.scroll + height {
            self.scroll = self.cursor + 1 - height;
        }
    }
}

/// Short label for a session's kind, shown in the row's left column.
pub fn kind_label(kind: SessionKind) -> &'static str {
    match kind {
        SessionKind::Local => "local",
        SessionKind::Pr => "pr",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::review_store::SessionRef;
    use chrono::Utc;

    fn summary(slug: &str) -> SessionSummary {
        SessionSummary {
            session_ref: SessionRef::from_path(format!("/tmp/{slug}.json")),
            slug: slug.to_string(),
            kind: SessionKind::Local,
            updated_at: Utc::now(),
            comment_count: 0,
            reviewed_count: 0,
            file_count: 1,
            anchor: "main".to_string(),
            active: false,
        }
    }

    #[test]
    fn should_start_empty() {
        let tab = SessionsTab::default();
        assert!(tab.is_empty());
        assert!(tab.cursor_session().is_none());
    }

    #[test]
    fn should_apply_rows_and_select_first() {
        let mut tab = SessionsTab::default();
        tab.apply_load(Ok(vec![summary("a"), summary("b")]));
        assert_eq!(tab.cursor_session().map(|s| s.slug.as_str()), Some("a"));
    }

    #[test]
    fn should_clamp_cursor_at_both_ends() {
        let mut tab = SessionsTab::default();
        tab.apply_load(Ok(vec![summary("a"), summary("b")]));
        tab.cursor_up();
        assert_eq!(tab.cursor(), 0);
        tab.cursor_down();
        tab.cursor_down();
        tab.cursor_down();
        assert_eq!(tab.cursor(), 1);
    }

    #[test]
    fn should_not_move_cursor_when_empty() {
        let mut tab = SessionsTab::default();
        tab.apply_load(Ok(Vec::new()));
        tab.cursor_down();
        assert_eq!(tab.cursor(), 0);
        assert!(tab.cursor_session().is_none());
    }

    #[test]
    fn should_record_error_and_drop_rows() {
        let mut tab = SessionsTab::default();
        tab.apply_load(Ok(vec![summary("a")]));
        tab.apply_load(Err("boom".to_string()));
        assert!(tab.is_empty());
        assert_eq!(tab.error(), Some("boom"));
    }

    #[test]
    fn should_scroll_viewport_to_follow_cursor() {
        let mut tab = SessionsTab::default();
        tab.apply_load(Ok((0..10).map(|i| summary(&i.to_string())).collect()));
        for _ in 0..4 {
            tab.cursor_down();
        }
        tab.ensure_cursor_visible(3);
        assert_eq!(tab.scroll(), 2);
        for _ in 0..4 {
            tab.cursor_up();
        }
        tab.ensure_cursor_visible(3);
        assert_eq!(tab.scroll(), 0);
    }
}
