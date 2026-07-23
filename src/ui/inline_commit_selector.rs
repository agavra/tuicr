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

    // Rows are built in data order (index into newest-first `review_commits`),
    // then reversed for ascending display so the oldest commit sits on top.
    let mut items: Vec<Line> = app
        .review_commits
        .iter()
        .enumerate()
        .map(|(i, commit)| {
            render_commit_row(&CommitRowSpec {
                commit,
                is_cursor: i == app.commit_list_cursor,
                is_selected: app.is_commit_selected(i),
                is_reviewed: app.is_commit_reviewed_by_viewer(i),
                theme,
            })
        })
        .collect();

    let height = inner.height as usize;
    let n = app.review_commits.len();
    if app.commits_ascending() {
        items.reverse();
        // The renderer owns the scroll offset in display space for ascending
        // order (descending order lets commit_select_up/down maintain it):
        // clamp it so the cursor's display row stays visible.
        if n > 0 && height > 0 {
            let cursor_row = app.commit_data_index(app.commit_list_cursor);
            let offset = app.commit_list_scroll_offset;
            let offset = if cursor_row < offset {
                cursor_row
            } else if cursor_row >= offset + height {
                cursor_row + 1 - height
            } else {
                offset
            };
            app.commit_list_scroll_offset = offset.min(n.saturating_sub(1));
        }
    }

    let visible_items: Vec<Line> = items
        .into_iter()
        .skip(app.commit_list_scroll_offset)
        .take(height)
        .collect();

    frame.render_widget(
        Paragraph::new(visible_items).style(styles::panel_style(theme)),
        inner,
    );
}
