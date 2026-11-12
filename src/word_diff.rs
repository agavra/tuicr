//! Word-level ("intraline") diff for changed lines.
//!
//! A hunk shows a modified line as a deletion followed by an addition. This
//! module pairs those lines up and finds the words that actually changed, so
//! renderers can paint them with a stronger background than the rest of the
//! line. The result is byte ranges into `DiffLine::content`; rendering decides
//! how to show them.

use std::ops::Range;

use crate::model::{DiffLine, LineOrigin};

/// Changed byte ranges of one line, sorted and non-overlapping.
pub type ChangedRanges = Vec<Range<usize>>;

/// Skip pairs whose token-count product exceeds this. The LCS table is
/// `n * m` cells, and a line that long is unreadable either way.
const MAX_LCS_CELLS: usize = 250_000;

/// Above this share of changed bytes the two lines are a rewrite, not an
/// edit, and emphasizing nearly everything is noise.
const MAX_CHANGED_RATIO: f64 = 0.6;

/// Changed ranges for every line of a hunk, indexed like `lines`.
///
/// `None` means "draw the line as usual": context lines, additions or
/// deletions with no partner, and pairs that differ too much to be worth
/// emphasizing. Within each run of deletions followed directly by a run of
/// additions, the k-th deletion is paired with the k-th addition.
pub fn hunk_changed_ranges(lines: &[DiffLine]) -> Vec<Option<ChangedRanges>> {
    let mut out: Vec<Option<ChangedRanges>> = vec![None; lines.len()];
    let mut i = 0;
    while i < lines.len() {
        if lines[i].origin != LineOrigin::Deletion {
            i += 1;
            continue;
        }
        let del_start = i;
        while i < lines.len() && lines[i].origin == LineOrigin::Deletion {
            i += 1;
        }
        let add_start = i;
        while i < lines.len() && lines[i].origin == LineOrigin::Addition {
            i += 1;
        }
        let pairs = (add_start - del_start).min(i - add_start);
        for k in 0..pairs {
            let (del, add) = (del_start + k, add_start + k);
            if let Some((old, new)) = changed_ranges(&lines[del].content, &lines[add].content) {
                out[del] = Some(old);
                out[add] = Some(new);
            }
        }
    }
    out
}

/// Changed byte ranges of `old` and `new` when they are similar enough to
/// be read as an edit of one another.
pub fn changed_ranges(old: &str, new: &str) -> Option<(ChangedRanges, ChangedRanges)> {
    if old == new {
        return None;
    }
    let old_tokens = tokenize(old);
    let new_tokens = tokenize(new);

    // Common prefix and suffix are cheap to strip and keep the table small.
    let prefix = old_tokens
        .iter()
        .zip(&new_tokens)
        .take_while(|(a, b)| old[(*a).clone()] == new[(*b).clone()])
        .count();
    let suffix = old_tokens[prefix..]
        .iter()
        .rev()
        .zip(new_tokens[prefix..].iter().rev())
        .take_while(|(a, b)| old[(*a).clone()] == new[(*b).clone()])
        .count();
    let old_mid = &old_tokens[prefix..old_tokens.len() - suffix];
    let new_mid = &new_tokens[prefix..new_tokens.len() - suffix];

    if old_mid.len().saturating_mul(new_mid.len()) > MAX_LCS_CELLS {
        return None;
    }
    let (old_kept, new_kept) = lcs_matches(old, old_mid, new, new_mid);

    let old_changed = changed_token_ranges(old, old_mid, &old_kept);
    let new_changed = changed_token_ranges(new, new_mid, &new_kept);

    let changed: usize = old_changed
        .iter()
        .chain(&new_changed)
        .map(|r| r.len())
        .sum();
    let total = old.len() + new.len();
    if total == 0 || changed as f64 / total as f64 > MAX_CHANGED_RATIO {
        return None;
    }
    Some((old_changed, new_changed))
}

/// Split a line into word runs (alphanumerics and `_`), whitespace runs, and
/// single punctuation characters, as byte ranges.
fn tokenize(line: &str) -> Vec<Range<usize>> {
    #[derive(PartialEq)]
    enum Class {
        Word,
        Space,
        Other,
    }
    let class = |c: char| {
        if c.is_alphanumeric() || c == '_' {
            Class::Word
        } else if c.is_whitespace() {
            Class::Space
        } else {
            Class::Other
        }
    };

    let mut tokens: Vec<Range<usize>> = Vec::new();
    let mut current: Option<(usize, Class)> = None;
    for (idx, c) in line.char_indices() {
        let cls = class(c);
        match current {
            Some((_, ref cur)) if *cur == cls && cls != Class::Other => {}
            Some((start, _)) => {
                tokens.push(start..idx);
                current = Some((idx, cls));
            }
            None => current = Some((idx, cls)),
        }
    }
    if let Some((start, _)) = current {
        tokens.push(start..line.len());
    }
    tokens
}

