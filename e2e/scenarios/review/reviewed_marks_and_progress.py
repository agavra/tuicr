#!/usr/bin/env python3
# r toggles file reviewed (the progress counter follows), R toggles hunk reviewed; both persist with :w, scoped to the
# file they were made on.
from e2e import Scenario, scenario


@scenario
def main(s: Scenario) -> None:
    tui = s.diff()
    tui.expect("Files · 0/8")
    tui.keys("r").wait_for("Files · 1/8").expect(r"✓ M README\.md|✓.*README\.md")
    tui.keys("r").wait_for("Files · 0/8")
    tui.keys("r").wait_for("Files · 1/8")
    # hunk reviewed: the src/main.rs hunk folds to a `✓ @@` row
    tui.cursor_on(r'println!\("hello"\)')
    tui.keys("R").wait_for(r"▶ *✓ @@").wait_gone(r'println!\("hello"\)')
    tui.expect("Files · 1/8")  # a hunk mark is not a file mark
    tui.save()
    files = s.session()["files"]
    reviewed = sorted(p for p, f in files.items() if f["reviewed"])
    hunks = {p: len(f["reviewed_hunks"]) for p, f in files.items() if f["reviewed_hunks"]}
    assert reviewed == ["README.md"], f"reviewed files {reviewed}, want only README.md"
    assert hunks == {"src/main.rs": 1}, f"reviewed hunks {hunks}, want one in src/main.rs"
    # R on the folded hunk toggles it back
    tui.keys("R").wait_for(r'println!\("hello"\)').wait_gone(r"✓ @@")
    tui.wait_gone("Saved to")  # the first save's message expires, so the next one is this save's
    tui.save()
    assert s.session()["files"]["src/main.rs"]["reviewed_hunks"] == [], "the hunk mark survived the toggle-off"
    tui.quit()
