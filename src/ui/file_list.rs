use ratatui::{
    Frame,
    layout::{Position, Rect},
    style::Style,
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

    let mut title = format!(
        " Files \u{00b7} {}/{} ",
        app.reviewed_count(),
        app.file_count()
    );
    // A filter changes what the counts above mean, so say how much is hidden.
    if app.file_filter_active() {
        title.push_str(&format!(
            "\u{00b7} {} of {} ",
            app.file_count(),
            app.unfiltered_file_count()
        ));
    }
    let mut block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .style(styles::panel_style(&app.theme))
        .border_style(styles::border_style(&app.theme, focused));

    // Bottom border doubles as the filter status / prompt line. Using a
    // block title instead of reserving an inner row keeps the list geometry
    // (and therefore viewport_height and every scroll calculation) untouched.
    if let Some(bottom) = filter_footer(app) {
        block = block.title_bottom(bottom);
    }

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

    // Park the terminal cursor at the end of the prompt buffer so typing has
    // a visible insertion point.
    if let Some(draft) = app.file_tree_draft() {
        let prefix = PROMPT_PAD + 2 + draft.buffer.width() as u16;
        frame.set_cursor_position(Position {
            x: (area.x + prefix).min(area.x + area.width.saturating_sub(1)),
            y: area.y + area.height.saturating_sub(1),
        });
    }
}

/// Leading `│` border plus one space before the prompt sigil.
const PROMPT_PAD: u16 = 2;

/// Bottom-border content for the file tree: the active prompt while one is
/// open, otherwise a summary of the applied filters and search.
fn filter_footer(app: &App) -> Option<Line<'static>> {
    let theme = &app.theme;

    if let Some(draft) = app.file_tree_draft() {
        return Some(Line::from(vec![
            Span::styled(
                format!(" {} ", draft.prompt.sigil()),
                styles::mode_style(theme),
            ),
            Span::styled(
                format!("{} ", draft.buffer),
                Style::default().fg(theme.fg_primary),
            ),
        ]));
    }

    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut push = |label: char, value: String| {
        if !spans.is_empty() {
            spans.push(Span::styled(
                " \u{00b7} ",
                Style::default().fg(theme.fg_dim),
            ));
        }
        spans.push(Span::styled(
            format!("{label}:"),
            Style::default().fg(theme.fg_dim),
        ));
        spans.push(Span::styled(value, Style::default().fg(theme.fg_secondary)));
    };

    // Only the filters earn a slot here: they change what the panes contain
    // and there is no other persistent cue for them. The `/` query is left
    // out on purpose — the panel is narrow (a third pattern truncates to
    // noise), and every `n`/`N` step reports the query and match position in
    // the status bar anyway.
    if let Some(include) = app.file_filter.include.as_ref() {
        push('i', include.source.clone());
    }
    if let Some(exclude) = app.file_filter.exclude.as_ref() {
        push('e', exclude.source.clone());
    }
    // Hiding reviewed files has no pattern to show, and the title fraction
    // deliberately keeps counting them, so this line is the only persistent
    // cue that rows are missing.
    if !app.show_reviewed() {
        if !spans.is_empty() {
            spans.push(Span::styled(
                " \u{00b7} ",
                Style::default().fg(theme.fg_dim),
            ));
        }
        spans.push(Span::styled(
            "reviewed hidden",
            Style::default().fg(theme.fg_secondary),
        ));
    }

    if spans.is_empty() {
        return None;
    }
    spans.insert(0, Span::raw(" "));
    spans.push(Span::raw(" "));
    Some(Line::from(spans))
}

#[cfg(test)]
mod tests {
    //! Render checks for the filter status/prompt line in the file tree's
    //! bottom border, driven through the real `ui::render`.
    use crate::app::{App, DiffSource, FileTreePrompt, FocusedPanel, InputMode};
    use crate::model::{DiffFile, DiffLine, FileStatus, ReviewSession, SessionDiffSource};
    use crate::vcs::traits::{VcsBackend, VcsInfo, VcsType};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use std::path::PathBuf;

    struct StubVcs(VcsInfo);
    impl VcsBackend for StubVcs {
        fn info(&self) -> &VcsInfo {
            &self.0
        }
        fn get_working_tree_diff(
            &self,
            _hl: &crate::syntax::SyntaxHighlighter,
        ) -> crate::error::Result<Vec<DiffFile>> {
            Ok(Vec::new())
        }
        fn fetch_context_lines(
            &self,
            _path: &std::path::Path,
            _status: FileStatus,
            _ref_commit: Option<&str>,
            _start: u32,
            _end: u32,
        ) -> crate::error::Result<Vec<DiffLine>> {
            Ok(Vec::new())
        }
        fn file_line_count(
            &self,
            _path: &std::path::Path,
            _status: FileStatus,
            _ref_commit: Option<&str>,
        ) -> crate::error::Result<u32> {
            Ok(0)
        }
    }

