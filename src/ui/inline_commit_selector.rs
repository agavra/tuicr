use ratatui::{
    Frame,
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};

use crate::app::{App, FocusedPanel};
use crate::ui::styles;
use crate::ui::text_utils::truncate_str;

pub(super) fn render_inline_commit_selector(frame: &mut Frame, app: &mut App, area: Rect) {
    let focused = app.focused_panel == FocusedPanel::CommitSelector;
    let block = Block::default()
        .title(" Commits ")
        .borders(Borders::ALL)
        .border_style(styles::border_style(&app.theme, focused));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Update viewport height for scroll
    app.commit_list_viewport_height = inner.height as usize;
    app.commit_list_inner_area = Some(inner);

    {
        let range = app.commit_selection_range;
        let total_commits = app.review_commits.len();

        let items: Vec<Line> = app
            .review_commits
            .iter()
            .take(total_commits)
            .enumerate()
            .map(|(i, commit)| {
                let is_selected = app.is_commit_selected(i);
                let is_cursor = i == app.commit_list_cursor;

                // Range boundary indicators
                let range_marker = match range {
                    Some((start, end)) if i == start && i == end => "\u{2500}",
                    Some((start, _)) if i == start => "\u{250c}",
                    Some((_, end)) if i == end => "\u{2514}",
                    Some((start, end)) if i > start && i < end => "\u{2502}",
                    _ => " ",
                };

                let checkbox = if is_selected { "[x]" } else { "[ ]" };

                let style = if is_cursor {
                    styles::selected_style(&app.theme)
                } else if is_selected {
                    Style::default().fg(app.theme.fg_secondary)
                } else {
                    Style::default()
                };

                let checkbox_style = if is_selected {
                    styles::reviewed_style(&app.theme)
                } else {
                    styles::pending_style(&app.theme)
                };

                let range_style = if is_selected {
                    styles::reviewed_style(&app.theme)
                } else {
                    Style::default().fg(app.theme.fg_secondary)
                };

                let pointer = if is_cursor { "> " } else { "  " };

                let time_str = commit.time.format("%Y-%m-%d").to_string();
                let mut spans = vec![
                    Span::styled(pointer.to_string(), style),
                    Span::styled(format!("{} ", range_marker), range_style),
                    Span::styled(format!("{} ", checkbox), checkbox_style),
                    Span::styled(
                        format!("{} ", commit.short_id),
                        styles::hash_style(&app.theme),
                    ),
                ];

                if let Some(branch_name) = &commit.branch_name {
                    spans.push(Span::styled(
                        format!("[{}] ", truncate_str(branch_name, 20)),
                        styles::branch_style(&app.theme),
                    ));
                }

                spans.push(Span::styled(truncate_str(&commit.summary, 50), style));
                spans.push(Span::styled(
                    format!(" ({}, {})", commit.author, time_str),
                    Style::default().fg(app.theme.fg_secondary),
                ));

                Line::from(spans)
            })
            .collect();

        let visible_items: Vec<Line> = items
            .into_iter()
            .skip(app.commit_list_scroll_offset)
            .take(inner.height as usize)
            .collect();

        let paragraph = Paragraph::new(visible_items);
        frame.render_widget(paragraph, inner);
    }
}
