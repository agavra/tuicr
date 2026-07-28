use ratatui::{
    Frame,
    layout::Rect,
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem},
};
use std::path::Path;
use unicode_width::UnicodeWidthStr;

use crate::app::{App, FileTreeItem, FocusedPanel};
use crate::ui::diff_view::apply_horizontal_scroll;
use crate::ui::styles;

const EXPANDED_GLYPH: &str = "\u{25bc}"; // ▼
const COLLAPSED_GLYPH: &str = "\u{25b6}"; // ▶
const REVIEWED_BOX: &str = "\u{25a3}"; // ▣
const UNREVIEWED_BOX: &str = "\u{25a2}"; // ▢

pub(super) fn render_file_list(frame: &mut Frame, app: &mut App, area: Rect) {
    let focused = app.focused_panel == FocusedPanel::FileList;

    let title = format!(
        " Files \u{00b7} {}/{} ",
        app.reviewed_count(),
        app.file_count()
    );
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .style(styles::panel_style(&app.theme))
        .border_style(styles::border_style(&app.theme, focused));

    let inner = block.inner(area);
    app.file_list_inner_area = Some(inner);
    let visible_items = app.build_visible_items();

    let max_content_width = visible_items
        .iter()
        .map(|item| match item {
            FileTreeItem::PrInfo => {
                let hint = app
                    .pr_info
                    .as_ref()
                    .map(pr_info_tree_hint)
                    .unwrap_or_default();
                if hint.is_empty() {
                    "PR Description".chars().count()
                } else {
                    format!("PR Description · {hint}").chars().count()
                }
            }
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
                depth * 2 + 4 + filename.width()
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

    // When diff panel is focused, sync file list selection to current view
    // But preserve the current offset to not interfere with manual scrolling
    if app.focused_panel == FocusedPanel::Diff {
        if app.is_cursor_in_overview() && app.pr_info.is_some() {
            if let Some(tree_idx) = app.pr_info_tree_idx()
                && app.file_list_state.selected() != tree_idx
            {
                let current_offset = app.file_list_state.list_state.offset();
                app.file_list_state.select(tree_idx);
                *app.file_list_state.list_state.offset_mut() = current_offset;
            }
        } else {
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
    }

    let items: Vec<ListItem> = visible_items
        .iter()
        .map(|item| {
            let line = match item {
                FileTreeItem::PrInfo => {
                    let hint = app
                        .pr_info
                        .as_ref()
                        .map(pr_info_tree_hint)
                        .unwrap_or_default();
                    let label = if hint.is_empty() {
                        "PR Description".to_string()
                    } else {
                        format!("PR Description · {hint}")
                    };
                    Line::from(vec![Span::styled(
                        label,
                        styles::file_header_style(&app.theme),
                    )])
                }
                FileTreeItem::Directory {
                    path,
                    depth,
                    expanded,
                } => {
                    let indent = "  ".repeat(*depth);
                    let icon = if *expanded {
                        EXPANDED_GLYPH
                    } else {
                        COLLAPSED_GLYPH
                    };
                    let dir_name = Path::new(path)
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or(path);
                    Line::from(vec![
                        Span::raw(indent),
                        Span::styled(format!("{icon} "), styles::dir_icon_style(&app.theme)),
                        Span::raw(format!("{dir_name}/")),
                    ])
                }
                FileTreeItem::File { file_idx, depth } => {
                    let file = &app.diff_files[*file_idx];
                    let path = file.display_path();
                    let is_reviewed = app.session.is_file_reviewed(path);
                    let checkbox = if is_reviewed {
                        REVIEWED_BOX
                    } else {
                        UNREVIEWED_BOX
                    };
                    let checkbox_style = if is_reviewed {
                        styles::reviewed_style(&app.theme)
                    } else {
                        styles::pending_style(&app.theme)
                    };
                    if file.is_commit_message {
                        Line::from(vec![
                            Span::styled(format!("{checkbox} "), checkbox_style),
                            Span::raw(format!("  {}", path.display())),
                        ])
                    } else {
                        let filename = path.file_name().and_then(|n| n.to_str()).unwrap_or("?");
                        let indent = "  ".repeat(*depth);
                        let mut spans = vec![
                            Span::raw(indent),
                            Span::styled(format!("{checkbox} "), checkbox_style),
                        ];
                        // Pristine mode reviews unchanged code; the M/A/D
                        // badge would lie. Suppress it and leave the row as
                        // checkbox + filename.
                        if !app.is_pristine_mode {
                            let status = file.status.as_char();
                            spans.push(Span::styled(
                                format!("{status} "),
                                styles::file_status_style(&app.theme, status),
                            ));
                        }
                        spans.push(Span::raw(filename.to_string()));
                        Line::from(spans)
                    }
                }
            };

            ListItem::new(apply_horizontal_scroll(line, scroll_x))
        })
        .collect();

    // Full-row bg highlight on the selected row (no leading cursor glyph or
    // underline modifier) — mirrors how the diff view highlights its cursor
    // line.
    let list = List::new(items)
        .style(styles::panel_style(&app.theme))
        .highlight_style(styles::selected_style(&app.theme))
        .block(block);

    frame.render_stateful_widget(list, area, &mut app.file_list_state.list_state);
}

fn pr_info_tree_hint(info: &crate::forge::traits::PullRequestInfo) -> String {
    let details = &info.details;
    if details.merged_at.is_some() {
        return "merged".to_string();
    }
    if details.closed {
        return "closed".to_string();
    }
    if details.is_draft {
        return "draft".to_string();
    }
    let failing = info.checks.iter().filter(|check| {
        let outcome = check
            .conclusion
            .as_deref()
            .or(check.status.as_deref())
            .unwrap_or("")
            .to_ascii_uppercase();
        !matches!(
            outcome.as_str(),
            "SUCCESS" | "COMPLETED" | "NEUTRAL" | "SKIPPED" | "IN_PROGRESS" | "QUEUED"
                | "PENDING" | "WAITING" | "REQUESTED" | ""
        )
    }).count();
    if failing > 0 {
        return format!("{failing} checks failing");
    }
    if let Some(decision) = &info.review_decision {
        return decision.replace('_', " ").to_lowercase();
    }
    details.state.to_lowercase()
}
