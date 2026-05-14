//! Remote review comment/thread models.
//!
//! These types carry existing GitHub review discussions into the App for
//! read-only display, filtering, and export. They are deliberately
//! source-of-truth-on-remote: we never mutate, reply to, or persist them
//! locally past the in-memory cache.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Which side of the diff a remote comment anchors to.
///
/// Mirrors GitHub's submission model: `RIGHT` is the head side (added/context
/// lines), `LEFT` is the base side (deleted lines).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RemoteCommentSide {
    Right,
    Left,
}

impl RemoteCommentSide {
    pub fn parse(value: &str) -> Self {
        match value.to_ascii_uppercase().as_str() {
            "LEFT" => RemoteCommentSide::Left,
            _ => RemoteCommentSide::Right,
        }
    }
}

/// A single remote review comment, fetched from a forge.
///
/// Anchor fields (`path`, `line`, `side`) live on the parent
/// `RemoteReviewThread`, not on each comment, mirroring GitHub's GraphQL
/// schema where `PullRequestReviewComment` does not carry these directly.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteReviewComment {
    /// Forge-assigned comment node ID (opaque string).
    pub id: String,
    /// Login/handle of the comment author, when available.
    pub author: Option<String>,
    /// Markdown body as written on the forge.
    pub body: String,
    pub created_at: Option<DateTime<Utc>>,
    /// For reply comments, the ID of the parent comment.
    pub in_reply_to: Option<String>,
    /// Permalink to the comment on the forge.
    pub url: String,
}

/// A discussion thread on a forge — one root comment plus zero or more replies.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteReviewThread {
    /// Forge-assigned thread node ID.
    pub id: String,
    /// File path the thread anchors to.
    pub path: String,
    /// Anchor line on the chosen side. `None` for fully-outdated threads.
    pub line: Option<u32>,
    pub side: RemoteCommentSide,
    pub is_resolved: bool,
    pub is_outdated: bool,
    /// Root comment first, replies in posted order.
    pub comments: Vec<RemoteReviewComment>,
}

impl RemoteReviewThread {
    /// Per the spec, the default `:comments unresolved` view shows only
    /// threads that are neither resolved nor outdated. `:comments all`
    /// shows everything, and `:comments hide` shows nothing.
    pub fn is_active(&self) -> bool {
        !self.is_resolved && !self.is_outdated
    }

    /// The first comment is the thread root for display purposes.
    pub fn root(&self) -> Option<&RemoteReviewComment> {
        self.comments.first()
    }

    /// Iterator over reply comments (everything after the root).
    pub fn replies(&self) -> impl Iterator<Item = &RemoteReviewComment> {
        self.comments.iter().skip(1)
    }
}

/// User-controlled visibility for remote review comments in PR mode.
///
/// Persisted per-session so visibility survives reopen. Default is
/// `Unresolved` — see the spec section "Existing GitHub Comments".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrCommentsVisibility {
    /// Show only unresolved (and not outdated) threads. Default.
    #[default]
    Unresolved,
    /// Show all fetched threads, with muted styling for resolved/outdated.
    All,
    /// Show nothing.
    Hide,
}

impl PrCommentsVisibility {
    /// Decide whether a thread should appear under this visibility setting.
    /// Returns:
    /// - `Some(false)` — render with normal styling
    /// - `Some(true)` — render with muted styling (resolved or outdated)
    /// - `None`       — do not render
    pub fn render_decision(&self, thread: &RemoteReviewThread) -> Option<bool> {
        match self {
            PrCommentsVisibility::Hide => None,
            PrCommentsVisibility::Unresolved => {
                if thread.is_active() {
                    Some(false)
                } else {
                    None
                }
            }
            PrCommentsVisibility::All => Some(!thread.is_active()),
        }
    }

    /// Short label for use in status bar / footer hints.
    pub fn label(&self) -> &'static str {
        match self {
            PrCommentsVisibility::Unresolved => "unresolved",
            PrCommentsVisibility::All => "all",
            PrCommentsVisibility::Hide => "hidden",
        }
    }
}

/// Filter a list of threads by the active visibility setting. Threads that
/// should not render are dropped; remaining ones keep their flags so the
/// renderer can decide on muted styling per-thread.
pub fn filter_threads(
    threads: &[RemoteReviewThread],
    visibility: PrCommentsVisibility,
) -> Vec<&RemoteReviewThread> {
    threads
        .iter()
        .filter(|t| visibility.render_decision(t).is_some())
        .collect()
}

/// Count the number of rendered lines a thread occupies in the diff view.
/// Used by `App::rebuild_annotations` to push the matching number of
/// annotations so cursor/hit-test math stays in sync with rendering.
///
/// Layout (must match `ui::comment_panel::format_remote_thread_lines`):
/// - 1 header line for the root comment (`╭─ [github @author] L42 ──`)
/// - 1 separator line per reply (`├─ ↳ @author ──`)
/// - 1 body line per `\n`-split line in each comment's body
/// - 1 footer line at the end of the thread (`╰────`)
pub fn thread_display_lines(thread: &RemoteReviewThread) -> usize {
    let mut total = 0;
    for comment in &thread.comments {
        // header (root) or separator (reply) + body lines
        total += 1 + comment.body.split('\n').count();
    }
    // single closing rule for the whole thread
    total += 1;
    total
}

