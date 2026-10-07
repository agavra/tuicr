use crate::app::*;
use crate::handler::handle_comment_action;
use crate::input::keybindings::map_key_to_action;
use crate::model::{DiffHunk, DiffLine, FileStatus, LineOrigin};
use crate::vcs::traits::VcsType;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{Terminal, backend::TestBackend, layout::Position};

struct StubVcs(VcsInfo);

impl VcsBackend for StubVcs {
    fn info(&self) -> &VcsInfo {
        &self.0
    }

    fn get_working_tree_diff(&self, _: &SyntaxHighlighter) -> Result<Vec<DiffFile>> {
        Ok(Vec::new())
    }

    fn fetch_context_lines(
        &self,
        _: &Path,
        _: FileStatus,
        _: Option<&str>,
        _: u32,
        _: u32,
    ) -> Result<Vec<DiffLine>> {
        Ok(Vec::new())
    }

    fn file_line_count(&self, _: &Path, _: FileStatus, _: Option<&str>) -> Result<u32> {
        Ok(1)
    }
}

struct Editor {
    app: App,
    terminal: Terminal<TestBackend>,
}

impl Editor {
    fn new(width: u16, height: u16) -> Self {
        let info = VcsInfo {
            root_path: PathBuf::from("/comment-navigation-test"),
            head_commit: "abc123".into(),
            branch_name: None,
            vcs_type: VcsType::Git,
        };
        let session = ReviewSession::new(
            info.root_path.clone(),
            info.head_commit.clone(),
            None,
            SessionDiffSource::WorkingTree,
        );
        let hunks = vec![DiffHunk {
            header: "@@ -0,0 +1 @@".into(),
            old_start: 0,
            old_count: 0,
            new_start: 1,
            new_count: 1,
            lines: vec![DiffLine {
                origin: LineOrigin::Addition,
                content: "code".into(),
                old_lineno: None,
                new_lineno: Some(1),
                highlighted_spans: None,
            }],
        }];
        let file = DiffFile {
            old_path: None,
            new_path: Some(PathBuf::from("example.rs")),
            status: FileStatus::Added,
            content_hash: DiffFile::compute_content_hash(&hunks),
            hunks,
            is_binary: false,
            is_too_large: false,
            is_commit_message: false,
        };
        let mut app = App::build(
            Box::new(StubVcs(info.clone())),
            info,
            Theme::dark(),
            None,
            false,
            vec![file],
            session,
            DiffSource::WorkingTree,
            InputMode::Normal,
            Vec::new(),
            None,
            None,
        )
        .unwrap();
        app.show_file_list = false;
        app.go_to_source_line(1, LineSide::New);
        app.enter_comment_mode(false, Some((1, LineSide::New)));
        let mut editor = Self {
            app,
            terminal: Terminal::new(TestBackend::new(width, height)).unwrap(),
        };
        editor.draw();
        editor
    }

    fn draw(&mut self) -> Position {
        self.terminal
            .draw(|frame| crate::ui::render(frame, &mut self.app))
            .unwrap();
        self.terminal.get_cursor_position().unwrap()
    }

    fn key(&mut self, code: KeyCode) -> Position {
        let modifiers = if code == KeyCode::Enter {
            KeyModifiers::ALT
        } else {
            KeyModifiers::NONE
        };
        let action = map_key_to_action(KeyEvent::new(code, modifiers), self.app.input_mode, ';');
        handle_comment_action(&mut self.app, action);
        self.draw()
    }

    fn type_text(&mut self, text: &str) -> Position {
        for ch in text.chars() {
            self.key(if ch == '\n' {
                KeyCode::Enter
            } else {
                KeyCode::Char(ch)
            });
        }
        self.draw()
    }
}

#[test]
fn wrapped_movement_uses_the_resized_viewport_in_both_diff_layouts() {
    for mode in [DiffViewMode::Unified, DiffViewMode::SideBySide] {
        let mut editor = Editor::new(80, 24);
        editor.app.diff_view_mode = mode;
        editor.type_text("abcdefghijklm");
        editor.terminal.backend_mut().resize(20, 24);
        let before = editor.draw();
        assert_eq!(
            editor.key(KeyCode::Up),
            Position::new(before.x, before.y - 1)
        );
        editor.key(KeyCode::Char('!'));
        assert_eq!(editor.app.comment_buffer, "abc!defghijklm");
    }
}

#[test]
fn vertical_movement_keeps_the_cursor_visible_in_a_tall_comment() {
    let text = ('A'..='X')
        .map(|ch| ch.to_string().repeat(3))
        .collect::<Vec<_>>()
        .join("\n");
    for mode in [DiffViewMode::Unified, DiffViewMode::SideBySide] {
        let mut editor = Editor::new(80, 12);
        editor.app.diff_view_mode = mode;
        editor.type_text(&text);
        editor.key(KeyCode::Left);
        for ch in ('A'..='W').rev() {
            let cursor = editor.key(KeyCode::Up);
            assert_eq!(
                editor.terminal.backend().buffer()[cursor].symbol(),
                ch.to_string(),
                "{mode:?}"
            );
        }
        for ch in 'B'..='X' {
            let cursor = editor.key(KeyCode::Down);
            assert_eq!(
                editor.terminal.backend().buffer()[cursor].symbol(),
                ch.to_string(),
                "{mode:?}"
            );
        }
        assert_eq!(editor.app.comment_buffer, text);
    }
}

