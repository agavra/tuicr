//! Vim-style modal editing for the comment box, backed by `edtui`.
//!
//! An overlay: `App::comment_buffer`/`comment_cursor` stay canonical (rendering
//! reads them); the editor syncs back into them after each event. Cursor
//! conversions bridge tuicr's UTF-8 byte offset and edtui's `Index2` (row +
//! char-col), both UTF-8 aware.

use crossterm::event::{Event, KeyCode, KeyEvent};
use edtui::{EditorEventHandler, EditorMode, EditorState, Index2, Lines};

/// Key codes edtui can convert; anything else makes it panic.
fn edtui_handles(code: KeyCode) -> bool {
    matches!(
        code,
        KeyCode::Char(_)
            | KeyCode::Enter
            | KeyCode::Esc
            | KeyCode::Backspace
            | KeyCode::Delete
            | KeyCode::Tab
            | KeyCode::Left
            | KeyCode::Right
            | KeyCode::Up
            | KeyCode::Down
            | KeyCode::Home
            | KeyCode::End
            | KeyCode::PageUp
            | KeyCode::PageDown
    )
}

/// Active modal editor for the comment box. Present only while in comment mode
/// with `comment_vim` enabled.
pub struct CommentVimEditor {
    state: EditorState,
    events: EditorEventHandler,
}

impl CommentVimEditor {
    /// Build an editor seeded from the buffer text + byte cursor, starting in
    /// Insert mode (typing works immediately; `Esc` drops to Normal).
    pub fn from_buffer(text: &str, cursor_byte: usize) -> Self {
        let mut state = EditorState::new(Lines::from(text));
        state.mode = EditorMode::Insert;
        state.cursor = byte_to_index2(text, cursor_byte);
        Self {
            state,
            events: EditorEventHandler::default(),
        }
    }

    /// True when the editor is in Normal mode. Used to decide whether `Esc`
    /// cancels the comment and whether `Enter` submits.
    pub fn is_normal_mode(&self) -> bool {
        self.state.mode == EditorMode::Normal
    }

    /// Short status label for the current mode. The catch-all tolerates any
    /// extra edtui modes (e.g. search).
    pub fn label(&self) -> &'static str {
        match self.state.mode {
            EditorMode::Normal => "NORMAL",
            EditorMode::Insert => "INSERT",
            EditorMode::Visual => "VISUAL",
            _ => "NORMAL",
        }
    }

    /// Feed a key event to the editor and return the resulting
    /// `(text, byte_cursor)` for the caller to sync into the canonical buffer.
    pub fn feed_key(&mut self, key: KeyEvent) -> (String, usize) {
        // edtui panics on keys it cannot map (BackTab, F-keys, Insert, ...), so
        // only forward the ones it understands. Normal-mode comment type
        // cycling is handled by the caller.
        if edtui_handles(key.code) {
            self.events.on_key_event(key, &mut self.state);
        }
        self.text_and_cursor()
    }

    /// Feed a bracketed-paste payload, returning the synced `(text, byte_cursor)`.
    pub fn feed_paste(&mut self, text: String) -> (String, usize) {
        self.events.on_event(Event::Paste(text), &mut self.state);
        self.text_and_cursor()
    }

    /// Extract the full buffer text and the byte-offset cursor.
    pub fn text_and_cursor(&self) -> (String, usize) {
        let text = self.state.lines.to_string();
        let cursor = index2_to_byte(&text, self.state.cursor);
        (text, cursor)
    }
}

/// Convert an edtui `(row, col)` index into a UTF-8 byte offset into `text`.
/// `col` counts characters within the row; values past the end of a line clamp
/// to the line end, and rows past the end clamp to the buffer end.
pub fn index2_to_byte(text: &str, idx: Index2) -> usize {
    let mut offset = 0usize;
    for (row, line) in text.split('\n').enumerate() {
        if row == idx.row {
            // Walk `col` characters into this line.
            for (chars, (byte, _)) in line.char_indices().enumerate() {
                if chars == idx.col {
                    return offset + byte;
                }
            }
            // col at or beyond the last char: clamp to end of line.
            return offset + line.len();
        }
        offset += line.len() + 1; // + 1 for the '\n' separator
    }
    text.len()
}