/// Group threads by file path for export grouping. Preserves the input
/// order within each file.
pub fn group_threads_by_path(
    threads: &[RemoteReviewThread],
) -> Vec<(&str, Vec<&RemoteReviewThread>)> {
    let mut groups: Vec<(&str, Vec<&RemoteReviewThread>)> = Vec::new();
    for thread in threads {
        if let Some((_, bucket)) = groups.iter_mut().find(|(p, _)| *p == thread.path.as_str()) {
            bucket.push(thread);
        } else {
            groups.push((thread.path.as_str(), vec![thread]));
        }
    }
    groups
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_thread(
        id: &str,
        path: &str,
        line: Option<u32>,
        is_resolved: bool,
        is_outdated: bool,
    ) -> RemoteReviewThread {
        RemoteReviewThread {
            id: id.to_string(),
            path: path.to_string(),
            line,
            side: RemoteCommentSide::Right,
            is_resolved,
            is_outdated,
            comments: vec![RemoteReviewComment {
                id: format!("{id}-root"),
                author: Some("alice".to_string()),
                body: "Root body".to_string(),
                created_at: None,
                in_reply_to: None,
                url: format!("https://example.com/{id}"),
            }],
        }
    }

    #[test]
    fn should_default_visibility_to_unresolved() {
        // given/when
        let v = PrCommentsVisibility::default();
        // then
        assert_eq!(v, PrCommentsVisibility::Unresolved);
    }

    #[test]
    fn should_show_only_active_threads_when_unresolved() {
        // given
        let v = PrCommentsVisibility::Unresolved;
        let active = make_thread("a", "src/lib.rs", Some(10), false, false);
        let resolved = make_thread("b", "src/lib.rs", Some(20), true, false);
        let outdated = make_thread("c", "src/lib.rs", Some(30), false, true);
        // when/then
        assert_eq!(v.render_decision(&active), Some(false));
        assert_eq!(v.render_decision(&resolved), None);
        assert_eq!(v.render_decision(&outdated), None);
    }

    #[test]
    fn should_show_all_threads_with_muted_for_inactive_when_all() {
        // given
        let v = PrCommentsVisibility::All;
        let active = make_thread("a", "src/lib.rs", Some(10), false, false);
        let resolved = make_thread("b", "src/lib.rs", Some(20), true, false);
        let outdated = make_thread("c", "src/lib.rs", Some(30), false, true);
        // when/then
        assert_eq!(v.render_decision(&active), Some(false));
        assert_eq!(v.render_decision(&resolved), Some(true));
        assert_eq!(v.render_decision(&outdated), Some(true));
    }

    #[test]
    fn should_show_no_threads_when_hidden() {
        // given
        let v = PrCommentsVisibility::Hide;
        let active = make_thread("a", "src/lib.rs", Some(10), false, false);
        // when/then
        assert_eq!(v.render_decision(&active), None);
    }

    #[test]
    fn should_filter_threads_preserving_order() {
        // given
        let threads = vec![
            make_thread("a", "src/lib.rs", Some(10), false, false),
            make_thread("b", "src/lib.rs", Some(20), true, false),
            make_thread("c", "src/main.rs", Some(30), false, false),
        ];
        // when
        let unresolved = filter_threads(&threads, PrCommentsVisibility::Unresolved);
        // then
        assert_eq!(unresolved.len(), 2);
        assert_eq!(unresolved[0].id, "a");
        assert_eq!(unresolved[1].id, "c");
    }

    #[test]
    fn should_round_trip_visibility_via_serde() {
        // given
        let cases = [
            PrCommentsVisibility::Unresolved,
            PrCommentsVisibility::All,
            PrCommentsVisibility::Hide,
        ];
        // when/then
        for c in cases {
            let json = serde_json::to_string(&c).unwrap();
            let back: PrCommentsVisibility = serde_json::from_str(&json).unwrap();
            assert_eq!(back, c);
        }
    }

    #[test]
    fn should_group_threads_by_file_preserving_order() {
        // given
        let threads = vec![
            make_thread("a", "src/lib.rs", Some(10), false, false),
            make_thread("b", "src/main.rs", Some(5), false, false),
            make_thread("c", "src/lib.rs", Some(20), false, false),
        ];
        // when
        let groups = group_threads_by_path(&threads);
        // then
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].0, "src/lib.rs");
        assert_eq!(groups[0].1.len(), 2);
        assert_eq!(groups[1].0, "src/main.rs");
        assert_eq!(groups[1].1.len(), 1);
    }

    #[test]
    fn should_parse_remote_comment_side() {
        // given/when/then
        assert_eq!(RemoteCommentSide::parse("LEFT"), RemoteCommentSide::Left);
        assert_eq!(RemoteCommentSide::parse("RIGHT"), RemoteCommentSide::Right);
        assert_eq!(RemoteCommentSide::parse("left"), RemoteCommentSide::Left);
        // unknown defaults to RIGHT (head side) — safer for display
        assert_eq!(RemoteCommentSide::parse(""), RemoteCommentSide::Right);
    }
}
