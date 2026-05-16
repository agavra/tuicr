//! Shared commit-row rendering used by both the fullscreen review-target
//! selector and the inline commit selector shown above the diff. Keeps the
//! row layout (cursor arrow, range bar, checkbox, hash, branch chip, summary,
//! author/date) consistent across surfaces.

use chrono::{DateTime, Utc};
use ratatui::style::Style;
use ratatui::text::{Line, Span};

use crate::app::{STAGED_SELECTION_ID, UNSTAGED_SELECTION_ID};
use crate::theme::Theme;
use crate::ui::styles;
use crate::ui::text_utils::truncate_str;
use crate::vcs::CommitInfo;

pub const CURSOR_GLYPH: &str = "\u{25b8}"; // ▸
pub const RANGE_BAR_GLYPH: &str = "\u{258c}"; // ▌
pub const SELECTED_BOX_GLYPH: &str = "\u{25a3}"; // ▣
pub const UNSELECTED_BOX_GLYPH: &str = "\u{25a2}"; // ▢

pub struct CommitRowSpec<'a> {
    pub commit: &'a CommitInfo,
    pub is_cursor: bool,
    pub is_selected: bool,
    pub theme: &'a Theme,
}

pub fn render_commit_row<'a>(spec: &CommitRowSpec<'a>) -> Line<'a> {
    let theme = spec.theme;

    let row_text_style = if spec.is_cursor {
        styles::selected_style(theme)
    } else if spec.is_selected {
        Style::default().fg(theme.fg_secondary)
    } else {
        Style::default().fg(theme.fg_primary)
    };

    let mut spans: Vec<Span<'a>> = Vec::with_capacity(10);
    spans.push(Span::styled(
        if spec.is_cursor {
            format!("{CURSOR_GLYPH} ")
        } else {
            "  ".to_string()
        },
        row_text_style,
    ));
    spans.push(Span::styled(
        if spec.is_selected {
            format!("{RANGE_BAR_GLYPH} ")
        } else {
            "  ".to_string()
        },
        styles::range_bar_style(theme),
    ));
    spans.push(Span::styled(
        if spec.is_selected {
            format!("{SELECTED_BOX_GLYPH} ")
        } else {
            format!("{UNSELECTED_BOX_GLYPH} ")
        },
        if spec.is_selected {
            styles::reviewed_style(theme)
        } else {
            styles::pending_style(theme)
        },
    ));

    if spec.commit.id == STAGED_SELECTION_ID || spec.commit.id == UNSTAGED_SELECTION_ID {
        let tag = if spec.commit.id == STAGED_SELECTION_ID {
            " \u{00b7} staged \u{00b7}   "
        } else {
            " \u{00b7} unstaged \u{00b7} "
        };
        spans.push(Span::styled(tag, styles::pseudo_commit_tag_style(theme)));
        spans.push(Span::styled(spec.commit.summary.clone(), row_text_style));
        return Line::from(spans);
    }

    spans.push(Span::styled(
        format!("{} ", spec.commit.short_id),
        styles::hash_style(theme),
    ));

    if let Some(branch_name) = &spec.commit.branch_name {
        spans.push(Span::styled(
            format!("[{}] ", truncate_str(branch_name, 20)),
            styles::branch_style(theme),
        ));
    }

    spans.push(Span::styled(
        truncate_str(&spec.commit.summary, 50),
        row_text_style,
    ));

    let when = format_relative_short(&spec.commit.time);
    spans.push(Span::styled(
        format!("  {} \u{00b7} {}", spec.commit.author, when),
        Style::default().fg(theme.fg_secondary),
    ));

    Line::from(spans)
}

