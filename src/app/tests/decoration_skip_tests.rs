use crate::app::*;

fn diff_line(file_idx: usize, new_lineno: u32) -> AnnotatedLine {
    AnnotatedLine::DiffLine {
        file_idx,
        hunk_idx: 0,
        line_idx: 0,
        old_lineno: None,
        new_lineno: Some(new_lineno),
    }
}

/// Two files: header, content, spacing, header, content.
fn fixture() -> Vec<AnnotatedLine> {
    vec![
        AnnotatedLine::FileHeader { file_idx: 0 }, // 0
        diff_line(0, 1),                           // 1
        AnnotatedLine::Spacing,                    // 2
        AnnotatedLine::FileHeader { file_idx: 1 }, // 3
        diff_line(1, 1),                           // 4
    ]
}

#[test]
fn forward_skips_spacing_and_header_to_next_content_line() {
    let annotations = fixture();
    assert_eq!(skip_decoration_forward(&annotations, 2, 4), 4);
}

#[test]
fn forward_keeps_position_on_content_line() {
    let annotations = fixture();
    assert_eq!(skip_decoration_forward(&annotations, 1, 4), 1);
}

#[test]
fn forward_clamps_at_max_line_even_on_decoration() {
    let annotations = vec![
        diff_line(0, 1),                           // 0
        AnnotatedLine::Spacing,                    // 1
        AnnotatedLine::FileHeader { file_idx: 1 }, // 2
    ];
    assert_eq!(skip_decoration_forward(&annotations, 1, 2), 2);
}

#[test]
fn backward_skips_header_and_spacing_to_previous_content_line() {
    let annotations = fixture();
    assert_eq!(skip_decoration_backward(&annotations, 3), 1);
}

#[test]
fn backward_stops_at_zero_when_top_is_decoration() {
    let annotations = fixture();
    assert_eq!(skip_decoration_backward(&annotations, 0), 0);
}
