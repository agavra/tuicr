//! Shared commit-row rendering used by both the fullscreen review-target
//! selector and the inline commit selector shown above the diff. Keeps the
//! row layout (cursor arrow, range bar, checkbox, reviewed marker, hash,
//! branch chip, summary, author/date) consistent across surfaces.

use chrono::{DateTime, Utc};
use ratatui::style::Style;
use ratatui::text::{Line, Span};

use crate::app::{STAGED_SELECTION_ID, UNSTAGED_SELECTION_ID};
use crate::theme::Theme;
use crate::ui::styles;
use crate::ui::text_utils::{truncate_or_pad, truncate_str};
use crate::vcs::CommitInfo;

pub const CURSOR_GLYPH: &str = "\u{25b8}"; // ▸
pub const RANGE_BAR_GLYPH: &str = "\u{258c}"; // ▌
pub const SELECTED_BOX_GLYPH: &str = "\u{25a3}"; // ▣
pub const UNSELECTED_BOX_GLYPH: &str = "\u{25a2}"; // ▢
pub const REVIEWED_GLYPH: &str = "\u{2713}"; // ✓
pub const REVIEWED_LABEL: &str = "✓ ";

// Baseline column widths preserve the compact layout. Narrow rows shrink both
// columns proportionally; wider rows give one third of their surplus to the
// branch and two thirds to the commit summary.
const COMPACT_BRANCH_COL_WIDTH: usize = 16;
const COMPACT_SUMMARY_COL_WIDTH: usize = 50;
const AUTHOR_COL_WIDTH: usize = 12;
const CONTROL_COL_WIDTH: usize = 8;
const HASH_COL_WIDTH: usize = 8;
const RELATIVE_DATE_COL_WIDTH: usize = 8; // "just now"
const METADATA_COL_WIDTH: usize = 2 + AUTHOR_COL_WIDTH + 3 + RELATIVE_DATE_COL_WIDTH;

