#!/usr/bin/env python3
# Covers #917: Gitea clamps `limit` (MaxResponseItems). With 45 PRs and a server cap of 20/page, the PR list's
# load-more must continue after the rows already shown. Baseline derived the page as already_loaded / page_size (20/30
# + 1 = 1),
# refetched page 1 and showed rows 1..20 twice.
import re
from collections import Counter

from e2e import Scenario, repos, scenario


@scenario
def main(s: Scenario) -> None:
    repo = repos.pr_repo(s.dir / "repo", origin="https://gitea.example.com/octo/demo.git")
    (s.dir / "gt_reviews.json").unlink(missing_ok=True)
    tui = s.start(cwd=repo, env={"E2E_GT_ROWS": "45", "E2E_GT_CAP": "20"})
    tui.wait_for("Pull Requests").settle().keys("Tab").wait_for("GT row 1 ")
    tui.keys(*["j"] * 19).settle()  # most of the way down in one burst
    moves = 0
    while not any("▸" in line and "load more" in line for line in tui.screen().split("\n")):
        tui.keys("j").settle(0.1)
        moves += 1
        assert moves < 80, "never reached the load-more row"
    print(f"load-more row reached after {moves} moves")
    tui.keys("Enter")
    # barrier: the next page's first row (21) is listed right below the load-more row
    tui.wait_for("GT row 21 ")
    for _ in range(60):
        tui.keys("j")
    tui.settle()
    # the list is scrolled; collect every row seen by paging up/down is overkill: duplicates show as row numbers
    # repeating
    dups = [row for row, n in Counter(re.findall("GT row [0-9]*", tui.screen())).items() if n > 1][:3]
    assert not dups, f"duplicate rows after load-more: {dups}"
    tui.expect("GT row (2[1-9]|3[0-9]|4[0-5])")  # else: load-more did not reach rows past the first page
    tui.quit()
