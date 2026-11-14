use ratatui::style::{Color, Modifier, Style};

use crate::model::LineOrigin;
use crate::theme::Theme;

pub fn selected_style(theme: &Theme) -> Style {
    Style::default().bg(theme.bg_highlight).fg(theme.fg_primary)
}

pub fn dim_style(theme: &Theme) -> Style {
    Style::default().fg(theme.fg_dim)
}

pub fn diff_add_style(theme: &Theme) -> Style {
    Style::default().fg(theme.diff_add).bg(theme.diff_add_bg)
}

pub fn diff_del_style(theme: &Theme) -> Style {
    Style::default().fg(theme.diff_del).bg(theme.diff_del_bg)
}

pub fn diff_context_style(theme: &Theme) -> Style {
    Style::default().fg(theme.diff_context)
}

/// Live background a syntax-highlighted diff span should show for a line of
/// the given origin. `DiffLine.highlighted_spans` bakes in whatever
/// `syntax_add_bg`/`syntax_del_bg` was active when the diff was loaded (or
/// last rehighlighted); this lets renderers override that per-frame from the
/// currently active theme instead of trusting the cached span's own
/// background, so add/del backgrounds never lag behind the active theme even
/// if a rehighlight was skipped or missed. `None` for context lines, which
/// never carry a baked-in background (`syntax::apply_diff_background`
/// intentionally excludes them).
pub fn diff_syntax_bg(theme: &Theme, origin: LineOrigin) -> Option<Color> {
    match origin {
        LineOrigin::Addition => Some(theme.syntax_add_bg),
        LineOrigin::Deletion => Some(theme.syntax_del_bg),
        LineOrigin::Context => None,
    }
}

/// Style of the change bar (`▌`) and the gap after it. A syntax-colored line
/// sits on `syntax_add_bg`/`syntax_del_bg`, so its bar does too and the row
/// keeps one background; an uncolored line keeps `diff_add_bg`/`diff_del_bg`.
pub fn diff_marker_style(theme: &Theme, origin: LineOrigin, highlighted: bool) -> Style {
    let style = match origin {
        LineOrigin::Addition => diff_add_style(theme),
        LineOrigin::Deletion => diff_del_style(theme),
        LineOrigin::Context => return diff_context_style(theme),
    };
    if highlighted {
        patch_highlighted_span_bg(style, theme, origin)
    } else {
        style
    }
}

/// Apply `diff_syntax_bg` to `style`, leaving it untouched when there is no
/// live override for `origin` (context lines).
pub fn patch_highlighted_span_bg(style: Style, theme: &Theme, origin: LineOrigin) -> Style {
    match diff_syntax_bg(theme, origin) {
        Some(bg) => style.bg(bg),
        None => style,
    }
}

/// Style patched over the changed words of a paired deletion/addition line:
/// the line's background pushed toward its diff foreground, so it stays in
/// the same hue but stands out. `highlighted` selects the syntax background
/// the line is drawn on. Themes using named terminal colors can't be blended
/// and get reversed video instead.
pub fn word_emphasis_style(theme: &Theme, origin: LineOrigin, highlighted: bool) -> Style {
    let (line_bg, accent) = match (origin, highlighted) {
        (LineOrigin::Addition, true) => (theme.syntax_add_bg, theme.diff_add),
        (LineOrigin::Addition, false) => (theme.diff_add_bg, theme.diff_add),
        (LineOrigin::Deletion, true) => (theme.syntax_del_bg, theme.diff_del),
        (LineOrigin::Deletion, false) => (theme.diff_del_bg, theme.diff_del),
        (LineOrigin::Context, _) => return Style::default(),
    };
    match (line_bg, accent) {
        (Color::Rgb(..), Color::Rgb(..)) => {
            Style::default().bg(crate::theme::blend(line_bg, accent, WORD_EMPHASIS_PERCENT))
        }
        _ => Style::default().add_modifier(Modifier::REVERSED),
    }
}

/// How far `word_emphasis_style` moves the background toward the foreground.
const WORD_EMPHASIS_PERCENT: u8 = 30;

