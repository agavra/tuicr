use ratatui::style::{Modifier, Style};

use crate::theme::Theme;

pub fn header_style(theme: &Theme) -> Style {
    Style::default()
        .fg(theme.fg_primary)
        .add_modifier(Modifier::BOLD)
}

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
        .fg(theme.diff_hunk_header)
        .add_modifier(Modifier::BOLD)
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

pub fn dir_icon_style(theme: &Theme) -> Style {
    Style::default().fg(theme.diff_hunk_header)
}

pub fn comment_type_style(theme: &Theme, comment_type: crate::model::CommentType) -> Style {
    use crate::model::CommentType;
    let color = match comment_type {
        CommentType::Note => theme.comment_note,
        CommentType::Suggestion => theme.comment_suggestion,
        CommentType::Issue => theme.comment_issue,
        CommentType::Praise => theme.comment_praise,
    };
    Style::default().fg(color).add_modifier(Modifier::BOLD)
}

pub fn comment_border_style(theme: &Theme, comment_type: crate::model::CommentType) -> Style {
    use crate::model::CommentType;
    let color = match comment_type {
        CommentType::Note => theme.comment_note,
        CommentType::Suggestion => theme.comment_suggestion,
        CommentType::Issue => theme.comment_issue,
        CommentType::Praise => theme.comment_praise,
    };
    Style::default().fg(color)
}
