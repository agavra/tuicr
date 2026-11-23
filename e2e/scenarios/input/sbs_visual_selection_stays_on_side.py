#!/usr/bin/env python3
# Covers #577: in side-by-side, a visual selection stays on the pane it was anchored on. `j` in
# visual mode steps over rows that exist only on the other pane, so `y` copies just the anchored
# side; plain `j` still visits every row. Before, one `j` landed on a deletion-only row and the
# yank pulled the deleted old-side text into a new-side selection.
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


@scenario
def main(s: Scenario) -> None:
    repo = repos.empty_repo(s.dir / "repo")
    repos.write(repo / "shapes.rs", OLD)
    repos.git(repo, "add", "-A")
    repos.git(repo, "commit", "-qm", "initial")
    repos.write(repo / "shapes.rs", NEW)

    repos.write(s.dir / "config" / "tuicr" / "config.toml", 'diff_view = "side-by-side"\n')
    clipboard = s.fake_clipboard()
    tui = s.start(cwd=repo, keep=True)
    tui.wait_for("Unstaged changes").keys("Enter")
    tui.wait_for("Files · 0/1")
    tui.cursor_on(r"Shape::Circle \{ r \} => \{")

    # Control: normal-mode `j` still visits the deletion-only row below.
    tui.keys("j").settle()
    row = tui.cursor_row()
    assert "let pi" in row, f"normal j no longer reaches the deletion-only row:\n{row}"
    tui.keys("k").settle()

    # A visual selection anchored on the NEW side skips that row and copies
    # only new-side text.
    tui.keys("v", "j", "y")
    clip = clipboard.text()
    expected = "        Shape::Circle { r } => std::f64::consts::PI * r * r,\n        Shape::Square { s } => s * s,"
    assert clip == expected, f"visual yank on the new side copied old-side rows:\n{clip!r}"
    tui.quit()
