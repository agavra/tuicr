use ratatui::{
    Frame,
    layout::Rect,
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem},
};
use std::path::Path;
use unicode_width::UnicodeWidthStr;

use crate::app::{App, FileTreeItem, FocusedPanel};
use crate::ui::diff_view::apply_horizontal_scroll_without_prefix;
use crate::ui::styles;

const EXPANDED_GLYPH: &str = "\u{25bc}"; // ▼
const COLLAPSED_GLYPH: &str = "\u{25b6}"; // ▶
const REVIEWED_BOX: &str = "\u{25a3}"; // ▣
const UNREVIEWED_BOX: &str = "\u{25a2}"; // ▢

/// Compact text badges used only by flat mode. Keeping these ASCII-only makes
/// file-type hints render consistently in every terminal font.
fn file_type_icon(path: &Path) -> &'static str {
    let filename = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("");
    if filename.eq_ignore_ascii_case("dockerfile") {
        return "[DOCKER]";
    }
    if filename.eq_ignore_ascii_case("makefile") {
        return "[MAKE]";
    }

    match path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.to_ascii_lowercase())
        .as_deref()
    {
        Some("js") | Some("jsx") | Some("mjs") | Some("cjs") => "[JS]",
        Some("ts") | Some("tsx") => "[TS]",
        Some("vue") => "[VUE]",
        Some("java") => "[JAVA]",
        Some("sql") => "[SQL]",
        Some("md") | Some("markdown") => "[MD]",
        Some("py") => "[PY]",
        Some("rs") => "[RS]",
        Some("go") => "[GO]",
        Some("rb") => "[RB]",
        Some("php") => "[PHP]",
        Some("html") | Some("htm") => "[HTML]",
        Some("css") | Some("scss") | Some("sass") | Some("less") => "[CSS]",
        Some("json") => "[JSON]",
        Some("yaml") | Some("yml") => "[YAML]",
        Some("sh") | Some("bash") | Some("zsh") | Some("fish") => "[SH]",
        Some("c") | Some("h") => "[C]",
        Some("cc") | Some("cpp") | Some("cxx") | Some("hpp") => "[C++]",
        Some("cs") => "[C#]",
        Some("kt") | Some("kts") => "[KT]",
        Some("swift") => "[SWIFT]",
        Some("xml") => "[XML]",
        Some("toml") => "[TOML]",
        Some("lock") => "[LOCK]",
        _ => "[FILE]",
    }
}

pub(super) fn render_file_list(frame: &mut Frame, app: &mut App, area: Rect) {
    let focused = app.focused_panel == FocusedPanel::FileList;

    let mode = if app.file_list_flat { "flat" } else { "tree" };
    let title = format!(
        " Files \u{00b7} {}/{} \u{00b7} {mode} ",
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
            FileTreeItem::Directory { path, depth, .. } => {
                let dir_name = Path::new(path)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or(path);
                depth * 2 + 2 + dir_name.width() + 1
            }
            FileTreeItem::File { file_idx, depth } => {
                let file = &app.diff_files[*file_idx];
                let path = file.display_path();
                let name = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("?")
                    .to_string();
                let icon_width = if app.file_list_flat {
                    file_type_icon(path).width() + 1
                } else {
                    0
                };
                depth * 2 + 4 + icon_width + name.width()
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

    let items: Vec<ListItem> = visible_items
        .iter()
        .map(|item| {
            let line = match item {
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
                        let filename = path
                            .file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or("?")
                            .to_string();
                        let indent = "  ".repeat(*depth);
                        let mut spans = vec![
                            Span::raw(indent),
                            Span::styled(format!("{checkbox} "), checkbox_style),
                        ];
                        if app.file_list_flat {
                            let badge = file_type_icon(path);
                            spans.push(Span::styled(
                                format!("{badge} "),
                                styles::file_type_badge_style(badge),
                            ));
                        }
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

            ListItem::new(apply_horizontal_scroll_without_prefix(line, scroll_x))
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

#[cfg(test)]
mod tests {
    use ratatui::text::{Line, Span};
    use std::path::Path;

    use super::{apply_horizontal_scroll_without_prefix, file_type_icon};

    #[test]
    fn horizontal_scroll_removes_file_tree_indentation() {
        let line = Line::from(vec![
            Span::raw("    "),
            Span::raw("▢ M "),
            Span::raw("nested.rs"),
        ]);

        let scrolled = apply_horizontal_scroll_without_prefix(line, 4);
        let content: String = scrolled
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect();

        assert_eq!(content, "▢ M nested.rs");
    }

    #[test]
    fn file_type_icon_covers_common_review_languages() {
        assert_eq!(file_type_icon(Path::new("app.js")), "[JS]");
        assert_eq!(file_type_icon(Path::new("app.vue")), "[VUE]");
        assert_eq!(file_type_icon(Path::new("app.java")), "[JAVA]");
        assert_eq!(file_type_icon(Path::new("query.sql")), "[SQL]");
        assert_eq!(file_type_icon(Path::new("README.md")), "[MD]");
        assert_eq!(file_type_icon(Path::new("unknown")), "[FILE]");
    }

    #[test]
    fn file_type_icon_matches_extensions_case_insensitively() {
        assert_eq!(
            file_type_icon(Path::new("APP.JS")),
            file_type_icon(Path::new("app.js"))
        );
        assert_eq!(
            file_type_icon(Path::new("Dockerfile")),
            file_type_icon(Path::new("dockerfile"))
        );
    }
}