pub fn expanded_context_style(theme: &Theme) -> Style {
    Style::default().fg(theme.expanded_context_fg)
}

pub fn diff_hunk_header_style(theme: &Theme) -> Style {
    Style::default()
        .fg(theme.fg_dim)
        .bg(theme.section_highlight_bg())
}

pub fn file_header_style(theme: &Theme) -> Style {
    Style::default()
        .fg(theme.fg_primary)
        .add_modifier(Modifier::BOLD)
}

pub fn reviewed_style(theme: &Theme) -> Style {
    Style::default().fg(theme.reviewed)
}

pub fn pending_style(theme: &Theme) -> Style {
    Style::default().fg(theme.pending)
}

pub fn border_style(theme: &Theme, focused: bool) -> Style {
    if focused {
        Style::default().fg(theme.border_focused)
    } else {
        Style::default().fg(theme.border_unfocused)
    }
}

pub fn panel_style(theme: &Theme) -> Style {
    Style::default().bg(theme.panel_bg).fg(theme.fg_primary)
}

pub fn popup_style(theme: &Theme) -> Style {
    panel_style(theme)
}

pub fn status_bar_style(theme: &Theme) -> Style {
    Style::default()
        .bg(theme.status_bar_bg)
        .fg(theme.fg_primary)
}

pub fn mode_style(theme: &Theme) -> Style {
    Style::default()
        .fg(theme.mode_fg)
        .bg(theme.mode_bg)
        .add_modifier(Modifier::BOLD)
}

pub fn file_status_style(theme: &Theme, status: char) -> Style {
    let color = match status {
        'A' => theme.file_added,
        'M' => theme.file_modified,
        'D' => theme.file_deleted,
        'R' => theme.file_renamed,
        _ => theme.fg_secondary,
    };
    Style::default().fg(color)
}

/// Override only the marker foreground, preserving the caller's legacy style.
pub fn selection_marker_style(theme: &Theme, fallback: Style) -> Style {
    match theme.selection_marker {
        Some(color) => fallback.fg(color),
        None => fallback,
    }
}

pub fn current_line_indicator_style(theme: &Theme) -> Style {
    selection_marker_style(theme, Style::default().fg(theme.border_focused))
}

pub fn hash_style(theme: &Theme) -> Style {
    Style::default().fg(theme.cursor_color)
}

pub fn branch_style(theme: &Theme) -> Style {
    Style::default().fg(theme.branch_name)
}

pub fn dir_icon_style(theme: &Theme) -> Style {
    Style::default().fg(theme.diff_hunk_header)
}

pub fn comment_type_style(_theme: &Theme, color: Color) -> Style {
    Style::default().fg(color).add_modifier(Modifier::BOLD)
}

pub fn comment_border_style(theme: &Theme, _color: Color) -> Style {
    // Match the file-header separator look so the comment box reads as a
    // structural divider rather than as colour-coded chrome. The comment-
    // type colour still lives on the [NOTE]/[ISSUE]/... label inside.
    file_header_style(theme)
}

/// Fixed palette used to tint comment chrome by author. Excludes red/green so
/// the colour never collides with diff add/del semantics. Cyan/yellow/magenta/
/// blue/light-magenta/light-cyan give us six visually distinct slots — enough
/// for a handful of agents alongside the human reviewer.
const AUTHOR_PALETTE: &[Color] = &[
    Color::Cyan,
    Color::Yellow,
    Color::Magenta,
    Color::Blue,
    Color::LightMagenta,
    Color::LightCyan,
];

