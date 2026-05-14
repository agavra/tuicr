use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem},
};
use std::path::Path;
use unicode_width::UnicodeWidthStr;

use crate::app::{App, FileTreeItem, FocusedPanel};
use crate::ui::diff_view::apply_horizontal_scroll;
use crate::ui::styles;

pub(super) fn render_file_list(frame: &mut Frame, app: &mut App, area: Rect) {
    let focused = app.focused_panel == FocusedPanel::FileList;

    let block = Block::default()
        .title(" Files ")
        .borders(Borders::ALL)
        .style(styles::panel_style(&app.theme))
        .border_style(styles::border_style(&app.theme, focused));

    let inner = block.inner(area);
    app.file_list_inner_area = Some(inner);
    let visible_items = app.build_visible_items();

    let max_content_width = visible_items
        .iter()
        .map(|item| match item {
            FileTreeItem::Directory { path, depth, .. } => {
                let dir_name = Path::new(path)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or(path);
                depth * 2 + 2 + dir_name.width() + 1
            }
            FileTreeItem::File { file_idx, depth } => {
                let file = &app.diff_files[*file_idx];
                let filename = file
                    .display_path()
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("?");
                depth * 2 + 3 + 3 + filename.width()
            }
        })
        .max()
        .unwrap_or(0);

    app.file_list_state.viewport_width = inner.width as usize;
    app.file_list_state.viewport_height = inner.height as usize;
    app.file_list_state.max_content_width = max_content_width;

    let max_scroll_x = max_content_width.saturating_sub(inner.width as usize);
    if app.file_list_state.scroll_x > max_scroll_x {
        app.file_list_state.scroll_x = max_scroll_x;
    }
    let scroll_x = app.file_list_state.scroll_x;

    // When diff panel is focused, sync file list selection to current file
    // But preserve the current offset to not interfere with manual scrolling
    if app.focused_panel == FocusedPanel::Diff {
        let current_file_idx = app.diff_state.current_file_idx;
        for (tree_idx, item) in visible_items.iter().enumerate() {
            if let FileTreeItem::File { file_idx, .. } = item
                && *file_idx == current_file_idx
            {
                if app.file_list_state.selected() != tree_idx {
                    // Save current offset before changing selection
                    let current_offset = app.file_list_state.list_state.offset();
                    app.file_list_state.select(tree_idx);
                    // Restore offset to prevent auto-scrolling
                    *app.file_list_state.list_state.offset_mut() = current_offset;
                }
                break;
            }
        }
    }

    let selected_idx = app.file_list_state.selected();

    let items: Vec<ListItem> = visible_items
        .iter()
        .enumerate()
        .map(|(i, item)| {
            let is_selected = i == selected_idx;

            match item {
                FileTreeItem::Directory {
                    path,
                    depth,
                    expanded,
                } => {
                    let indent = "  ".repeat(*depth);
                    let icon = if *expanded { "▾" } else { "▸" };
                    let dir_name = Path::new(path)
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or(path);

                    let style = if is_selected {
                        styles::selected_style(&app.theme).add_modifier(Modifier::UNDERLINED)
                    } else {
                        Style::default()
                    };

                    let line = Line::from(vec![
                        Span::styled(indent, Style::default()),
                        Span::styled(format!("{icon} "), styles::dir_icon_style(&app.theme)),
                        Span::styled(format!("{dir_name}/"), style),
                    ]);

                    ListItem::new(apply_horizontal_scroll(line, scroll_x))
                }
                FileTreeItem::File { file_idx, depth } => {
                    let file = &app.diff_files[*file_idx];
                    let path = file.display_path();
                    let is_reviewed = app.session.is_file_reviewed(path);
                    let review_mark = if is_reviewed { "✓" } else { " " };

                    let style = if is_selected {
                        styles::selected_style(&app.theme).add_modifier(Modifier::UNDERLINED)
                    } else {
                        Style::default()
                    };

                    let line = if file.is_commit_message {
                        Line::from(vec![
                            Span::styled(
                                format!("[{review_mark}]"),
                                if is_reviewed {
                                    styles::reviewed_style(&app.theme)
                                } else {
                                    styles::pending_style(&app.theme)
                                },
                            ),
                            Span::styled("   Commit Message".to_string(), style),
                        ])
                    } else {
                        let filename = path.file_name().and_then(|n| n.to_str()).unwrap_or("?");
                        let status = file.status.as_char();
                        let indent = "  ".repeat(*depth);
                        Line::from(vec![
                            Span::styled(indent, Style::default()),
                            Span::styled(
                                format!("[{review_mark}]"),
                                if is_reviewed {
                                    styles::reviewed_style(&app.theme)
                                } else {
                                    styles::pending_style(&app.theme)
                                },
                            ),
                            Span::styled(
                                format!(" {status} "),
                                styles::file_status_style(&app.theme, status),
                            ),
                            Span::styled(filename.to_string(), style),
                        ])
                    };

                    ListItem::new(apply_horizontal_scroll(line, scroll_x))
                }
            }
        })
        .collect();

    let list = List::new(items)
        .style(styles::panel_style(&app.theme))
        .block(block);

    frame.render_stateful_widget(list, area, &mut app.file_list_state.list_state);
}
