use crate::app::*;

fn p(idx: usize, off: usize) -> SelPoint {
    SelPoint {
        annotation_idx: idx,
        char_offset: off,
        side: LineSide::New,
    }
}

#[test]
fn collapsed_starts_at_point() {
    let sel = VisualSelection::collapsed(p(5, 3));
    assert_eq!(sel.anchor, p(5, 3));
    assert_eq!(sel.head, p(5, 3));
}

#[test]
fn ordered_returns_anchor_head_when_already_in_order() {
    let sel = VisualSelection {
        anchor: p(1, 0),
        head: p(4, 8),
    };
    let (start, end) = sel.ordered();
    assert_eq!(start, p(1, 0));
    assert_eq!(end, p(4, 8));
}

#[test]
fn ordered_swaps_when_head_before_anchor_by_idx() {
    let sel = VisualSelection {
        anchor: p(4, 0),
        head: p(1, 0),
    };
    let (start, end) = sel.ordered();
    assert_eq!(start, p(1, 0));
    assert_eq!(end, p(4, 0));
}

#[test]
fn ordered_breaks_ties_on_idx_by_char_offset() {
    let sel = VisualSelection {
        anchor: p(7, 20),
        head: p(7, 5),
    };
    let (start, end) = sel.ordered();
    assert_eq!(start, p(7, 5));
    assert_eq!(end, p(7, 20));
}