    fn file(path: &str) -> DiffFile {
        DiffFile {
            old_path: None,
            new_path: Some(PathBuf::from(path)),
            status: FileStatus::Modified,
            hunks: vec![],
            is_binary: false,
            is_too_large: false,
            is_commit_message: false,
            content_hash: 0,
        }
    }

    fn app_with(paths: &[&str]) -> App {
        let vcs_info = VcsInfo {
            root_path: PathBuf::from("/tmp"),
            head_commit: "head".into(),
            branch_name: Some("main".into()),
            vcs_type: VcsType::Git,
        };
        let session = ReviewSession::new(
            vcs_info.root_path.clone(),
            vcs_info.head_commit.clone(),
            vcs_info.branch_name.clone(),
            SessionDiffSource::WorkingTree,
        );
        let mut app = App::build(
            Box::new(StubVcs(vcs_info.clone())),
            vcs_info,
            crate::theme::Theme::dark(),
            None,
            false,
            paths.iter().map(|p| file(p)).collect(),
            session,
            DiffSource::WorkingTree,
            InputMode::Normal,
            Vec::new(),
            None,
            None,
        )
        .expect("build app");
        app.show_file_list = true;
        app.focused_panel = FocusedPanel::FileList;
        app
    }

    fn draw(app: &mut App) -> Buffer {
        let backend = TestBackend::new(120, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| crate::ui::render(frame, app))
            .expect("draw frame");
        terminal.backend().buffer().clone()
    }

    fn buffer_text(buffer: &Buffer) -> String {
        (0..buffer.area.height)
            .map(|y| {
                (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol().to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn should_render_the_prompt_sigil_and_buffer_while_a_prompt_is_open() {
        let mut app = app_with(&["src/main.rs", "README.md"]);
        app.begin_file_tree_prompt(FileTreePrompt::Include);
        for ch in r"\.rs$".chars() {
            app.file_tree_prompt_insert_char(ch);
        }

        let text = buffer_text(&draw(&mut app));

        assert!(
            text.contains(r"i \.rs$"),
            "expected the include prompt in the tree border, got:\n{text}"
        );
    }

    #[test]
    fn should_render_applied_filters_in_the_border_after_the_prompt_closes() {
        let mut app = app_with(&["src/main.rs", "tests/smoke.rs", "README.md"]);
        app.begin_file_tree_prompt(FileTreePrompt::Include);
        for ch in r"\.rs$".chars() {
            app.file_tree_prompt_insert_char(ch);
        }
        app.commit_file_tree_prompt();
        app.begin_file_tree_prompt(FileTreePrompt::Exclude);
        for ch in "^tests/".chars() {
            app.file_tree_prompt_insert_char(ch);
        }
        app.commit_file_tree_prompt();

        let text = buffer_text(&draw(&mut app));

        assert!(
            text.contains(r"i:\.rs$") && text.contains("e:^tests/"),
            "expected both applied patterns in the tree border, got:\n{text}"
        );
        // Title reports the filtered count against the unfiltered total.
        assert!(
            text.contains("1 of 3"),
            "expected a filtered/total count in the tree title, got:\n{text}"
        );
    }

    #[test]
    fn should_leave_the_border_clean_when_no_filter_is_set() {
        let mut app = app_with(&["src/main.rs"]);

        let text = buffer_text(&draw(&mut app));

        assert!(
            !text.contains("i:") && !text.contains("e:"),
            "unfiltered tree should not advertise filters, got:\n{text}"
        );
    }

    #[test]
    fn should_announce_hidden_reviewed_files_in_the_border() {
        let mut app = app_with(&["src/main.rs", "README.md"]);
        app.set_show_reviewed(false);

        let text = buffer_text(&draw(&mut app));

        assert!(
            text.contains("reviewed hidden"),
            "expected the reviewed-hidden cue in the tree border, got:\n{text}"
        );
    }

    #[test]
    fn should_keep_the_progress_fraction_in_the_title_while_hiding_reviewed_files() {
        let mut app = app_with(&["src/main.rs", "README.md"]);
        let reviewed = app
            .diff_files
            .iter()
            .position(|file| file.display_path().display().to_string() == "README.md")
            .expect("README.md in the diff");
        app.toggle_reviewed_for_file_idx(reviewed, false);

        app.set_show_reviewed(false);

        let text = buffer_text(&draw(&mut app));
        // Counting the hidden row as not-shown would render `0/1` here.
        assert!(
            text.contains("1/2"),
            "expected progress over the whole population, got:\n{text}"
        );
    }

    #[test]
    fn should_not_advertise_hiding_when_reviewed_files_are_shown() {
        let mut app = app_with(&["src/main.rs"]);

        let text = buffer_text(&draw(&mut app));

        assert!(
            !text.contains("reviewed hidden"),
            "default state should not advertise hiding, got:\n{text}"
        );
    }
}
