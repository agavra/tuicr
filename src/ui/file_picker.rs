//! `<leader>p` file picker modal (`InputMode::FilePicker`). Laid out like the
//! `:theme` picker (`ui/theme_picker.rs`); this module only renders the
//! current `FilePickerState`.

use crate::ui::app_layout::centered_rect;
use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Layout},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph},
};

use crate::app::App;
use crate::ui::diff_view::truncate_path_smart;
use crate::ui::styles;

/// Diff-area width below which the popup spans the whole diff.
const NARROW_WIDTH: u16 = 80;

pub fn render_file_picker(frame: &mut Frame, app: &mut App) {
    let theme = &app.theme;
    // Wider than the theme picker: paths need the room. In a narrow pane (the
    // case the picker exists for, with the file list hidden) take the whole
    // diff width rather than a share of it.
    let diff_area = app.diff_area.unwrap_or(frame.area());
    let percent_x = if diff_area.width < NARROW_WIDTH {
        100
    } else {
        70
    };
    let area = centered_rect(percent_x, 60, diff_area);
    frame.render_widget(Clear, area);

    let filtered = app.file_picker.filtered_indices();
    let title = format!(
        " Go to File ({}/{}) ",
        filtered.len(),
        app.file_picker.candidates.len()
    );
    let block = Block::default()
        .title(title)
        .title_alignment(Alignment::Center)
        .borders(Borders::ALL)
        .style(styles::popup_style(theme))
        .border_style(styles::border_style(theme, true));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let filtering = app.file_picker_filtering();
    let filter_height: u16 = if filtering { 1 } else { 0 };
    let [filter_area, list_area, footer_area] = Layout::vertical([
        Constraint::Length(filter_height),
        Constraint::Min(0),
        Constraint::Length(1),
    ])
    .areas(inner);

    if filter_height > 0 {
        let text = app.file_picker.draft.as_deref().unwrap_or("");
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("/", Style::default().fg(theme.fg_secondary)),
                Span::styled(text, Style::default().fg(theme.fg_primary)),
            ])),
            filter_area,
        );
    }

    if filtered.is_empty() {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "No files match",
                styles::dim_style(theme),
            ))),
            list_area,
        );
    } else {
        // Status glyph + space take two cells; long paths lose leading
        // directories (`…/github/review_threads.rs`) so the basename stays.
        let path_width = usize::from(list_area.width).saturating_sub(2).max(1);
        let items: Vec<ListItem> = filtered
            .iter()
            .map(|&candidate_idx| {
                let (file_idx, path) = &app.file_picker.candidates[candidate_idx];
                let status = app
                    .diff_files
                    .get(*file_idx)
                    .map(|file| file.status.as_char())
                    .unwrap_or(' ');
                ListItem::new(Line::from(vec![
                    Span::styled(
                        format!("{status} "),
                        styles::file_status_style(theme, status),
                    ),
                    Span::raw(truncate_path_smart(path, path_width)),
                ]))
            })
            .collect();
        let list = List::new(items)
            .style(styles::panel_style(theme))
            .highlight_style(styles::selected_style(theme));
        frame.render_stateful_widget(list, list_area, &mut app.file_picker.list_state);
    }

    let footer = if filtering {
        "type to filter \u{00b7} \u{2191}/\u{2193} move \u{00b7} \u{21b5} open \u{00b7} esc cancel filter"
    } else {
        "j/k move \u{00b7} \u{21b5} open \u{00b7} / filter \u{00b7} esc cancel"
    };
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            footer,
            Style::default()
                .fg(theme.fg_secondary)
                .add_modifier(Modifier::DIM),
        ))),
        footer_area,
    );
}