pub struct CommitRowSpec<'a> {
    pub commit: &'a CommitInfo,
    pub available_width: u16,
    pub is_cursor: bool,
    pub is_selected: bool,
    pub is_reviewed: bool,
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

    let mut spans: Vec<Span<'a>> = Vec::with_capacity(11);
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
    spans.push(Span::styled(
        if spec.is_reviewed {
            REVIEWED_LABEL.to_string()
        } else {
            " ".repeat(REVIEWED_LABEL.chars().count())
        },
        if spec.is_reviewed {
            styles::reviewed_style(theme)
        } else {
            row_text_style
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

    let hash_content_width = (HASH_COL_WIDTH - 1).max(spec.commit.short_id.chars().count());
    let hash = format!(
        "{} ",
        truncate_or_pad_column(&spec.commit.short_id, hash_content_width)
    );
    spans.push(Span::styled(hash, styles::hash_style(theme)));

    let when = format_relative_short(&spec.commit.time);
    let metadata = format!(
        "  {} \u{00b7} {}",
        truncate_or_pad(&spec.commit.author, AUTHOR_COL_WIDTH),
        when
    );
    let hash_col_width = hash_content_width + 1;
    let (branch_col_width, summary_col_width) =
        commit_column_widths(spec.available_width as usize, hash_col_width);

    // Branch column: always branch_col_width cells. With a branch we render
    // `[<name>]` padded out with trailing spaces; without one we render
    // the same number of spaces so the summary column starts at the same x.
    if let Some(branch_name) = &spec.commit.branch_name {
        let chip = format!(
            "[{}]",
            truncate_str(branch_name, branch_col_width.saturating_sub(3))
        );
        spans.push(Span::styled(
            truncate_or_pad_column(&chip, branch_col_width),
            styles::branch_style(theme),
        ));
    } else {
        spans.push(Span::raw(" ".repeat(branch_col_width)));
    }

    spans.push(Span::styled(
        truncate_or_pad_column(&spec.commit.summary, summary_col_width),
        row_text_style,
    ));

    spans.push(Span::styled(
        metadata,
        Style::default().fg(theme.fg_secondary),
    ));

    Line::from(spans)
}

fn commit_column_widths(available_width: usize, hash_col_width: usize) -> (usize, usize) {
    let fixed_width = CONTROL_COL_WIDTH + hash_col_width + METADATA_COL_WIDTH;
    let budget = available_width.saturating_sub(fixed_width);
    let compact_width = COMPACT_BRANCH_COL_WIDTH + COMPACT_SUMMARY_COL_WIDTH;

    if budget <= compact_width {
        let branch_width = budget * COMPACT_BRANCH_COL_WIDTH / compact_width;
        return (branch_width, budget - branch_width);
    }

    let surplus = budget - compact_width;
    let branch_surplus = surplus / 3;
    (
        COMPACT_BRANCH_COL_WIDTH + branch_surplus,
        COMPACT_SUMMARY_COL_WIDTH + surplus - branch_surplus,
    )
}

fn truncate_or_pad_column(value: &str, width: usize) -> String {
    if width < 3 {
        value.chars().take(width).collect()
    } else {
        truncate_or_pad(value, width)
    }
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
    fn should_preserve_compact_column_widths_and_split_surplus_toward_summary() {
        assert_eq!(commit_column_widths(107, HASH_COL_WIDTH), (16, 50));
        assert_eq!(commit_column_widths(137, HASH_COL_WIDTH), (26, 70));
        assert_eq!(commit_column_widths(8, HASH_COL_WIDTH), (0, 0));
    }

    #[test]
    fn should_preserve_backend_provided_short_hashes() {
        // given
        let theme = Theme::dark();
        for short_id in ["abcdef12", "abcdef123456"] {
            let mut c = commit("abcdef1234567890", "Add feature", Some("main"));
            c.short_id = short_id.to_string();

            // when
            let text = line_text(&render_commit_row(&CommitRowSpec {
                commit: &c,
                available_width: 107,
                is_cursor: false,
                is_selected: false,
                is_reviewed: false,
                theme: &theme,
            }));

            // then
            assert!(text.contains(short_id), "missing {short_id:?} in {text:?}");
        }
    }

    #[test]
    fn should_render_cursor_arrow_when_is_cursor() {
        // given
        let theme = Theme::dark();
        let c = commit("abc1234", "Add feature", Some("main"));
        // when
        let line = render_commit_row(&CommitRowSpec {
            commit: &c,
            available_width: 107,
            is_cursor: true,
            is_selected: false,
            is_reviewed: false,
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
            available_width: 107,
            is_cursor: false,
            is_selected: true,
            is_reviewed: false,
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
            available_width: 107,
            is_cursor: false,
            is_selected: false,
            is_reviewed: false,
            theme: &theme,
        });
        // then
        let text = line_text(&line);
        assert!(!text.contains(RANGE_BAR_GLYPH), "got: {text:?}");
        assert!(text.contains(UNSELECTED_BOX_GLYPH), "got: {text:?}");
    }

    #[test]
    fn should_render_reviewed_marker_when_commit_was_already_reviewed() {
        // given
        let theme = Theme::dark();
        let c = commit("abc1234", "Add feature", None);
        // when
        let line = render_commit_row(&CommitRowSpec {
            commit: &c,
            available_width: 107,
            is_cursor: false,
            is_selected: false,
            is_reviewed: true,
            theme: &theme,
        });
        // then
        let text = line_text(&line);
        assert!(text.contains(REVIEWED_LABEL), "got: {text:?}");
    }

    #[test]
    fn should_render_pseudo_commit_with_tag_and_drop_metadata() {
        // given
        let theme = Theme::dark();
        let c = commit(STAGED_SELECTION_ID, "Staged changes", None);
        // when
        let line = render_commit_row(&CommitRowSpec {
            commit: &c,
            available_width: 107,
            is_cursor: false,
            is_selected: false,
            is_reviewed: false,
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
            available_width: 107,
            is_cursor: false,
            is_selected: false,
            is_reviewed: false,
            theme: &theme,
        });
        // then
        let text = line_text(&line);
        assert!(text.contains("[feat/foo]"), "got: {text:?}");
    }

    #[test]
    fn should_preserve_all_columns_at_the_compact_width() {
        // given
        let theme = Theme::dark();
        let c = commit("abc1234", "Add feature", Some("feat/foo"));
        // when
        let text = line_text(&render_commit_row(&CommitRowSpec {
            commit: &c,
            available_width: 107,
            is_cursor: true,
            is_selected: true,
            is_reviewed: true,
            theme: &theme,
        }));
        // then
        for expected in [
            CURSOR_GLYPH,
            RANGE_BAR_GLYPH,
            SELECTED_BOX_GLYPH,
            REVIEWED_GLYPH,
            "abc1234",
            "[feat/foo]",
            "Add feature",
            "alice",
            "·",
        ] {
            assert!(text.contains(expected), "missing {expected:?} in {text:?}");
        }
    }

    #[test]
    fn should_expand_branch_and_summary_columns_at_wide_widths() {
        // given
        let theme = Theme::dark();
        let alpha = commit(
            "abc1234",
            "Keep the commit summary visible alongside its metadata",
            Some("feat/responsive-branch-alpha"),
        );
        let beta = commit(
            "def5678",
            "Keep the commit summary visible alongside its metadata",
            Some("feat/responsive-branch-beta"),
        );
        // when
        let alpha = line_text(&render_commit_row(&CommitRowSpec {
            commit: &alpha,
            available_width: 160,
            is_cursor: false,
            is_selected: false,
            is_reviewed: false,
            theme: &theme,
        }));
        let beta = line_text(&render_commit_row(&CommitRowSpec {
            commit: &beta,
            available_width: 160,
            is_cursor: false,
            is_selected: false,
            is_reviewed: false,
            theme: &theme,
        }));
        // then
        assert!(alpha.contains("branch-alpha]"), "got: {alpha:?}");
        assert!(beta.contains("branch-beta]"), "got: {beta:?}");
        assert!(alpha.contains("Keep the commit summary"), "got: {alpha:?}");
        assert!(alpha.contains("alice"), "got: {alpha:?}");
        assert!(alpha.contains('·'), "got: {alpha:?}");
    }

    #[test]
    fn should_align_summaries_for_rows_with_and_without_branches() {
        // given
        let theme = Theme::dark();
        let with_branch = commit("abc1234", "same summary", Some("feature/foo"));
        let without_branch = commit("def5678", "same summary", None);
        // when
        let with_branch = line_text(&render_commit_row(&CommitRowSpec {
            commit: &with_branch,
            available_width: 120,
            is_cursor: false,
            is_selected: false,
            is_reviewed: false,
            theme: &theme,
        }));
        let without_branch = line_text(&render_commit_row(&CommitRowSpec {
            commit: &without_branch,
            available_width: 120,
            is_cursor: false,
            is_selected: false,
            is_reviewed: false,
            theme: &theme,
        }));
        // then
        assert_eq!(
            with_branch.find("same summary"),
            without_branch.find("same summary")
        );
    }

    #[test]
    fn should_render_very_narrow_rows_without_panicking() {
        // given
        let theme = Theme::dark();
        let c = commit("abc1234", "summary", Some("feature/foo"));
        // when
        let line = render_commit_row(&CommitRowSpec {
            commit: &c,
            available_width: 8,
            is_cursor: false,
            is_selected: false,
            is_reviewed: false,
            theme: &theme,
        });
        // then
        assert!(!line_text(&line).is_empty());
    }

    #[test]
    fn should_keep_pseudo_commit_content_stable_across_widths() {
        // given
        let theme = Theme::dark();
        for (id, summary, tag) in [
            (STAGED_SELECTION_ID, "Staged changes", "staged"),
            (UNSTAGED_SELECTION_ID, "Unstaged changes", "unstaged"),
        ] {
            let c = commit(id, summary, None);
            // when
            let narrow = line_text(&render_commit_row(&CommitRowSpec {
                commit: &c,
                available_width: 20,
                is_cursor: false,
                is_selected: false,
                is_reviewed: false,
                theme: &theme,
            }));
            let wide = line_text(&render_commit_row(&CommitRowSpec {
                commit: &c,
                available_width: 160,
                is_cursor: false,
                is_selected: false,
                is_reviewed: false,
                theme: &theme,
            }));
            // then
            assert_eq!(narrow, wide);
            assert!(wide.contains(tag), "got: {wide:?}");
            assert!(wide.contains(summary), "got: {wide:?}");
        }
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