/// Convert a UTF-8 byte offset into an edtui `(row, col)` index, where `col` is
/// a character index within the row.
pub fn byte_to_index2(text: &str, byte: usize) -> Index2 {
    let byte = byte.min(text.len());
    let mut row = 0usize;
    let mut line_start = 0usize;
    for (i, ch) in text.char_indices() {
        if i >= byte {
            break;
        }
        if ch == '\n' {
            row += 1;
            line_start = i + 1;
        }
    }
    let col = text[line_start..byte].chars().count();
    Index2::new(row, col)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn key(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
    }

    fn special(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn insert_mode_typing_syncs_buffer() {
        let mut editor = CommentVimEditor::from_buffer("", 0);
        assert!(!editor.is_normal_mode());
        let mut last = (String::new(), 0);
        for c in "hi".chars() {
            last = editor.feed_key(key(c));
        }
        assert_eq!(last, ("hi".to_string(), 2));
        assert_eq!(editor.label(), "INSERT");
    }

    #[test]
    fn insert_mode_backtab_is_ignored() {
        let mut editor = CommentVimEditor::from_buffer("hello", 5);
        let result = editor.feed_key(KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT));
        assert_eq!(result, ("hello".to_string(), 5));
        assert_eq!(editor.label(), "INSERT");
        assert_eq!(editor.feed_key(key('!')), ("hello!".to_string(), 6));
    }

    #[test]
    fn unmapped_keys_are_ignored_in_both_modes() {
        let mut editor = CommentVimEditor::from_buffer("hello", 5);
        for code in [KeyCode::F(1), KeyCode::Insert] {
            let r = editor.feed_key(KeyEvent::new(code, KeyModifiers::NONE));
            assert_eq!(r, ("hello".to_string(), 5));
        }
        editor.feed_key(special(KeyCode::Esc));
        assert_eq!(editor.label(), "NORMAL");
        let r = editor.feed_key(KeyEvent::new(KeyCode::F(1), KeyModifiers::NONE));
        assert_eq!(r.0, "hello");
    }

    #[test]
    fn esc_switches_to_normal_mode() {
        let mut editor = CommentVimEditor::from_buffer("hello", 5);
        editor.feed_key(special(KeyCode::Esc));
        assert!(editor.is_normal_mode());
        assert_eq!(editor.label(), "NORMAL");
    }

    #[test]
    fn normal_mode_x_deletes_char_under_cursor() {
        // Seed "abc" with cursor at start, drop to normal, delete first char.
        let mut editor = CommentVimEditor::from_buffer("abc", 0);
        editor.feed_key(special(KeyCode::Esc));
        let (text, _) = editor.feed_key(key('x'));
        assert_eq!(text, "bc");
    }

    #[test]
    fn normal_mode_dd_deletes_line() {
        let mut editor = CommentVimEditor::from_buffer("line1\nline2", 0);
        editor.feed_key(special(KeyCode::Esc));
        editor.feed_key(key('d'));
        let (text, _) = editor.feed_key(key('d'));
        assert_eq!(text, "line2");
    }

    fn roundtrip(text: &str, byte: usize) {
        let idx = byte_to_index2(text, byte);
        assert_eq!(
            index2_to_byte(text, idx),
            byte,
            "roundtrip failed for {text:?} @ {byte}"
        );
    }

    #[test]
    fn roundtrip_ascii_multiline() {
        let text = "hello\nworld\nfoo";
        for byte in 0..=text.len() {
            if text.is_char_boundary(byte) {
                roundtrip(text, byte);
            }
        }
    }

    #[test]
    fn roundtrip_multibyte() {
        // CJK + emoji across lines.
        let text = "héllo\n世界\n👋🏽 bye";
        for byte in 0..=text.len() {
            if text.is_char_boundary(byte) {
                roundtrip(text, byte);
            }
        }
    }

    #[test]
    fn index2_clamps_past_line_and_buffer() {
        let text = "ab\ncd";
        // col past end of row 0 clamps to end of "ab" (byte 2).
        assert_eq!(index2_to_byte(text, Index2::new(0, 99)), 2);
        // row past end clamps to buffer end.
        assert_eq!(index2_to_byte(text, Index2::new(99, 0)), text.len());
    }

    #[test]
    fn byte_to_index2_on_second_line() {
        let text = "ab\ncde";
        // byte 4 = 'd' on row 1, col 1.
        let idx = byte_to_index2(text, 4);
        assert_eq!((idx.row, idx.col), (1, 1));
    }
}
