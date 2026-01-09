use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
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
    let mode_str = match app.input_mode {
        InputMode::Normal => " NORMAL ",
        InputMode::Command => " COMMAND ",
        InputMode::Comment => " COMMENT ",
        InputMode::Help => " HELP ",
        InputMode::Confirm => " CONFIRM ",
        InputMode::CommitSelect => " SELECT ",
    };

    let mode_span = Span::styled(mode_str, styles::mode_style());

    let hints = match app.input_mode {
        InputMode::Normal => " j/k:scroll  {/}:file  r:reviewed  c:comment  ?:help  :q:quit ",
        InputMode::Command => " Enter:execute  Esc:cancel ",
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

    let message = if let Some(msg) = &app.message {
        let color = match msg.message_type {
            MessageType::Info => styles::FG_PRIMARY,
            MessageType::Warning => styles::PENDING, // Amber/yellow
            MessageType::Error => styles::COMMENT_ISSUE, // Red
        };
        Span::styled(
            format!(" {} ", msg.content),
            Style::default().fg(color).add_modifier(Modifier::ITALIC),
        )
    } else {
        Span::raw("")
    };

    let mut spans = vec![mode_span, hints_span, dirty_indicator];
    if !message.content.is_empty() {
        spans.push(message);
    }

    let line = Line::from(spans);

    let status = Paragraph::new(line)
        .style(styles::status_bar_style())
        .block(Block::default());

    frame.render_widget(status, area);
}

pub fn render_command_line(frame: &mut Frame, app: &App, area: Rect) {
    let content = match app.input_mode {
        InputMode::Command => format!(":{}", app.command_buffer),
        _ => String::new(),
    };

    let line = Paragraph::new(content)
        .style(Style::default().fg(styles::FG_PRIMARY))
        .block(Block::default());

    frame.render_widget(line, area);
}
