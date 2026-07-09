use crate::app::*;

fn make_diff_line(file_idx: usize, new_lineno: Option<u32>) -> AnnotatedLine {
    AnnotatedLine::DiffLine {
        file_idx,
        hunk_idx: 0,
        line_idx: 0,
        old_lineno: None,
        new_lineno,
    }
}

fn make_diff_line_with_old(
    file_idx: usize,
    old_lineno: Option<u32>,
    new_lineno: Option<u32>,
) -> AnnotatedLine {
    AnnotatedLine::DiffLine {
        file_idx,
        hunk_idx: 0,
        line_idx: 0,
        old_lineno,
        new_lineno,
    }
}

fn make_sbs_line(file_idx: usize, new_lineno: Option<u32>) -> AnnotatedLine {
    AnnotatedLine::SideBySideLine {
        file_idx,
        hunk_idx: 0,
        del_line_idx: None,
        add_line_idx: None,
        old_lineno: None,
        new_lineno,
    }
}

#[test]
fn should_find_exact_match() {
    let annotations = vec![
        AnnotatedLine::FileHeader { file_idx: 0 },
        make_diff_line(0, Some(10)),
        make_diff_line(0, Some(11)),
        make_diff_line(0, Some(12)),
    ];

    let result = find_source_line(&annotations, 0, 11, LineSide::New);
    assert_eq!(result, FindSourceLineResult::Exact(2));
}

#[test]
fn should_find_nearest_when_no_exact_match() {
    let annotations = vec![
        make_diff_line(0, Some(10)),
        make_diff_line(0, Some(15)),
        make_diff_line(0, Some(20)),
    ];

    // Target 12 is closest to line 10 (dist=2) vs 15 (dist=3) vs 20 (dist=8)
    let result = find_source_line(&annotations, 0, 12, LineSide::New);
    assert_eq!(result, FindSourceLineResult::Nearest(0));
}

#[test]
fn should_find_nearest_above_target() {
    let annotations = vec![
        make_diff_line(0, Some(10)),
        make_diff_line(0, Some(15)),
        make_diff_line(0, Some(20)),
    ];

    // Target 18 is closest to line 20 (dist=2) vs 15 (dist=3) vs 10 (dist=8)
    let result = find_source_line(&annotations, 0, 18, LineSide::New);
    assert_eq!(result, FindSourceLineResult::Nearest(2));
}

#[test]
fn should_return_not_found_for_empty_annotations() {
    let annotations: Vec<AnnotatedLine> = vec![];
    let result = find_source_line(&annotations, 0, 42, LineSide::New);
    assert_eq!(result, FindSourceLineResult::NotFound);
}

#[test]
fn should_return_not_found_when_no_lines_in_current_file() {
    let annotations = vec![make_diff_line(1, Some(10)), make_diff_line(1, Some(20))];

    // File 0 has no lines
    let result = find_source_line(&annotations, 0, 10, LineSide::New);
    assert_eq!(result, FindSourceLineResult::NotFound);
}

#[test]
fn should_skip_lines_from_other_files() {
    let annotations = vec![
        make_diff_line(0, Some(100)), // file 0, line 100
        make_diff_line(1, Some(42)),  // file 1, exact match but wrong file
        make_diff_line(0, Some(50)),  // file 0, line 50
    ];

    // Searching file 0 for line 42 — should find nearest (50, dist=8) not file 1's exact match
    let result = find_source_line(&annotations, 0, 42, LineSide::New);
    assert_eq!(result, FindSourceLineResult::Nearest(2));
}

#[test]
fn should_skip_non_diff_line_annotations() {
    let annotations = vec![
        AnnotatedLine::FileHeader { file_idx: 0 },
        AnnotatedLine::HunkHeader {
            file_idx: 0,
            hunk_idx: 0,
        },
        AnnotatedLine::Spacing,
        make_diff_line(0, Some(42)),
    ];

    let result = find_source_line(&annotations, 0, 42, LineSide::New);
    assert_eq!(result, FindSourceLineResult::Exact(3));
}

