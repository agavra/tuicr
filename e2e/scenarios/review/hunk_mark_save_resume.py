#!/usr/bin/env python3
# Marking ONE hunk reviewed (R) collapses just that hunk to a "✓ @@" row; after :w, quit and restart the same hunk (and
# only it) is still collapsed; R on it again restores its lines and the saved session forgets the mark.
from e2e import Scenario, Tui, repos, scenario


@scenario
def main(s: Scenario) -> None:
    repo = repos.fresh(s.dir / "repo")
    repos.run(repo, "git", "init", "-q", "-b", "main", str(repo))
    many = repo / "many.txt"
    repos.write(many, "".join(f"row {i}\n" for i in range(1, 201)))
    repos.git(repo, "add", "-A")
    repos.git(repo, "commit", "-qm", "base")
    lines = many.read_bytes().decode().split("\n")
    for i, w in ((10, "ten"), (100, "hundred"), (190, "ninety")):
        lines[i - 1] = f"row {w} CHANGED"
    repos.write(many, "\n".join(lines))

    def open_diff() -> Tui:
        tui = s.start(cwd=repo, keep=True)
        tui.wait_for("Unstaged changes").keys("Enter").wait_for(r"many\.txt")
        return tui.wait_for(r"@@ -187,7 \+187,7 @@")

    tui = open_diff()
    tui.keys("]", "]").wait_for(r"▶ +@@ -97,7 \+97,7 @@")  # the middle hunk
    tui.expect("row hundred CHANGED")
    tui.keys("R").wait_gone("row hundred CHANGED").expect(r"✓ @@ -97,7 \+97,7 @@")
    tui.expect("row ten CHANGED").expect("row ninety CHANGED")  # the other hunks are untouched
    tui.expect("Files · 0/1")  # a hunk mark is not a file mark
    tui.save().cmd("q!").wait_exit()
    fr = s.session()["files"]["many.txt"]
    assert len(fr["reviewed_hunks"]) == 1 and not fr["reviewed"], f"saved session lacks exactly one hunk mark: {fr}"
    # restart: the same hunk is still reviewed, the others are not
    tui = open_diff()
    tui.wait_for(r"✓ @@ -97,7 \+97,7 @@").wait_gone("row hundred CHANGED")
    tui.expect("row ten CHANGED").expect("row ninety CHANGED").reject(r"✓ @@ -7,7|✓ @@ -187,7")
    # toggle it off again: lines are back, and a saved session forgets the mark
    tui.keys("]", "]").wait_for(r"▶ +✓ @@ -97,7 \+97,7 @@")
    tui.keys("R").wait_for("row hundred CHANGED").wait_gone("✓ @@")
    tui.save()
    fr = s.session()["files"]["many.txt"]
    assert fr["reviewed_hunks"] == [], f"hunk mark survived the toggle-off: {fr}"
    tui.quit()
