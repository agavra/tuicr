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
    graphql = tui.forge_calls(where=lambda c: "graphql" in c["argv"])
    assert graphql, "no GraphQL call was made: nothing checked"

    def fields(argv: list[str], flag: str) -> list[str]:
        """The `key=value` arguments that follow `flag` in argv."""
        return [v for f, v in zip(argv, argv[1:], strict=False) if f == flag]

    for c in graphql:
        typed = [v for v in fields(c["argv"], "-F") if v.split("=", 1)[0] in ("owner", "name", "after")]
        assert not typed, f"GraphQL owner/name/after sent with typed -F: {typed} in {c['argv']}"
    # ...and they are sent as strings: `-f name=123` stays the string "123"
    raw = [fields(c["argv"], "-f") for c in graphql]
    sent = any("owner=octo" in f and "name=123" in f for f in raw)
    assert sent, f"no GraphQL call sent -f owner=octo -f name=123: {raw}"
    tui.quit()
