//! UTF-8 aware text editing utilities.
//!
//! Provides cursor movement and text manipulation functions that correctly
//! handle multi-byte UTF-8 characters (CJK, emoji, etc.).

/// Find the byte position of the previous character boundary.
/// Returns 0 if already at the start.
pub fn prev_char_boundary(buffer: &str, cursor: usize) -> usize {
    if cursor == 0 {
        return 0;
    }
    let cursor = cursor.min(buffer.len());
    // Move back one byte at a time until we find a char boundary
    let mut pos = cursor - 1;
    while pos > 0 && !buffer.is_char_boundary(pos) {
        pos -= 1;
    }
    pos
}

/// Find the byte position of the next character boundary.
/// Returns buffer.len() if already at the end.
pub fn next_char_boundary(buffer: &str, cursor: usize) -> usize {
    if cursor >= buffer.len() {
        return buffer.len();
    }
    // Move forward one byte at a time until we find a char boundary
    let mut pos = cursor + 1;
    while pos < buffer.len() && !buffer.is_char_boundary(pos) {
        pos += 1;
    }
    pos
}

/// Delete the character before the cursor position, returning the new cursor position.
/// Handles multi-byte UTF-8 characters correctly.
pub fn delete_char_before(buffer: &mut String, cursor: usize) -> usize {
    if cursor == 0 {
        return 0;
    }
    let prev = prev_char_boundary(buffer, cursor);
    buffer.replace_range(prev..cursor, "");
    prev
}