#[test]
fn should_skip_diff_lines_with_no_new_lineno() {
    // Deletion-only lines have new_lineno = None
    let annotations = vec![make_diff_line(0, None), make_diff_line(0, Some(20))];

    let result = find_source_line(&annotations, 0, 5, LineSide::New);
    assert_eq!(result, FindSourceLineResult::Nearest(1));
}

#[test]
fn should_work_with_side_by_side_lines() {
    let annotations = vec![
        make_sbs_line(0, Some(10)),
        make_sbs_line(0, Some(20)),
        make_sbs_line(0, Some(30)),
    ];

    let result = find_source_line(&annotations, 0, 20, LineSide::New);
    assert_eq!(result, FindSourceLineResult::Exact(1));
}

#[test]
fn should_handle_mixed_diff_and_sbs_lines() {
    let annotations = vec![
        make_diff_line(0, Some(10)),
        make_sbs_line(0, Some(20)),
        make_diff_line(0, Some(30)),
    ];

    let result = find_source_line(&annotations, 0, 25, LineSide::New);
    // Nearest is line 20 (dist=5) or line 30 (dist=5), first match wins
    assert_eq!(result, FindSourceLineResult::Nearest(1));
}

#[test]
fn should_return_not_found_when_only_non_line_annotations() {
    let annotations = vec![
        AnnotatedLine::FileHeader { file_idx: 0 },
        AnnotatedLine::Spacing,
        AnnotatedLine::HunkHeader {
            file_idx: 0,
            hunk_idx: 0,
        },
    ];

    let result = find_source_line(&annotations, 0, 42, LineSide::New);
    assert_eq!(result, FindSourceLineResult::NotFound);
}

#[test]
fn should_prefer_exact_match_over_earlier_nearest() {
    let annotations = vec![
        make_diff_line(0, Some(41)), // dist=1 from target 42
        make_diff_line(0, Some(42)), // exact match
        make_diff_line(0, Some(43)), // dist=1 from target 42
    ];

    let result = find_source_line(&annotations, 0, 42, LineSide::New);
    assert_eq!(result, FindSourceLineResult::Exact(1));
}

#[test]
fn should_find_nearest_for_target_zero() {
    // target_lineno = 0 is out-of-range (lines are 1-indexed) but should
    // still return the nearest line rather than panicking.
    let annotations = vec![make_diff_line(0, Some(1)), make_diff_line(0, Some(5))];

    let result = find_source_line(&annotations, 0, 0, LineSide::New);
    assert_eq!(result, FindSourceLineResult::Nearest(0));
}

#[test]
fn should_tie_break_nearest_by_iteration_order() {
    // When two lines are equidistant, the first one encountered wins.
    // Here lines are in descending order; line 30 (idx 0) and line 10 (idx 2)
    // are both dist=10 from target 20, so idx 0 should win.
    let annotations = vec![
        make_diff_line(0, Some(30)),
        make_diff_line(0, Some(50)),
        make_diff_line(0, Some(10)),
    ];

    let result = find_source_line(&annotations, 0, 20, LineSide::New);
    assert_eq!(result, FindSourceLineResult::Nearest(0));
}

#[test]
fn should_match_old_lineno_when_side_is_old() {
    // Deletion-only lines carry old_lineno but no new_lineno. `:o<n>`
    // must match those.
    let annotations = vec![
        make_diff_line_with_old(0, Some(5), None),
        make_diff_line_with_old(0, Some(10), None),
        make_diff_line(0, Some(50)), // new-side line — should be ignored when side=Old
    ];

    let exact = find_source_line(&annotations, 0, 10, LineSide::Old);
    assert_eq!(exact, FindSourceLineResult::Exact(1));

    let nearest = find_source_line(&annotations, 0, 7, LineSide::Old);
    assert_eq!(nearest, FindSourceLineResult::Nearest(0));
}

#[test]
fn should_not_match_new_lineno_when_side_is_old() {
    // A pure-addition line has no old_lineno; searching old-side should
    // not fall back to its new_lineno.
    let annotations = vec![make_diff_line(0, Some(42))];

    let result = find_source_line(&annotations, 0, 42, LineSide::Old);
    assert_eq!(result, FindSourceLineResult::NotFound);
}
