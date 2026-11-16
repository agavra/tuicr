#!/usr/bin/env python3
# With `show_reviewed = false`, a PR review opens with the cursor on the description at the top. Baseline jumped to
# the first file at an offset measured before the first frame knew the real width, so the diff opened scrolled past
# the end and looked empty.
from e2e import Scenario, repos, scenario


@scenario
def main(s: Scenario) -> None:
    repos.write(s.dir / "config/tuicr/config.toml", "show_reviewed = false\n")
    tui = s.pr_open(keep=True)
    tui.wait_for("PR Mode · octo/demo#1")
    tui.expect("▶ ═══ PR #1 Add greeting and tweak README").expect("More description text here")
    tui.quit()
