#!/usr/bin/env python3
# r toggles file reviewed, R toggles hunk reviewed; progress counter and persistence.
from e2e import Scenario, grep, scenario


@scenario
def main(s: Scenario) -> None:
    tui = s.diff()
    tui.expect("Files · 0/8")
    tui.keys("r").wait_for("Files · 1/8").expect(r"✓ M README\.md|✓.*README\.md")
    tui.keys("r").wait_for("Files · 0/8")
    tui.keys("r").wait_for("Files · 1/8")
    # hunk reviewed: go to src/main.rs hunk and press R
    tui.keys("G").keys("R")
    tui.expect("reviewed|✓")
    tui.save()
    j = s.session_json()
    assert grep(r'"reviewed": *true', j), f"no reviewed=true in session\n{j[:2000]}"
    assert grep(r'"reviewed_hunks":\["', j.replace(" ", "").replace("\n", "")), f"no reviewed_hunks entry\n{j[:2000]}"
    tui.quit()
