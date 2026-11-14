#!/usr/bin/env python3
# Covers #908: a failed load-more on the Pull Requests tab must keep the already loaded rows (and the
# load-more row for a retry). Baseline replaced the whole list with the error panel. 45 rows, page size 30; fake gh
# fails any
# `--limit` above 31 (the load-more request asks for 61).
from e2e import Scenario, repos, scenario


@scenario
def main(s: Scenario) -> None:
    repo = repos.pr_repo(s.dir / "repo")
    tui = s.start(cwd=repo, env={"E2E_LIST_ROWS": "45", "E2E_LIST_FAIL_ABOVE": "31"})
    tui.wait_for("Pull Requests").settle().keys("Tab").wait_for("Row 1 ")
    for _ in range(30):
        tui.keys("j")
    tui.settle()
    print("\n".join(line[:110] for line in tui.screen().split("\n") if "▶" in line or "load more" in line))
    tui.keys("Enter")
    # barrier: the failing load-more request (--limit 61) reached the forge; then the UI absorbs the failure
    tui.wait_forge_log('"--limit", "61"').settle(0.4)
    lines = tui.screen().split("\n")
    print("\n".join(line[:140] for line in lines[0:8] + lines[39:45]))
    tui.expect("Row 1 ", 2)  # else: loaded rows vanished after a failed load-more
    tui.expect("load more pull requests", 2)  # else: load-more row gone, cannot retry
    tui.quit()
