//! Runtime `:theme` picker modal (`InputMode::ThemePicker`). Live preview
//! happens on `App::theme` as the selection or filter changes; this module
//! only renders the current `ThemePickerState`.

use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Flex, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph},
};

use crate::app::App;
use crate::ui::styles;

pub fn render_theme_picker(frame: &mut Frame, app: &mut App) {
    let theme = &app.theme;
    let area = centered_rect(50, 60, app.diff_area.unwrap_or(frame.area()));
    frame.render_widget(Clear, area);

    let block = Block::default()
        .title(" Switch Theme ")
        .title_alignment(Alignment::Center)
        .borders(Borders::ALL)
        .style(styles::popup_style(theme))
        .border_style(styles::border_style(theme, true));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let filtering = app.theme_picker_filtering();
    let filter_height: u16 = if filtering || app.theme_picker.filter.is_some() {
        1
    } else {
        0
    };
    let [filter_area, list_area, footer_area] = Layout::vertical([
        Constraint::Length(filter_height),
        Constraint::Min(0),
        Constraint::Length(1),
    ])
    .areas(inner);

    if filter_height > 0 {
        let prefix = if filtering { "/" } else { "filter: " };
        let text = if filtering {
            app.theme_picker.draft.as_deref().unwrap_or("")
        } else {
            app.theme_picker.filter.as_deref().unwrap_or("")
        };
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(prefix, Style::default().fg(theme.fg_secondary)),
                Span::styled(text, Style::default().fg(theme.fg_primary)),
            ])),
            filter_area,
        );
    }

    let filtered = app.theme_picker.filtered_indices();
    if filtered.is_empty() {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "No themes match",
                styles::dim_style(theme),
            ))),
            list_area,
        );
    } else {
        let items: Vec<ListItem> = filtered
            .iter()
            .map(|&candidate_idx| ListItem::new(app.theme_picker.candidates[candidate_idx].clone()))
            .collect();
        // Full-row bg highlight on the selected row, no leading cursor glyph
        // -- mirrors `file_list.rs`'s list rendering. `ListState` auto-scrolls
        // to keep the selection visible, so the picker is scrollable for
        // catalogs taller than the popup.
        let list = List::new(items)
            .style(styles::panel_style(theme))
            .highlight_style(styles::selected_style(theme));
        frame.render_stateful_widget(list, list_area, &mut app.theme_picker.list_state);
    }

    let footer = if filtering {
        "type to filter \u{00b7} \u{21b5} apply filter \u{00b7} esc cancel filter"
    } else {
        "j/k move \u{00b7} \u{21b5} apply \u{00b7} / filter \u{00b7} esc cancel"
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

fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let vertical = Layout::vertical([Constraint::Percentage(percent_y)]).flex(Flex::Center);
    let horizontal = Layout::horizontal([Constraint::Percentage(percent_x)]).flex(Flex::Center);
    let [area] = vertical.areas(area);
    let [area] = horizontal.areas(area);
    area
}
