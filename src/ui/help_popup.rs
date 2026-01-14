use ratatui::{
    Frame,
    layout::{Constraint, Flex, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
};

use crate::app::App;
use crate::ui::styles;

pub fn render_help(frame: &mut Frame, app: &mut App) {
    let area = centered_rect(60, 70, frame.area());

    // Clear the area behind the popup
    frame.render_widget(Clear, area);

    let block = Block::default()
        .title(" Help (j/k to scroll) - Press ? or Esc to close ")
        .borders(Borders::ALL)
        .border_style(styles::border_style(true));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let help_text = vec![
        Line::from(Span::styled(
            "Navigation",
            Style::default().add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled(
                "  j/k       ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("Scroll down/up"),
        ]),
        Line::from(vec![
            Span::styled(
                "  Ctrl-d/u  ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("Half page down/up"),
        ]),
        Line::from(vec![
            Span::styled(
                "  Ctrl-f/b  ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("Full page down/up"),
        ]),
        Line::from(vec![
            Span::styled(
                "  g/G       ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("Go to first/last file"),
        ]),
        Line::from(vec![
            Span::styled(
                "  {/}       ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("Jump to prev/next file"),
        ]),
        Line::from(vec![
            Span::styled(
                "  [/]       ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("Jump to prev/next hunk"),
        ]),
        Line::from(vec![
            Span::styled(
                "  /         ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("Search within diff"),
        ]),
        Line::from(vec![
            Span::styled(
                "  n/N       ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("Next/prev search match"),
        ]),
        Line::from(vec![
            Span::styled(
                "  Enter     ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("Expand/collapse hidden context"),
        ]),
        Line::from(vec![
            Span::styled(
                "  Tab       ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("Toggle focus file list/diff"),
        ]),
        Line::from(vec![
            Span::styled(
                "  ;h/;l     ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("Focus file list/diff"),
        ]),
        Line::from(vec![
            Span::styled(
                "  ;e        ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("Toggle file list visibility"),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "File Tree",
            Style::default().add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled(
                "  Space     ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("Toggle expand directory"),
        ]),
        Line::from(vec![
            Span::styled(
                "  Enter     ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("Expand dir / Jump to file"),
        ]),
        Line::from(vec![
            Span::styled(
                "  o         ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("Expand all directories"),
        ]),
        Line::from(vec![
            Span::styled(
                "  O         ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("Collapse all directories"),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "Review Actions",
            Style::default().add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled(
                "  r         ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("Toggle file reviewed"),
        ]),
        Line::from(vec![
            Span::styled(
                "  c         ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("Add line comment"),
        ]),
        Line::from(vec![
            Span::styled(
                "  C         ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("Add file comment"),
        ]),
        Line::from(vec![
            Span::styled(
                "  i         ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("Edit comment at cursor"),
        ]),
        Line::from(vec![
            Span::styled(
                "  dd        ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("Delete comment at cursor"),
        ]),
        Line::from(vec![
            Span::styled(
                "  y         ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("Yank (copy) review to clipboard"),
        ]),
        Line::from(vec![
            Span::styled(
                "  v         ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("Toggle unified/side-by-side diff view"),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "Comment Mode",
            Style::default().add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled(
                "  Tab       ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("Toggle type: Note/Suggestion/Issue/Praise"),
        ]),
        Line::from(vec![
            Span::styled(
                "  Ctrl-S    ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("Save comment"),
        ]),
        Line::from(vec![
            Span::styled(
                "  Ctrl-A/E  ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("Line start/end"),
        ]),
        Line::from(vec![
            Span::styled(
                "  Ctrl/Alt-Left/Right",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("Word left/right"),
        ]),
        Line::from(vec![
            Span::styled(
                "  Cmd-Left/Right",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("Line start/end (macOS)"),
        ]),
        Line::from(vec![
            Span::styled(
                "  Esc/Ctrl-C",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("Cancel"),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "Commands",
            Style::default().add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled(
                "  :w        ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("Save review session"),
        ]),
        Line::from(vec![
            Span::styled(
                "  :e        ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("Reload diff files"),
        ]),
        Line::from(vec![
            Span::styled(
                "  :clip     ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("Copy review to clipboard"),
        ]),
        Line::from(vec![
            Span::styled(
                "  :set wrap ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("Enable line wrap in diff view"),
        ]),
        Line::from(vec![
            Span::styled(
                "  :set wrap!",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("Toggle line wrap in diff view"),
        ]),
        Line::from(vec![
            Span::styled(
                "  :clear    ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("Clear all comments"),
        ]),
        Line::from(vec![
            Span::styled(
                "  :q        ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("Quit"),
        ]),
        Line::from(vec![
            Span::styled(
                "  :wq       ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("Save and quit"),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled(
                "  ?         ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("Toggle this help"),
        ]),
    ];

    // Update help state with total lines and viewport height
    let total_lines = help_text.len();
    let viewport_height = inner.height as usize;
    app.help_state.total_lines = total_lines;
    app.help_state.viewport_height = viewport_height;

    // Calculate if we can scroll
    let can_scroll_up = app.help_state.scroll_offset > 0;
    let can_scroll_down = app.help_state.scroll_offset + viewport_height < total_lines;

    // Apply scroll offset
    let visible_lines: Vec<Line> = help_text
        .into_iter()
        .skip(app.help_state.scroll_offset)
        .take(viewport_height)
        .collect();

    let paragraph = Paragraph::new(visible_lines);
    frame.render_widget(paragraph, inner);

    // Render scroll indicators
    let indicator_style = Style::default().fg(Color::DarkGray);

    if can_scroll_up {
        let up_indicator = Paragraph::new(Line::from(Span::styled("▲ more", indicator_style)));
        let up_area = Rect {
            x: inner.x + inner.width.saturating_sub(8),
            y: inner.y,
            width: 7,
            height: 1,
        };
        frame.render_widget(up_indicator, up_area);
    }

    if can_scroll_down {
        let down_indicator = Paragraph::new(Line::from(Span::styled("▼ more", indicator_style)));
        let down_area = Rect {
            x: inner.x + inner.width.saturating_sub(8),
            y: inner.y + inner.height.saturating_sub(1),
            width: 7,
            height: 1,
        };
        frame.render_widget(down_indicator, down_area);
    }
}

fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let vertical = Layout::vertical([Constraint::Percentage(percent_y)]).flex(Flex::Center);
    let horizontal = Layout::horizontal([Constraint::Percentage(percent_x)]).flex(Flex::Center);
    let [area] = vertical.areas(area);
    let [area] = horizontal.areas(area);
    area
}
