#!/usr/bin/env python3
# Covers the follow-up to #984: in side-by-side, a new-side visual selection over a deletion run looks
# contiguous once `j` skips the rows that only exist on the old pane. `s` must open a suggestion with
# just the new-side lines instead of warning that the selection has gaps.
from e2e import Scenario, repos, scenario

OLD = """fn area(shape: &Shape) -> f64 {
    match shape {
        Shape::Circle { r } => {
            let pi = std::f64::consts::PI;
            let squared = r * r;
            let result = pi * squared;
            result
        }
        Shape::Square { s } => s * s,
    }
}
"""

NEW = """fn area(shape: &Shape) -> f64 {
    match shape {
        Shape::Circle { r } => std::f64::consts::PI * r * r,
        Shape::Square { s } => s * s,
    }
}
"""

SUGGESTION = """```suggestion
        Shape::Circle { r } => std::f64::consts::PI * r * r,
        Shape::Square { s } => s * s,
```"""


@scenario
def main(s: Scenario) -> None:
    repo = repos.empty_repo(s.dir / "repo")
    repos.write(repo / "shapes.rs", OLD)
    repos.git(repo, "add", "-A")
    repos.git(repo, "commit", "-qm", "initial")
    repos.write(repo / "shapes.rs", NEW)

    repos.write(s.dir / "config" / "tuicr" / "config.toml", 'diff_view = "side-by-side"\n')
    tui = s.start(cwd=repo, keep=True)
    tui.wait_for("Unstaged changes").keys("Enter")
    tui.wait_for("Files · 0/1")
    tui.cursor_on(r"Shape::Circle \{ r \} => \{")

    # `j` skips the old-only rows, so the selection reads as new lines 3-4 with
    # the deleted block invisible on the new pane. Before the fix `s` warned.
    tui.keys("v", "j", "s").settle()
    screen = tui.screen()
    assert "Suggestions must stay within one diff hunk" not in screen, (
        f"suggestion rejected the deletion run as a gap:\n{screen}"
    )
    tui.wait_for("^ COMMENT ")
    assert "```suggestion" in tui.screen(), "no suggestion fence in the comment box"

    tui.keys("Enter").wait_saved_comment(SUGGESTION)
    comments = [c for f in s.session()["files"].values() for cs in f["line_comments"].values() for c in cs]
    assert len(comments) == 1, comments
    comment = comments[0]
    assert comment["content"] == SUGGESTION, comment
    # New lines 3-4: the deletion run contributes no line to the range or body.
    assert comment["side"] == "new" and comment["line_range"] == {"start": 3, "end": 4}, comment
    tui.quit()
