#!/usr/bin/env python3
# #867: when the PR gets a new head with a changed commit list, `:e` resets a narrowed
# selection to the whole PR (no stale commit-message pseudo file above the cumulative diff),
# and the reviewed mark of a surviving commit's message is carried to the new head.
from e2e import Scenario, scenario


@scenario
def main(s: Scenario) -> None:
    s.dir.mkdir(parents=True, exist_ok=True)
    variant = s.dir / "variant"
    variant.write_text("threads_and_commits\n")
    tui = s.pr_open(env={"E2E_VARIANT_FILE": str(variant)})
    tui.keys(";k")
    tui.keys("j", "Enter")  # narrow to bbbbbbb (second commit)
    tui.wait_for(r"Commit Message \(bbbbbbb\)").wait_for("line from commit B")
    tui.keys("Escape").keys("g")  # diff pane, top = commit message entry
    tui.keys("r").wait_for(r"✓.*Commit Message \(bbbbbbb\)")
    # the PR gets a new head and a fourth commit
    variant.write_text("moved_head\n")
    tui.cmd("e")
    # the reload resets the selection to the cumulative diff: the single-commit message above it goes away
    tui.wait_gone(r"Commit Message \(", 15).wait_for("hello from the PR").reject(r"Commit Message \(")
    tui.check(r"▣.*4444444|4444444.*fourth|fourth commit", "new commit not listed")
    # narrow to bbbbbbb again: its message review must have been carried over
    tui.keys(";k").keys("j", "j").wait_for("▸.*bbbbbbb").keys("Enter")
    tui.wait_for(r"Commit Message \(bbbbbbb\)")
    tui.check(
        r"✓.*Commit Message \(bbbbbbb\)",
        "reviewed mark of the commit message was not carried",
    )
    tui.quit()
