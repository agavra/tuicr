#!/usr/bin/env python3
# #868: a same-head reload while the cumulative diff is shown must refresh the cached cumulative
# diff, so narrowing to a commit and widening back shows the reloaded diff, not the stale one.
from e2e import Scenario, scenario


@scenario
def main(s: Scenario) -> None:
    s.dir.mkdir(parents=True, exist_ok=True)
    variant = s.dir / "variant"
    variant.write_text("threads_and_commits\n")
    tui = s.pr_open(env={"E2E_VARIANT_FILE": str(variant)})
    tui.keys(";k").keys("j", "Enter")  # narrow to bbbbbbb
    tui.wait_for(r"Commit Message \(bbbbbbb\)")
    tui.keys(")").wait_for(r"Commit Message \(aaaaaaa\)")
    tui.keys(")").wait_gone(r"Commit Message \(").wait_for(r"\+?.*new line")  # whole PR again
    variant.write_text("cumulative_refresh\n")  # same head, but the cumulative diff changed
    tui.cmd("e")
    tui.wait_for("newer line")
    tui.keys(";k").keys("k", "k", "k").settle().keys("Enter")
    tui.wait_for(r"Commit Message \(2222222\)")
    tui.keys(")").wait_for(r"Commit Message \(bbbbbbb\)")
    tui.keys(")").wait_for(r"Commit Message \(aaaaaaa\)")
    tui.keys(")").wait_gone(r"Commit Message \(")
    tui.reject(r"Commit Message \(")  # else: not back on the whole PR
    tui.wait_for("newer line")  # else: widened selection shows the stale cumulative diff
    tui.quit()