#[test]
fn vertical_movement_stops_at_the_first_and_last_rows() {
    for text in ["", "abc", "abc\ndef"] {
        let mut editor = Editor::new(80, 24);
        let end = editor.type_text(text);
        assert_eq!(editor.key(KeyCode::Down), end);
        let top = editor.key(KeyCode::Up);
        assert_eq!(editor.key(KeyCode::Up), top);
        assert_eq!(editor.app.comment_buffer, text);
    }
}

#[test]
fn reopening_a_comment_starts_with_its_own_cursor_column() {
    let mut editor = Editor::new(80, 24);
    editor.type_text("abcdefghij\nx\nabcdefghij");
    editor.key(KeyCode::Up);
    editor.key(KeyCode::Esc);
    editor
        .app
        .session
        .get_file_mut(&PathBuf::from("example.rs"))
        .unwrap()
        .add_line_comment(
            1,
            crate::model::Comment::new(
                "abcdefghij\nabc".into(),
                crate::model::CommentType::from_id("note"),
                None,
            ),
        );
    editor.app.rebuild_annotations();
    let row = editor
        .app
        .line_annotations
        .iter()
        .enumerate()
        .filter(|(_, row)| matches!(row, AnnotatedLine::LineComment { .. }))
        .nth(2)
        .unwrap()
        .0;
    editor.app.move_cursor_to_annotation(row);
    assert!(editor.app.enter_edit_mode(true));
    let before = editor.draw();
    assert_eq!(
        editor.key(KeyCode::Up),
        Position::new(before.x, before.y - 1)
    );
    editor.key(KeyCode::Char('!'));
    assert_eq!(editor.app.comment_buffer, "abc!defghij\nabc");
}

#[test]
fn moving_to_a_full_wrapped_row_stays_on_that_row() {
    let mut editor = Editor::new(20, 24);
    let end = editor.type_text("abcdefghijklmnopqrst");
    assert_eq!(editor.key(KeyCode::Up), Position::new(end.x - 1, end.y - 1));
    assert_eq!(editor.key(KeyCode::Down), end);
    editor.key(KeyCode::Char('!'));
    assert_eq!(editor.app.comment_buffer, "abcdefghijklmnopqrst!");
}

#[test]
fn vertical_movement_uses_screen_columns_for_unicode() {
    for (text, expected) in [
        ("ab界cd\n1234", "ab界!cd\n1234"),
        ("abe\u{301}cd\n1234", "abe\u{301}c!d\n1234"),
        ("ab🦀cd\n1234", "ab🦀!cd\n1234"),
    ] {
        let mut editor = Editor::new(80, 24);
        let before = editor.type_text(text);
        assert_eq!(
            editor.key(KeyCode::Up),
            Position::new(before.x, before.y - 1)
        );
        editor.key(KeyCode::Char('!'));
        assert_eq!(editor.app.comment_buffer, expected);
    }
}

#[test]
fn up_preserves_screen_column_after_a_text_presentation_selector() {
    let mut editor = Editor::new(80, 24);
    let before = editor.type_text("\u{2648}\u{fe0e}x\nx");
    let after = editor.key(KeyCode::Up);
    assert_eq!(after, Position::new(before.x, before.y - 1));
    assert_eq!(editor.terminal.backend().buffer()[after].symbol(), "x");
    editor.key(KeyCode::Char('!'));
    assert_eq!(editor.app.comment_buffer, "\u{2648}\u{fe0e}!x\nx");
}

#[test]
fn horizontal_movement_and_edits_reset_the_preferred_column() {
    for key in [
        KeyCode::Left,
        KeyCode::Right,
        KeyCode::Home,
        KeyCode::End,
        KeyCode::Backspace,
        KeyCode::Char('!'),
    ] {
        let mut editor = Editor::new(80, 24);
        editor.type_text("abcdefghij\nx\nabcdefghij");
        editor.key(KeyCode::Up);
        let before = editor.key(key);
        assert_eq!(
            editor.key(KeyCode::Up),
            Position::new(before.x, before.y - 1),
            "{key:?}"
        );
    }
}

#[test]
fn vertical_movement_preserves_the_column_across_short_and_empty_rows() {
    for middle in ["x", ""] {
        let mut editor = Editor::new(80, 24);
        let end = editor.type_text(&format!("abcdefghij\n{middle}\nabcdefghij"));
        editor.key(KeyCode::Up);
        assert_eq!(editor.key(KeyCode::Up), Position::new(end.x, end.y - 2));
        editor.key(KeyCode::Down);
        assert_eq!(editor.key(KeyCode::Down), end);
    }
}

#[test]
fn down_returns_to_the_following_displayed_row() {
    for text in ["abc\ndef", "abcdefghijklm"] {
        let mut editor = Editor::new(20, 24);
        let end = editor.type_text(text);
        editor.key(KeyCode::Up);
        assert_eq!(editor.key(KeyCode::Down), end);
        editor.key(KeyCode::Char('!'));
        assert_eq!(editor.app.comment_buffer, format!("{text}!"));
    }
}

#[test]
fn up_moves_within_a_wrapped_line() {
    let mut editor = Editor::new(20, 24);
    let before = editor.type_text("abcdefghijklm");
    let after = editor.key(KeyCode::Up);
    assert_eq!(after, Position::new(before.x, before.y - 1));
    editor.key(KeyCode::Char('!'));
    assert_eq!(editor.app.comment_buffer, "abc!defghijklm");
}

#[test]
fn up_moves_to_the_preceding_newline_separated_row() {
    let mut editor = Editor::new(80, 24);
    let before = editor.type_text("abc\ndef");
    let after = editor.key(KeyCode::Up);
    assert_eq!(after, Position::new(before.x, before.y - 1));
    editor.key(KeyCode::Char('!'));
    assert_eq!(editor.app.comment_buffer, "abc!\ndef");
}
