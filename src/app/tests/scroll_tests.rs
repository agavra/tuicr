/// max_scroll_offset is simply total_lines - 1 (last line can be at top).
fn calc_max_scroll(total_lines: usize) -> usize {
    total_lines.saturating_sub(1)
}

#[test]
fn should_calculate_max_scroll() {
    // Last line can be scrolled to the top of the viewport
    assert_eq!(calc_max_scroll(103), 102);
    assert_eq!(calc_max_scroll(20), 19);
}

#[test]
fn should_handle_small_content() {
    // Even with few lines, can scroll last line to top
    assert_eq!(calc_max_scroll(13), 12);
    assert_eq!(calc_max_scroll(1), 0);
}

#[test]
fn should_handle_empty_content() {
    assert_eq!(calc_max_scroll(0), 0);
}