/// Longest common subsequence of two token lists. Returns, for each side, a
/// flag per token telling whether it is part of the common subsequence.
fn lcs_matches(
    old: &str,
    old_tokens: &[Range<usize>],
    new: &str,
    new_tokens: &[Range<usize>],
) -> (Vec<bool>, Vec<bool>) {
    let (n, m) = (old_tokens.len(), new_tokens.len());
    let mut old_kept = vec![false; n];
    let mut new_kept = vec![false; m];
    if n == 0 || m == 0 {
        return (old_kept, new_kept);
    }

    // lengths[i][j] = LCS length of old_tokens[i..] and new_tokens[j..].
    let width = m + 1;
    let mut lengths = vec![0u32; (n + 1) * width];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            lengths[i * width + j] = if old[old_tokens[i].clone()] == new[new_tokens[j].clone()] {
                lengths[(i + 1) * width + j + 1] + 1
            } else {
                lengths[(i + 1) * width + j].max(lengths[i * width + j + 1])
            };
        }
    }

    let (mut i, mut j) = (0, 0);
    while i < n && j < m {
        if old[old_tokens[i].clone()] == new[new_tokens[j].clone()] {
            old_kept[i] = true;
            new_kept[j] = true;
            i += 1;
            j += 1;
        } else if lengths[(i + 1) * width + j] >= lengths[i * width + j + 1] {
            i += 1;
        } else {
            j += 1;
        }
    }
    (old_kept, new_kept)
}

/// Merge the byte ranges of tokens that are not kept. A run of unchanged
/// whitespace between two changed tokens is absorbed, so `foo bar` replaced
/// by `baz qux` reads as one block instead of two.
fn changed_token_ranges(line: &str, tokens: &[Range<usize>], kept: &[bool]) -> ChangedRanges {
    let mut ranges: ChangedRanges = Vec::new();
    for (token, _) in tokens.iter().zip(kept).filter(|(_, kept)| !**kept) {
        match ranges.last_mut() {
            Some(last) if line[last.end..token.start].chars().all(char::is_whitespace) => {
                last.end = token.end;
            }
            _ => ranges.push(token.clone()),
        }
    }
    ranges
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(origin: LineOrigin, content: &str) -> DiffLine {
        DiffLine {
            origin,
            content: content.to_string(),
            raw_content: None,
            old_lineno: None,
            new_lineno: None,
            highlighted_spans: None,
        }
    }

    fn slices<'a>(text: &'a str, ranges: &[Range<usize>]) -> Vec<&'a str> {
        ranges.iter().map(|r| &text[r.clone()]).collect()
    }

    #[test]
    fn should_emphasize_only_the_changed_number() {
        let old = "the DB by RDS backups (35 days, prerequisite 17) and the DBA backups";
        let new = "the DB by RDS backups (35 days, prerequisite 14) and the DBA backups";

        let (old_r, new_r) = changed_ranges(old, new).expect("similar lines");

        assert_eq!(slices(old, &old_r), ["17"]);
        assert_eq!(slices(new, &new_r), ["14"]);
    }

    #[test]
    fn should_leave_one_side_empty_for_a_pure_insertion() {
        let old = "call(a)";
        let new = "call(a, b)";

        let (old_r, new_r) = changed_ranges(old, new).expect("similar lines");

        assert!(old_r.is_empty());
        assert_eq!(slices(new, &new_r), [", b"]);
    }

    #[test]
    fn should_merge_changed_words_separated_by_a_space() {
        let old = "let value = foo bar;";
        let new = "let value = baz qux;";

        let (old_r, new_r) = changed_ranges(old, new).expect("similar lines");

        assert_eq!(slices(old, &old_r), ["foo bar"]);
        assert_eq!(slices(new, &new_r), ["baz qux"]);
    }

    #[test]
    fn should_skip_lines_that_were_rewritten() {
        assert_eq!(changed_ranges("fn alpha() {}", "let x = 42;"), None);
    }

    #[test]
    fn should_skip_identical_lines() {
        assert_eq!(changed_ranges("same", "same"), None);
    }

    #[test]
    fn should_keep_ranges_on_char_boundaries_for_non_ascii_text() {
        let old = "привіт світ → 17";
        let new = "привіт світ → 14";

        let (old_r, new_r) = changed_ranges(old, new).expect("similar lines");

        assert_eq!(slices(old, &old_r), ["17"]);
        assert_eq!(slices(new, &new_r), ["14"]);
    }

    #[test]
    fn should_pair_deletions_with_additions_in_order() {
        let lines = vec![
            line(LineOrigin::Context, "fn f() {"),
            line(LineOrigin::Deletion, "    let a = 1;"),
            line(LineOrigin::Deletion, "    let b = 2;"),
            line(LineOrigin::Addition, "    let a = 10;"),
            line(LineOrigin::Addition, "    let b = 20;"),
            line(LineOrigin::Addition, "    let c = 30;"),
            line(LineOrigin::Context, "}"),
        ];

        let ranges = hunk_changed_ranges(&lines);

        let shown: Vec<Option<Vec<&str>>> = ranges
            .iter()
            .zip(&lines)
            .map(|(r, l)| r.as_ref().map(|r| slices(&l.content, r)))
            .collect();
        assert_eq!(
            shown,
            [
                None,
                Some(vec!["1"]),
                Some(vec!["2"]),
                Some(vec!["10"]),
                Some(vec!["20"]),
                None,
                None,
            ]
        );
    }

    #[test]
    fn should_not_pair_runs_split_by_context() {
        let lines = vec![
            line(LineOrigin::Deletion, "let a = 1;"),
            line(LineOrigin::Context, "middle"),
            line(LineOrigin::Addition, "let a = 2;"),
        ];

        assert_eq!(hunk_changed_ranges(&lines), [None, None, None]);
    }

    #[test]
    fn should_give_up_on_huge_lines() {
        let old = "a ".repeat(1000);
        let new = "b ".repeat(1000);

        assert_eq!(changed_ranges(&old, &new), None);
    }
}
