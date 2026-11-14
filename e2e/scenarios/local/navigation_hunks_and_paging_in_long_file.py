#!/usr/bin/env python3
# One file with three distant hunks (lines 10, 100, 190 of 200): `]`/`[` step hunk by hunk inside the file and stop
# at both
# ends, Ctrl-d / Ctrl-u page the cursor down and back up to known rows (a no-op paging implementation cannot pass).
from e2e import Scenario, repos, scenario


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
    tui = s.start(cwd=repo)
    tui.wait_for("Unstaged changes").keys("Enter").wait_for(r"many\.txt")
    tui.wait_for(r"@@ -187,7 \+187,7 @@")  # all three hunks are on screen at this size

    def at(text: str) -> None:  # the cursor row shows this text
        tui.wait_for(f"▶ +{text}")

    at(r"═══ many\.txt")
    # paging: half a page down lands on row 101, the next one on the last expand row, Ctrl-u comes back up
    tui.keys("C-d")
    at("101 +row 101")
    tui.keys("C-d")
    at(r"\.\.\. ↓ expand \(7 lines\)")
    tui.keys("C-u")
    at("97 +row 97")
    # hunks: forward through all three, no wrap past the last, then back
    tui.keys("g")
    at(r"\.\.\. ↑ expand \(6 lines\)")  # top of the file's content
    tui.keys("]")
    at(r"@@ -7,7 \+7,7 @@")
    tui.keys("]")
    at(r"@@ -97,7 \+97,7 @@")
    tui.keys("]")
    at(r"@@ -187,7 \+187,7 @@")
    tui.keys("]").settle()
    at(r"@@ -187,7 \+187,7 @@")  # last hunk: stays
    tui.keys("[")
    at(r"@@ -97,7 \+97,7 @@")
    tui.keys("[")
    at(r"@@ -7,7 \+7,7 @@")
    tui.quit()
