#!/usr/bin/env python3
# `tuicr review clear` empties a session but leaves the file alone while a running TUI owns it, so an agent can keep
# adding comments the TUI picks up (an inactive emptied session is removed: cli/review_json_input_delete_clear).
import json

from e2e import Scenario, scenario


@scenario
def main(s: Scenario) -> None:
    tui = s.diff()
    repo = s.dir / "repo"
    active = [x["slug"] for x in json.loads(s.cli("review", "list", "--repo", repo)) if x.get("active")]
    assert active, "no active session in review list"
    slug = active[0]

    def sess(cmd: str, *args: object) -> str:
        return s.cli("review", cmd, "--repo", repo, "--session", slug, *args)

    sess("add", "--target-file", "README.md", "--line", "4", "first agent note")
    tui.wait_for("first agent note", 15)
    sess("clear")
    tui.wait_gone("first agent note", 15)
    assert s.session_json(), "clear deleted the session a live TUI owns"
    sess("add", "--target-file", "README.md", "--line", "4", "second agent note")
    tui.wait_for("second agent note", 15)
    tui.quit()
