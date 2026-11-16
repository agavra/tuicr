#!/usr/bin/env python3
# Covers #915: GraphQL repository coordinates must be sent with `gh api -f` (string), not `-F` (typed: a repo named
# `123` becomes the integer 123 and GitHub rejects the query). Repo octo/123; assert no `-F owner=/name=/after=` in
# the log.
from e2e import Scenario, repos, scenario


@scenario
def main(s: Scenario) -> None:
    repo = repos.pr_repo(s.dir / "repo", origin="https://github.com/octo/123.git")
    tui = s.start("pr", "1", cwd=repo, env={"E2E_VARIANT": "threads_and_commits"})
    tui.wait_for("PR Description", 20).wait_for("remote open thread")
    bad = 0
    for call in tui.forge_calls():
        argv = call["argv"]
        if "graphql" in argv:
            print(" ".join(x for x in argv if not x.startswith("query="))[:150])
            for i, x in enumerate(argv):
                if x == "-F" and argv[i + 1].split("=", 1)[0] in (
                    "owner",
                    "name",
                    "after",
                ):
                    bad += 1
    assert bad == 0, "GraphQL owner/name/after sent with typed -F"
    tui.quit()
