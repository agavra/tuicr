#!/usr/bin/env python3
# `diff_watch_interval_ms` re-reads the working-tree diff on a timer: an edit made outside shows up without `:e`.
# Without the setting (default 0) the same edit stays invisible until `:e`.
from pathlib import Path

from e2e import Scenario, repos, scenario


def append(path: Path, text: str) -> None:
    with open(path, "a") as f:
        f.write(text)


@scenario
def main(s: Scenario) -> None:
    d = s.dir
    repo = repos.git_repo(d / "repo")
    repos.write(d / "config/tuicr/config.toml", "diff_watch_interval_ms = 200\n")
    tui = s.start("-w", cwd=repo, keep=True)
    tui.wait_for("unstaged line")
    append(repo / "README.md", "edited behind its back\n")
    tui.wait_for("edited behind its back", 15)
    tui.quit()
    repos.git(repo, "checkout", "-q", "README.md")
    append(repo / "README.md", "x\n")
    off = s.start("-w", cwd=repo, name=f"{s.name}_off")
    off.wait_for("5 ▌ x")
    append(repo / "README.md", "edited behind its back\n")
    off.reject("edited behind its back", 2)
    off.cmd("e").wait_for("edited behind its back")
    off.quit()
