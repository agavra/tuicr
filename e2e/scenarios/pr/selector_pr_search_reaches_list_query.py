#!/usr/bin/env python3
# Covers #738: `[forge] pr_search` replaces `review-requested:@me` as the GitHub search of the Pull Requests review
# scope, on the first page and on load more, and the scope shows as `search`. 45 rows, page size 30.
from e2e import Scenario, repos, scenario

SEARCH = "review-requested:@me draft:false"


@scenario
def main(s: Scenario) -> None:
    repo = repos.pr_repo(s.dir / "repo")
    repos.write(s.dir / "config" / "tuicr" / "config.toml", f'[forge]\npr_search = "{SEARCH}"\n')
    tui = s.start(cwd=repo, keep=True, env={"E2E_LIST_ROWS": "45"})
    tui.wait_for("Pull Requests").settle().keys("Tab").wait_for("Row 1 ")
    tui.wait_for("r search")  # the hint names the configured search scope, not "requested"
    tui.keys("r").wait_for("search · ").wait_for("Row 1 ")
    # first page of the review scope: the configured search, not the default query
    tui.wait_forge_log(f'"--limit", "31", "--json", "[^"]*", "--search", "{SEARCH}"')
    for _ in range(30):
        tui.keys("j")
    tui.settle().keys("Enter")
    # load more (the second page) still sends the configured search
    tui.wait_forge_log(f'"--limit", "61", "--json", "[^"]*", "--search", "{SEARCH}"')
    tui.wait_for("search · 45 loaded")
    lists = [c["argv"] for c in tui.forge_calls() if c["argv"][:2] == ["pr", "list"]]
    searches = [a[a.index("--search") + 1] for a in lists if "--search" in a]
    assert searches and all(q == SEARCH for q in searches), f"review scope sent another search: {lists}"
    tui.quit()
