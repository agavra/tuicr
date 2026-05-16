use ratatui::style::{Color, Modifier, Style};

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

pub fn current_line_indicator_style(theme: &Theme) -> Style {
    Style::default().fg(theme.border_focused)
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

pub fn visual_selection_style(theme: &Theme) -> Style {
    Style::default().bg(theme.bg_highlight)
}

pub fn help_indicator_style(theme: &Theme) -> Style {
    Style::default().fg(theme.help_indicator).bg(theme.panel_bg)
}

pub fn range_bar_style(theme: &Theme) -> Style {
    Style::default().fg(theme.border_focused)
}

pub fn error_inline_style(theme: &Theme) -> Style {
    Style::default()
        .fg(theme.message_error_fg)
        .add_modifier(Modifier::BOLD)
}

pub fn pseudo_commit_tag_style(theme: &Theme) -> Style {
    Style::default().fg(theme.file_modified)
}
