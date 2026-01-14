use ratatui::style::{Color, Modifier, Style};

// Base colors
pub const BG_HIGHLIGHT: Color = Color::Rgb(70, 70, 70);

pub const FG_PRIMARY: Color = Color::White;
pub const FG_SECONDARY: Color = Color::Rgb(210, 210, 210);
pub const FG_DIM: Color = Color::Rgb(160, 160, 160);

// Diff colors
pub const DIFF_ADD: Color = Color::Rgb(80, 220, 120);
pub const DIFF_ADD_BG: Color = Color::Rgb(0, 60, 20);
pub const DIFF_DEL: Color = Color::Rgb(240, 90, 90);
pub const DIFF_DEL_BG: Color = Color::Rgb(70, 0, 0);
pub const DIFF_CONTEXT: Color = Color::Rgb(200, 200, 200);
pub const DIFF_HUNK_HEADER: Color = Color::Rgb(90, 200, 255);
pub const EXPANDED_CONTEXT_FG: Color = Color::Rgb(140, 140, 140);

// File status colors
pub const FILE_ADDED: Color = Color::Rgb(80, 220, 120);
pub const FILE_MODIFIED: Color = Color::Rgb(255, 210, 90);
pub const FILE_DELETED: Color = Color::Rgb(240, 90, 90);
pub const FILE_RENAMED: Color = Color::Rgb(255, 140, 220);

// Review status colors
pub const REVIEWED: Color = Color::Rgb(80, 220, 120);
pub const PENDING: Color = Color::Rgb(255, 210, 90);

// Comment type colors
pub const COMMENT_NOTE: Color = Color::Rgb(90, 170, 255);
pub const COMMENT_SUGGESTION: Color = Color::Rgb(90, 220, 240);
pub const COMMENT_ISSUE: Color = Color::Rgb(240, 90, 90);
pub const COMMENT_PRAISE: Color = Color::Rgb(80, 220, 120);

// UI element colors
pub const BORDER_FOCUSED: Color = Color::Rgb(90, 200, 255);
pub const BORDER_UNFOCUSED: Color = Color::Rgb(110, 110, 110);
pub const STATUS_BAR_BG: Color = Color::Rgb(30, 30, 30);
pub const CURSOR_COLOR: Color = Color::Rgb(255, 210, 90);

// Styles
pub fn header_style() -> Style {
    Style::default().fg(FG_PRIMARY).add_modifier(Modifier::BOLD)
}

pub fn selected_style() -> Style {
    Style::default().bg(BG_HIGHLIGHT).fg(FG_PRIMARY)
}

pub fn dim_style() -> Style {
    Style::default().fg(FG_DIM)
}

pub fn diff_add_style() -> Style {
    Style::default().fg(DIFF_ADD).bg(DIFF_ADD_BG)
}

pub fn diff_del_style() -> Style {
    Style::default().fg(DIFF_DEL).bg(DIFF_DEL_BG)
}

pub fn diff_context_style() -> Style {
    Style::default().fg(DIFF_CONTEXT)
}

pub fn expanded_context_style() -> Style {
    Style::default().fg(EXPANDED_CONTEXT_FG)
}

pub fn diff_hunk_header_style() -> Style {
    Style::default()
        .fg(DIFF_HUNK_HEADER)
        .add_modifier(Modifier::BOLD)
}

pub fn file_header_style() -> Style {
    Style::default().fg(FG_PRIMARY).add_modifier(Modifier::BOLD)
}

pub fn reviewed_style() -> Style {
    Style::default().fg(REVIEWED)
}

pub fn pending_style() -> Style {
    Style::default().fg(PENDING)
}

pub fn border_style(focused: bool) -> Style {
    if focused {
        Style::default().fg(BORDER_FOCUSED)
    } else {
        Style::default().fg(BORDER_UNFOCUSED)
    }
}

pub fn status_bar_style() -> Style {
    Style::default().bg(STATUS_BAR_BG).fg(FG_PRIMARY)
}

pub fn mode_style() -> Style {
    Style::default()
        .fg(Color::Black)
        .bg(Color::Rgb(90, 200, 255))
        .add_modifier(Modifier::BOLD)
}

pub fn file_status_style(status: char) -> Style {
    let color = match status {
        'A' => FILE_ADDED,
        'M' => FILE_MODIFIED,
        'D' => FILE_DELETED,
        'R' => FILE_RENAMED,
        _ => FG_SECONDARY,
    };
    Style::default().fg(color)
}

pub fn current_line_indicator_style() -> Style {
    Style::default().fg(BORDER_FOCUSED)
}

pub fn hash_style() -> Style {
    Style::default().fg(Color::Rgb(255, 210, 90))
}

pub fn dir_icon_style() -> Style {
    Style::default().fg(DIFF_HUNK_HEADER)
}
