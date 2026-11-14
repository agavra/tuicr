#!/usr/bin/env python3
# `:q` with only an unsaved reviewed mark discards the session, but not one holding a comment an agent added since the
# last poll (polling is off here, so the TUI never saw it): it refuses and shows the comment instead. Header: panel
# G6-1.
import json

from e2e import Scenario, repos, scenario


@scenario
def main(s: Scenario) -> None:
    d = s.dir
    repo = repos.git_repo(d / "repo")
    repos.write(d / "config/tuicr/config.toml", "review_watch_interval_ms = 0\n")
    tui = s.start("-r", "HEAD~2..HEAD", cwd=repo, keep=True)
    tui.wait_for("Files · 0/8").keys("Escape")
    listed = json.loads(s.cli("review", "list", "--repo", repo))
    slug = next(x["slug"] for x in listed if x.get("active"))
    s.cli(
        "review",
        "add",
        "--repo",
        repo,
        "--session",
        slug,
        "--target-file",
        "README.md",
        "--line",
        "4",
        "agent note",
    )
    tui.keys("r").wait_for("Files · 1/8")
    tui.keys(":q", "Enter")
    tui.wait_for("Saved session has comments")
    assert s.session_has("agent note"), "quit deleted the agent's comment"
    tui.quit()