/// Compact relative time used in selector rows: `5m`, `3h`, `2d`, `6w`, `4mo`,
/// `2y`, or `just now`. Mirrors `format_relative_time` in `selector.rs` but
/// without the trailing "ago" so rows stay tight.
pub fn format_relative_short(time: &DateTime<Utc>) -> String {
    let now = Utc::now();
    let delta = now.signed_duration_since(*time);
    if delta.num_seconds() < 60 {
        return "just now".to_string();
    }
    let mins = delta.num_minutes();
    if mins < 60 {
        return format!("{mins}m");
    }
    let hours = delta.num_hours();
    if hours < 24 {
        return format!("{hours}h");
    }
    let days = delta.num_days();
    if days < 7 {
        return format!("{days}d");
    }
    if days < 30 {
        return format!("{}w", days / 7);
    }
    if days < 365 {
        return format!("{}mo", days / 30);
    }
    format!("{}y", days / 365)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn commit(id: &str, summary: &str, branch: Option<&str>) -> CommitInfo {
        CommitInfo {
            id: id.to_string(),
            short_id: id[..7.min(id.len())].to_string(),
            branch_name: branch.map(|s| s.to_string()),
            summary: summary.to_string(),
            body: None,
            author: "alice".to_string(),
            time: Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
        }
    }

    fn line_text(line: &Line<'_>) -> String {
        line.spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect::<String>()
    }

    #[test]
    fn should_render_cursor_arrow_when_is_cursor() {
        // given
        let theme = Theme::dark();
        let c = commit("abc1234", "Add feature", Some("main"));
        // when
        let line = render_commit_row(&CommitRowSpec {
            commit: &c,
            is_cursor: true,
            is_selected: false,
            theme: &theme,
        });
        // then
        let text = line_text(&line);
        assert!(text.starts_with(CURSOR_GLYPH), "got: {text:?}");
    }

    #[test]
    fn should_render_range_bar_when_selected() {
        // given
        let theme = Theme::dark();
        let c = commit("abc1234", "Add feature", None);
        // when
        let line = render_commit_row(&CommitRowSpec {
            commit: &c,
            is_cursor: false,
            is_selected: true,
            theme: &theme,
        });
        // then
        let text = line_text(&line);
        assert!(text.contains(RANGE_BAR_GLYPH), "got: {text:?}");
        assert!(text.contains(SELECTED_BOX_GLYPH), "got: {text:?}");
    }

    #[test]
    fn should_render_empty_box_when_not_selected() {
        // given
        let theme = Theme::dark();
        let c = commit("abc1234", "Add feature", None);
        // when
        let line = render_commit_row(&CommitRowSpec {
            commit: &c,
            is_cursor: false,
            is_selected: false,
            theme: &theme,
        });
        // then
        let text = line_text(&line);
        assert!(!text.contains(RANGE_BAR_GLYPH), "got: {text:?}");
        assert!(text.contains(UNSELECTED_BOX_GLYPH), "got: {text:?}");
    }

    #[test]
    fn should_render_pseudo_commit_with_tag_and_drop_metadata() {
        // given
        let theme = Theme::dark();
        let c = commit(STAGED_SELECTION_ID, "Staged changes", None);
        // when
        let line = render_commit_row(&CommitRowSpec {
            commit: &c,
            is_cursor: false,
            is_selected: false,
            theme: &theme,
        });
        // then
        let text = line_text(&line);
        assert!(text.contains("staged"), "got: {text:?}");
        assert!(text.contains("Staged changes"), "got: {text:?}");
        assert!(!text.contains("alice"), "should drop author: {text:?}");
    }

    #[test]
    fn should_render_branch_chip_when_present() {
        // given
        let theme = Theme::dark();
        let c = commit("abc1234", "Add feature", Some("feat/foo"));
        // when
        let line = render_commit_row(&CommitRowSpec {
            commit: &c,
            is_cursor: false,
            is_selected: false,
            theme: &theme,
        });
        // then
        let text = line_text(&line);
        assert!(text.contains("[feat/foo]"), "got: {text:?}");
    }

    #[test]
    fn should_format_short_relative_time_buckets() {
        // given
        let now = Utc::now();
        // when / then
        assert_eq!(format_relative_short(&now), "just now");
        assert_eq!(
            format_relative_short(&(now - chrono::Duration::minutes(5))),
            "5m"
        );
        assert_eq!(
            format_relative_short(&(now - chrono::Duration::hours(3))),
            "3h"
        );
        assert_eq!(
            format_relative_short(&(now - chrono::Duration::days(2))),
            "2d"
        );
        assert_eq!(
            format_relative_short(&(now - chrono::Duration::days(20))),
            "2w"
        );
        assert_eq!(
            format_relative_short(&(now - chrono::Duration::days(60))),
            "2mo"
        );
        assert_eq!(
            format_relative_short(&(now - chrono::Duration::days(800))),
            "2y"
        );
    }
}
