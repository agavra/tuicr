#!/usr/bin/env python3
# :set generated clamps cursor_line and scroll_offset instead of stranding the cursor past EOF.
from e2e import Scenario, repos, scenario


@scenario
def main(s: Scenario) -> None:
    repo = repos.empty_repo(s.dir / "repo")
    repos.write(repo / ".gitattributes", "a.auto linguist-generated\n")
    repos.write(repo / "a.auto", "base\n")
    repos.write(repo / "z_plain.txt", "base\n")
    repos.git(repo, "add", ".")
    repos.git(repo, "commit", "-qm", "base")
    repos.write(repo / "a.auto", "".join(f"GEN_LINE_{i}\n" for i in range(60)))
    repos.write(repo / "z_plain.txt", "PLAIN_TAIL\n")

    tui = s.start("-w", cwd=repo)
    tui.wait_for("PLAIN_TAIL")
    tui.cmd("set nogenerated").wait_for("GEN_LINE_0")
    tui.keys("G").wait_for("PLAIN_TAIL")
    tui.cmd("set generated").wait_for(r"Collapsing 1 generated file")
    # Cursor and viewport must remain in bounds on a navigable row so `r` works immediately.
    tui.expect(r"a\.auto.*\[generated\]")
    assert tui.cursor_row() is not None, "cursor disappeared after :set generated"
    tui.keys("r").wait_for("Files · 1/2")
    tui.quit()
