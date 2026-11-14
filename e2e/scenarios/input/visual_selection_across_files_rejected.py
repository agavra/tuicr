#!/usr/bin/env python3
# a visual (range) selection that crosses a file boundary must be rejected, not turned into a
# comment on lines of whichever file the numbers happen to fit. Baseline opened the comment box for the cross-file range.
from e2e import Scenario, grep, repos, scenario


@scenario
def main(s: Scenario) -> None:
    r = repos.empty_repo(s.dir / "repo")
    repos.write(r / "a.txt", "a1\na2\na3\n")
    repos.write(r / "b.txt", "b1\nb2\nb3\n")
    repos.git(r, "add", "-A")
    repos.git(r, "commit", "-qm", "base")
    repos.write(r / "a.txt", "a1\na2 changed\na3\n")
    repos.write(r / "b.txt", "b1\nb2 changed\nb3\n")
    tui = s.start("-w", cwd=r)
    tui.wait_for("a.txt").keys("Escape")
    tui.cursor_on("2 ▌ a2 changed")
    tui.keys("v", "j", "j", "j", "j", "j", "j", "j").settle()
    tui.keys("c").wait_for("Invalid selection|COMMENT")
    after = tui.screen()
    assert not grep("COMMENT", after), (
        f"comment box opened for a cross-file selection\n{after}"
    )
    assert grep("Invalid selection", after), f"no 'Invalid selection' warning\n{after}"
    # control: a single-file range still opens the comment box
    tui.keys("v", "j").keys("c").wait_for("COMMENT")
    tui.keys("Escape")
    tui.quit()
