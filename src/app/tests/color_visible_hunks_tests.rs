//! Table-driven coverage for `App::visible_hunk_indices`, whose own doc comment says
//! why that window arithmetic is a separate pure function.
//! `ui::app_layout::visible_coloring_tests` covers the wiring: that the render call
//! site passes the right height, and that a colored hunk reaches the screen.

use crate::app::AnnotatedLine;

/// One `HunkHeader` per index, so the expected result for any window `[start, end)`
/// is just that range of `(index, 0)` pairs.
fn one_hunk_per_annotation(count: usize) -> Vec<AnnotatedLine> {
    (0..count)
        .map(|i| AnnotatedLine::HunkHeader {
            file_idx: i,
            hunk_idx: 0,
        })
        .collect()
}

struct Case {
    name: &'static str,
    annotation_count: usize,
    scroll_offset: usize,
    max_rows: usize,
    /// Hand-derived, not computed by calling the code under test: a table that
    /// recomputed its own expectation with the same formula would pass no matter
    /// what that formula did.
    expected: std::ops::Range<usize>,
}

#[test]
fn should_compute_the_visible_window_for_each_case() {
    let cases = [
        Case {
            name: "offset zero",
            annotation_count: 10,
            scroll_offset: 0,
            max_rows: 3,
            expected: 0..6,
        },
        Case {
            name: "offset deep enough in a long diff for the backward margin to be live",
            annotation_count: 100,
            scroll_offset: 50,
            max_rows: 10,
            expected: 40..70,
        },
        Case {
            name: "offset past the last annotation clamps to the bottom",
            annotation_count: 20,
            scroll_offset: 25,
            max_rows: 5,
            expected: 15..20,
        },
        Case {
            name: "annotation list shorter than one screen",
            annotation_count: 3,
            scroll_offset: 0,
            max_rows: 10,
            expected: 0..3,
        },
        Case {
            name: "empty annotation list",
            annotation_count: 0,
            scroll_offset: 0,
            max_rows: 10,
            expected: 0..0,
        },
        Case {
            name: "zero row count still covers one row's worth either side",
            annotation_count: 10,
            scroll_offset: 5,
            max_rows: 0,
            expected: 4..7,
        },
    ];

    for case in cases {
        let mut app = crate::ui::row_height::tests::make_app();
        app.line_annotations = one_hunk_per_annotation(case.annotation_count);
        app.diff_state.scroll_offset = case.scroll_offset;

        let expected: Vec<(usize, usize)> = case.expected.map(|i| (i, 0)).collect();
        assert_eq!(
            app.visible_hunk_indices(case.max_rows),
            expected,
            "case: {}",
            case.name
        );
    }
}
