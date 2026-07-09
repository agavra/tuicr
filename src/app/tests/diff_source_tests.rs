use crate::app::*;

#[test]
fn unstaged_source_includes_worktree_changes() {
    assert!(DiffSource::Unstaged.includes_worktree_changes());
}

#[test]
fn commit_range_source_does_not_include_worktree_changes() {
    let source = DiffSource::CommitRange(vec!["abc123".to_string()]);

    assert!(!source.includes_worktree_changes());
}
