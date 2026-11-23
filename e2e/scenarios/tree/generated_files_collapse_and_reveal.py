#!/usr/bin/env python3
# Covers #874 / #588: both generated attributes collapse by default; explicit reveal and comment jumps keep them usable.
from e2e import Scenario, repos, scenario


@scenario
def main(s: Scenario) -> None:
    repo = repos.empty_repo(s.dir / "repo")
    repos.write(repo / ".gitattributes", "a.auto linguist-generated\nb.auto gitlab-generated\n")
    for name in ("a.auto", "b.auto", "plain.txt"):
        repos.write(repo / name, "original\n")
    repos.git(repo, "add", ".")
    repos.git(repo, "commit", "-qm", "base")
    for name, text in (("a.auto", "LINGUIST_BODY"), ("b.auto", "GITLAB_BODY"), ("plain.txt", "PLAIN_BODY")):
        repos.write(repo / name, text + "\n")
    tui = s.start("-w", cwd=repo)
    tui.wait_for("PLAIN_BODY")
    tui.expect(r"a\.auto.*\[generated\]").expect(r"b\.auto.*\[generated\]")
    tui.reject("LINGUIST_BODY|GITLAB_BODY")
    tui.keys("g", "Space").wait_for("LINGUIST_BODY")
    tui.cursor_on("LINGUIST_BODY").comment("generated comment remains discoverable")
    tui.keys("Space").wait_gone("LINGUIST_BODY")
    tui.expect("1 comment")
    tui.cmd("summary").wait_for("generated comment remains discoverable")
    tui.keys("Enter").wait_for("LINGUIST_BODY").expect("generated comment remains discoverable")
    tui.cmd("set nogenerated").wait_for("GITLAB_BODY").expect("PLAIN_BODY")
    tui.quit()
