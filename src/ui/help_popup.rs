use ratatui::{
    Frame,
    layout::{Constraint, Flex, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
};

use crate::ui::styles;

pub fn render_help(frame: &mut Frame) {
    let area = centered_rect(60, 70, frame.area());

    // Clear the area behind the popup
    frame.render_widget(Clear, area);

    let block = Block::default()
        .title(" Help - Press ? or Esc to close ")
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
                "  Tab       ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("Toggle focus file list/diff"),
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
                "  1-4       ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("Set type: Note/Suggestion/Issue/Praise"),
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

    let paragraph = Paragraph::new(help_text);
    frame.render_widget(paragraph, inner);
}

fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let vertical = Layout::vertical([Constraint::Percentage(percent_y)]).flex(Flex::Center);
    let horizontal = Layout::horizontal([Constraint::Percentage(percent_x)]).flex(Flex::Center);
    let [area] = vertical.areas(area);
    let [area] = horizontal.areas(area);
    area
}
