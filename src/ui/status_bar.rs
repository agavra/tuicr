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
    let branch = app.repo_info.branch_name.as_deref().unwrap_or("detached");

    let title = " tuicr - Code Review ".to_string();
    let branch_info = format!("[{}] ", branch);

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

    let title_span = Span::styled(title, styles::header_style());
    let branch_span = Span::styled(branch_info, Style::default().fg(styles::FG_SECONDARY));
    let source_span = Span::styled(source_info, Style::default().fg(styles::DIFF_HUNK_HEADER));
    let progress_span = Span::styled(
        progress,
        if app.reviewed_count() == app.file_count() {
            styles::reviewed_style()
        } else {
            styles::pending_style()
        },
    );

    let line = Line::from(vec![title_span, branch_span, source_span, progress_span]);

    let header = Paragraph::new(line)
        .style(styles::status_bar_style())
        .block(Block::default());

    frame.render_widget(header, area);
}

pub fn render_status_bar(frame: &mut Frame, app: &App, area: Rect) {
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
            Style::default().fg(styles::FG_PRIMARY),
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

        let mode_span = Span::styled(mode_str, styles::mode_style());

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
        let hints_span = Span::styled(hints, Style::default().fg(styles::FG_SECONDARY));

        let dirty_indicator = if app.dirty {
            Span::styled(" [modified] ", Style::default().fg(styles::PENDING))
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
            MessageType::Warning => (Color::Black, styles::PENDING),
            MessageType::Error => (Color::White, styles::COMMENT_ISSUE),
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
        .style(styles::status_bar_style())
        .block(Block::default());

    frame.render_widget(status, area);
}
