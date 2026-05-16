use ratatui::{
    Frame,
    layout::Rect,
    text::Line,
    widgets::{Block, Borders, Paragraph},
};

use crate::app::{App, FocusedPanel};
use crate::ui::commit_row::{CommitRowSpec, render_commit_row};
use crate::ui::styles;

pub(super) fn render_inline_commit_selector(frame: &mut Frame, app: &mut App, area: Rect) {
    let focused = app.focused_panel == FocusedPanel::CommitSelector;
    let theme = &app.theme;

    let block = Block::default()
        .title(" Commits ")
        .borders(Borders::ALL)
        .style(styles::panel_style(theme))
        .border_style(styles::border_style(theme, focused));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    app.commit_list_viewport_height = inner.height as usize;
    app.commit_list_inner_area = Some(inner);

    let items: Vec<Line> = app
        .review_commits
        .iter()
        .enumerate()
        .map(|(i, commit)| {
            render_commit_row(&CommitRowSpec {
                commit,
                is_cursor: i == app.commit_list_cursor,
                is_selected: app.is_commit_selected(i),
                theme,
            })
        })
        .collect();

    let visible_items: Vec<Line> = items
        .into_iter()
        .skip(app.commit_list_scroll_offset)
        .take(inner.height as usize)
        .collect();

    frame.render_widget(
        Paragraph::new(visible_items).style(styles::panel_style(theme)),
        inner,
    );
}
