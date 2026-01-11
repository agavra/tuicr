use ratatui::style::{Color, Modifier, Style};

// Base colors
pub const BG_HIGHLIGHT: Color = Color::Rgb(50, 50, 50);

pub const FG_PRIMARY: Color = Color::White;
pub const FG_SECONDARY: Color = Color::Gray;
pub const FG_DIM: Color = Color::DarkGray;

// Diff colors
pub const DIFF_ADD: Color = Color::Green;
pub const DIFF_ADD_BG: Color = Color::Rgb(0, 40, 0);
pub const DIFF_DEL: Color = Color::Red;
pub const DIFF_DEL_BG: Color = Color::Rgb(40, 0, 0);
pub const DIFF_CONTEXT: Color = Color::Gray;
pub const DIFF_HUNK_HEADER: Color = Color::Cyan;

// File status colors
pub const FILE_ADDED: Color = Color::Green;
pub const FILE_MODIFIED: Color = Color::Yellow;
pub const FILE_DELETED: Color = Color::Red;
pub const FILE_RENAMED: Color = Color::Magenta;

// Review status colors
pub const REVIEWED: Color = Color::Green;
pub const PENDING: Color = Color::Yellow;

// Comment type colors
pub const COMMENT_NOTE: Color = Color::Blue;
pub const COMMENT_SUGGESTION: Color = Color::Cyan;
pub const COMMENT_ISSUE: Color = Color::Red;
pub const COMMENT_PRAISE: Color = Color::Green;

// UI element colors
pub const BORDER_FOCUSED: Color = Color::Cyan;
pub const BORDER_UNFOCUSED: Color = Color::DarkGray;
pub const STATUS_BAR_BG: Color = Color::Rgb(40, 40, 40);
pub const CURSOR_COLOR: Color = Color::Yellow;

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
        .bg(Color::Cyan)
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
    Style::default().fg(Color::Yellow)
}

pub fn dir_icon_style() -> Style {
    Style::default().fg(DIFF_HUNK_HEADER)
}
