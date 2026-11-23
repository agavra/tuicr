#!/usr/bin/env python3
# Covers #944: with `show_reviewed = false`, a PR review opens with the cursor on the description at the top.
# Baseline jumped to the first file at an offset measured before the first frame knew the real width, so the diff
# opened scrolled past the end and looked empty.
from e2e import Scenario, repos, scenario


@scenario
def main(s: Scenario) -> None:
    repos.write(s.dir / "config/tuicr/config.toml", "show_reviewed = false\n")
    tui = s.pr_open(keep=True)
    tui.wait_for("PR Mode · octo/demo#1").wait_for("More description text here")
    # the jump happened after the first frame: judge the settled screen, not the first one that matched
    tui.settle()
    tui.check("▶ ═══ PR #1 Add greeting and tweak README", "the cursor left the PR description at startup")
    tui.check("More description text here", "the review opened scrolled past the description")
    tui.check("hello from the PR", "the review opened scrolled past the diff (looks empty)")
    tui.quit()