/// Deterministic palette colour for a given author name. Always returns
/// a colour; callers gate visibility separately via [`author_accent`].
/// Hashes via FNV-1a so the mapping is platform-independent and survives
/// across runs.
pub fn author_color_for(author: &str) -> Color {
    let mut hash: u64 = 0xcbf29ce484222325;
    for b in author.as_bytes() {
        hash ^= *b as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    AUTHOR_PALETTE[(hash as usize) % AUTHOR_PALETTE.len()]
}

/// `Some(colour)` when `author` differs from the viewer (so the comment
/// chrome should advertise authorship), `None` when the comment is the
/// viewer's own.
pub fn author_accent(viewer: &str, author: &str) -> Option<Color> {
    if author == viewer {
        None
    } else {
        Some(author_color_for(author))
    }
}

pub fn visual_selection_style(theme: &Theme) -> Style {
    Style::default().bg(theme.bg_highlight)
}

pub fn search_match_style(theme: &Theme) -> Style {
    Style::default().bg(theme.search_match_bg)
}

pub fn help_indicator_style(theme: &Theme) -> Style {
    Style::default().fg(theme.help_indicator).bg(theme.panel_bg)
}

pub fn range_bar_style(theme: &Theme) -> Style {
    current_line_indicator_style(theme)
}

pub fn error_inline_style(theme: &Theme) -> Style {
    Style::default()
        .fg(theme.message_error_fg)
        .add_modifier(Modifier::BOLD)
}

pub fn pseudo_commit_tag_style(theme: &Theme) -> Style {
    Style::default().fg(theme.file_modified)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A syntax-colored line's change bar takes the syntax background; an
    /// uncolored line's keeps the plain diff background.
    #[test]
    fn diff_marker_style_follows_the_line_background() {
        let theme = Theme::catppuccin_macchiato();

        let colored = diff_marker_style(&theme, LineOrigin::Addition, true);
        assert_eq!(colored.bg, Some(theme.syntax_add_bg));
        assert_eq!(colored.fg, Some(theme.diff_add));
        assert_eq!(
            diff_marker_style(&theme, LineOrigin::Deletion, true).bg,
            Some(theme.syntax_del_bg)
        );
        assert_eq!(
            diff_marker_style(&theme, LineOrigin::Addition, false),
            diff_add_style(&theme)
        );
        assert_eq!(
            diff_marker_style(&theme, LineOrigin::Context, true),
            diff_context_style(&theme)
        );
    }
    /// Changed words sit on the line's own background pushed toward its diff
    /// color, so syntax colors stay on the text.
    #[test]
    fn word_emphasis_blends_the_line_background_toward_the_diff_color() {
        let theme = Theme::catppuccin_macchiato();

        assert_eq!(
            word_emphasis_style(&theme, LineOrigin::Addition, true).bg,
            Some(crate::theme::blend(theme.syntax_add_bg, theme.diff_add, 30))
        );
        assert_eq!(
            word_emphasis_style(&theme, LineOrigin::Deletion, false).bg,
            Some(crate::theme::blend(theme.diff_del_bg, theme.diff_del, 30))
        );
        assert_eq!(
            word_emphasis_style(&theme, LineOrigin::Addition, true).fg,
            None
        );
        assert_eq!(
            word_emphasis_style(&theme, LineOrigin::Context, true),
            Style::default()
        );
        let mut named = Theme::dark();
        named.diff_add = Color::Green;
        assert!(
            word_emphasis_style(&named, LineOrigin::Addition, false)
                .add_modifier
                .contains(Modifier::REVERSED)
        );
    }

    #[test]
    fn selection_marker_changes_only_marker_foregrounds() {
        let mut theme = Theme::github_dark();
        let legacy_pointer = selected_style(&theme);
        assert_eq!(
            selection_marker_style(&theme, legacy_pointer),
            legacy_pointer
        );
        assert_eq!(
            current_line_indicator_style(&theme).fg,
            Some(theme.border_focused)
        );
        assert_eq!(range_bar_style(&theme).fg, Some(theme.border_focused));
        let unchanged = (
            border_style(&theme, true),
            hash_style(&theme),
            selected_style(&theme),
            visual_selection_style(&theme),
        );

        theme.selection_marker = Some(Color::Rgb(36, 45, 56));
        assert_eq!(
            current_line_indicator_style(&theme).fg,
            theme.selection_marker
        );
        assert_eq!(range_bar_style(&theme).fg, theme.selection_marker);
        assert_eq!(
            selection_marker_style(&theme, legacy_pointer).bg,
            legacy_pointer.bg
        );
        assert_eq!(
            unchanged,
            (
                border_style(&theme, true),
                hash_style(&theme),
                selected_style(&theme),
                visual_selection_style(&theme)
            )
        );
    }
}
