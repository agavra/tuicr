#!/usr/bin/env python3
# Covers #908: a failed load-more on the Pull Requests tab must keep the already loaded rows (and the
# load-more row for a retry). Baseline replaced the whole list with the error panel. 45 rows, page size 30; fake gh
# fails any `--limit` above 31 (the load-more request asks for 61).
from e2e import Scenario, repos, scenario


@scenario
def main(s: Scenario) -> None:
    repo = repos.pr_repo(s.dir / "repo")
    tui = s.start(cwd=repo, env={"E2E_LIST_ROWS": "45", "E2E_LIST_FAIL_ABOVE": "31"})
    tui.wait_for("Pull Requests").settle().keys("Tab").wait_for("Row 1 ").wait_for("30 loaded")
    for _ in range(30):
        tui.keys("j")
    tui.wait_for("▸ .*load more pull requests")
    tui.keys("Enter")
    # barrier: the failing load-more request (--limit 61) reached the forge, then the UI reported its failure
    tui.wait_forge_log('"--limit", "61"')
    tui.wait_for("simulated network failure")
    # the loaded rows and the load-more row are still listed, nothing of the failed page was added
    tui.check("Row 1 ", "loaded rows vanished after a failed load-more")
    tui.check("Row 30 ", "loaded rows vanished after a failed load-more")
    tui.check("load more pull requests", "load-more row gone, cannot retry")
    tui.check("30 loaded", "the loaded count changed after a failed load-more")
    tui.reject("Row 31 |error · ")  # else: rows of the failed page shown, or the list replaced by the error panel
    tui.quit()