/// Delete word before cursor, returning the new cursor position.
/// Handles multi-byte UTF-8 characters correctly.
pub fn delete_word_before(buffer: &mut String, cursor: usize) -> usize {
    if cursor == 0 {
        return 0;
    }

    let mut pos = cursor;

    // Skip whitespace backwards
    while pos > 0 {
        let prev = prev_char_boundary(buffer, pos);
        if let Some(ch) = buffer[prev..pos].chars().next()
            && !ch.is_whitespace()
        {
            break;
        }
        pos = prev;
    }

    // Skip non-whitespace backwards (the word itself)
    while pos > 0 {
        let prev = prev_char_boundary(buffer, pos);
        if let Some(ch) = buffer[prev..pos].chars().next()
            && ch.is_whitespace()
        {
            break;
        }
        pos = prev;
    }

    // Delete from pos to cursor
    buffer.replace_range(pos..cursor, "");
    pos
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- prev_char_boundary tests --

    #[test]
    fn should_return_zero_when_at_start() {
        // given
        let s = "hello";

        // when
        let result = prev_char_boundary(s, 0);

        // then
        assert_eq!(result, 0);
    }

    #[test]
    fn should_find_prev_boundary_for_ascii() {
        // given
        let s = "hello";

        // when/then
        assert_eq!(prev_char_boundary(s, 5), 4);
        assert_eq!(prev_char_boundary(s, 3), 2);
        assert_eq!(prev_char_boundary(s, 1), 0);
    }

    #[test]
    fn should_find_prev_boundary_for_multibyte_char() {
        // given - '좋' is 3 bytes, '아' is 3 bytes
        let s = "좋아";
        assert_eq!(s.len(), 6);

        // when/then
        assert_eq!(prev_char_boundary(s, 6), 3); // End -> start of '아'
        assert_eq!(prev_char_boundary(s, 3), 0); // Start of '아' -> start of '좋'
    }

    #[test]
    fn should_find_prev_boundary_for_emoji() {
        // given - '🦀' is 4 bytes
        let s = "🦀";
        assert_eq!(s.len(), 4);

        // when/then
        assert_eq!(prev_char_boundary(s, 4), 0);
    }

    #[test]
    fn should_find_prev_boundary_for_mixed_content() {
        // given
        let s = "a좋b"; // 1 + 3 + 1 = 5 bytes

        // when/then
        assert_eq!(prev_char_boundary(s, 5), 4); // After 'b' -> start of 'b'
        assert_eq!(prev_char_boundary(s, 4), 1); // After '좋' -> start of '좋'
        assert_eq!(prev_char_boundary(s, 1), 0); // After 'a' -> start
    }

    // -- next_char_boundary tests --

    #[test]
    fn should_return_len_when_at_end() {
        // given
        let s = "hello";

        // when
        let result = next_char_boundary(s, 5);

        // then
        assert_eq!(result, 5);
    }

    #[test]
    fn should_find_next_boundary_for_ascii() {
        // given
        let s = "hello";

        // when/then
        assert_eq!(next_char_boundary(s, 0), 1);
        assert_eq!(next_char_boundary(s, 2), 3);
        assert_eq!(next_char_boundary(s, 4), 5);
    }

    #[test]
    fn should_find_next_boundary_for_multibyte_char() {
        // given
        let s = "좋아";

        // when/then
        assert_eq!(next_char_boundary(s, 0), 3); // Start -> after '좋'
        assert_eq!(next_char_boundary(s, 3), 6); // After '좋' -> after '아'
    }

    #[test]
    fn should_find_next_boundary_for_emoji() {
        // given
        let s = "🦀";

        // when/then
        assert_eq!(next_char_boundary(s, 0), 4);
    }

    // -- delete_char_before tests --

    #[test]
    fn should_delete_ascii_char() {
        // given
        let mut s = String::from("hello");

        // when
        let cursor = delete_char_before(&mut s, 5);

        // then
        assert_eq!(s, "hell");
        assert_eq!(cursor, 4);
    }

    #[test]
    fn should_delete_multibyte_char() {
        // given
        let mut s = String::from("좋아");

        // when
        let cursor = delete_char_before(&mut s, 6);

        // then
        assert_eq!(s, "좋");
        assert_eq!(cursor, 3);
    }

    #[test]
    fn should_delete_multibyte_char_from_middle() {
        // given
        let mut s = String::from("좋아요");

        // when
        let cursor = delete_char_before(&mut s, 6); // After '좋아'

        // then
        assert_eq!(s, "좋요");
        assert_eq!(cursor, 3);
    }

    #[test]
    fn should_not_delete_when_at_start() {
        // given
        let mut s = String::from("hello");

        // when
        let cursor = delete_char_before(&mut s, 0);

        // then
        assert_eq!(s, "hello");
        assert_eq!(cursor, 0);
    }

    #[test]
    fn should_delete_emoji() {
        // given
        let mut s = String::from("hi🦀");

        // when
        let cursor = delete_char_before(&mut s, 6); // After emoji

        // then
        assert_eq!(s, "hi");
        assert_eq!(cursor, 2);
    }

    // -- delete_word_before tests --

    #[test]
    fn should_delete_ascii_word() {
        // given
        let mut s = String::from("hello world");

        // when
        let cursor = delete_word_before(&mut s, 11);

        // then
        assert_eq!(s, "hello ");
        assert_eq!(cursor, 6);
    }

    #[test]
    fn should_delete_multibyte_word() {
        // given
        let mut s = String::from("안녕 아가브라"); // 6 + 1 + 12 = 19 bytes

        // when
        let cursor = delete_word_before(&mut s, 19);

        // then
        assert_eq!(s, "안녕 ");
        assert_eq!(cursor, 7);
    }

    #[test]
    fn should_skip_trailing_whitespace_when_deleting_word() {
        // given
        let mut s = String::from("hello   ");

        // when
        let cursor = delete_word_before(&mut s, 8);

        // then
        assert_eq!(s, "");
        assert_eq!(cursor, 0);
    }

    #[test]
    fn should_not_delete_word_when_at_start() {
        // given
        let mut s = String::from("hello");

        // when
        let cursor = delete_word_before(&mut s, 0);

        // then
        assert_eq!(s, "hello");
        assert_eq!(cursor, 0);
    }

    // -- Integration tests --

    #[test]
    fn should_navigate_multibyte_string_correctly() {
        // given
        let s = "좋아요"; // 9 bytes, 3 chars

        // when - navigate right through all chars
        let mut cursor = 0;
        cursor = next_char_boundary(s, cursor);
        assert_eq!(cursor, 3);
        cursor = next_char_boundary(s, cursor);
        assert_eq!(cursor, 6);
        cursor = next_char_boundary(s, cursor);
        assert_eq!(cursor, 9);

        // when - navigate left through all chars
        cursor = prev_char_boundary(s, cursor);
        assert_eq!(cursor, 6);
        cursor = prev_char_boundary(s, cursor);
        assert_eq!(cursor, 3);
        cursor = prev_char_boundary(s, cursor);
        assert_eq!(cursor, 0);
    }

    #[test]
    fn should_handle_insert_delete_roundtrip() {
        // given
        let mut s = String::new();
        let mut cursor = 0;

        // when - insert multibyte chars
        for c in "좋아".chars() {
            s.insert(cursor, c);
            cursor += c.len_utf8();
        }

        // then
        assert_eq!(s, "좋아");
        assert_eq!(cursor, 6);

        // when - delete both chars
        cursor = delete_char_before(&mut s, cursor);
        assert_eq!(s, "좋");

        cursor = delete_char_before(&mut s, cursor);
        assert_eq!(s, "");
        assert_eq!(cursor, 0);
    }
}
