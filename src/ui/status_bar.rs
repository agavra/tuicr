use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Paragraph},
};

use crate::app::{App, DiffSource, InputMode, MessageType};
use crate::ui::styles;

pub fn render_header(frame: &mut Frame, app: &App, area: Rect) {
    let theme = &app.theme;
    let vcs_type = &app.vcs_info.vcs_type;
    let branch = app.vcs_info.branch_name.as_deref().unwrap_or("detached");

    let title = " tuicr - Code Review ".to_string();
    let vcs_info = format!("[{}:{}] ", vcs_type, branch);

    // Show diff source info
    let source_info = match &app.diff_source {
        DiffSource::WorkingTree => String::new(),
        DiffSource::CommitRange(commits) => {
            if commits.len() == 1 {
                format!("[commit {}] ", &commits[0][..7.min(commits[0].len())])
            } else {
                format!("[{} commits] ", commits.len())
            }
        }
    };

    let progress = format!("{}/{} reviewed ", app.reviewed_count(), app.file_count());

    let title_span = Span::styled(title, styles::header_style(theme));
    let vcs_span = Span::styled(vcs_info, Style::default().fg(theme.fg_secondary));
    let source_span = Span::styled(source_info, Style::default().fg(theme.diff_hunk_header));
    let progress_span = Span::styled(
        progress,
        if app.reviewed_count() == app.file_count() {
            styles::reviewed_style(theme)
        } else {
            styles::pending_style(theme)
        },
    );

    let line = Line::from(vec![title_span, vcs_span, source_span, progress_span]);

    let header = Paragraph::new(line)
        .style(styles::status_bar_style(theme))
        .block(Block::default());

    frame.render_widget(header, area);
}

pub fn render_status_bar(frame: &mut Frame, app: &App, area: Rect) {
    let theme = &app.theme;

    // In command/search mode, show the input on the left (vim-style)
    let left_spans = if matches!(app.input_mode, InputMode::Command | InputMode::Search) {
        let prefix = if app.input_mode == InputMode::Command {
            ":"
        } else {
            "/"
        };
        let buffer = if app.input_mode == InputMode::Command {
            &app.command_buffer
        } else {
            &app.search_buffer
        };
        let command_text = format!("{}{}", prefix, buffer);
        vec![Span::styled(
            command_text,
            Style::default().fg(theme.fg_primary),
        )]
    } else {
        let mode_str = match app.input_mode {
            InputMode::Normal => " NORMAL ",
            InputMode::Command => " COMMAND ",
            InputMode::Search => " SEARCH ",
            InputMode::Comment => " COMMENT ",
            InputMode::Help => " HELP ",
            InputMode::Confirm => " CONFIRM ",
            InputMode::CommitSelect => " SELECT ",
        };

        let mode_span = Span::styled(mode_str, styles::mode_style(theme));

        let hints = match app.input_mode {
            InputMode::Normal => {
                " j/k:scroll  {/}:file  r:reviewed  c:comment  /:search  ?:help  :q:quit "
            }
            InputMode::Command => " Enter:execute  Esc:cancel ",
            InputMode::Search => " Enter:search  Esc:cancel ",
            InputMode::Comment => " Ctrl-S:save  Esc:cancel ",
            InputMode::Help => " q/?/Esc:close ",
            InputMode::Confirm => " y:yes  n:no ",
            InputMode::CommitSelect => " j/k:navigate  Space:select  Enter:confirm  q:quit ",
        };
        let hints_span = Span::styled(hints, Style::default().fg(theme.fg_secondary));

        let dirty_indicator = if app.dirty {
            Span::styled(" [modified] ", Style::default().fg(theme.pending))
        } else {
            Span::raw("")
        };

        vec![mode_span, hints_span, dirty_indicator]
    };

    let left_width: usize = left_spans.iter().map(|s| s.content.len()).sum();

    // Build message span for right side with highlighted background
    let (message_span, message_width) = if let Some(msg) = &app.message {
        let (fg, bg) = match msg.message_type {
            MessageType::Info => (Color::Black, Color::Cyan),
            MessageType::Warning => (Color::Black, theme.pending),
            MessageType::Error => (Color::White, theme.comment_issue),
        };
        let content = format!(" {} ", msg.content);
        let width = content.len();
        (
            Span::styled(
                content,
                Style::default().fg(fg).bg(bg).add_modifier(Modifier::BOLD),
            ),
            width,
        )
    } else {
        (Span::raw(""), 0)
    };

    // Calculate padding to push message to the right
    let total_width = area.width as usize;
    let padding_width = total_width.saturating_sub(left_width + message_width);
    let padding = Span::raw(" ".repeat(padding_width));

    let mut spans = left_spans;
    spans.push(padding);
    if message_width > 0 {
        spans.push(message_span);
    }

    let line = Line::from(spans);

    let status = Paragraph::new(line)
        .style(styles::status_bar_style(theme))
        .block(Block::default());

    frame.render_widget(status, area);
}
