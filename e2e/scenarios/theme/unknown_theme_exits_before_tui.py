#!/usr/bin/env python3
# `--theme <unknown>` fails at startup (exit 2, lists the bundled themes) without opening the TUI; a valid `--theme`
# beats an unknown `theme` in config.toml.
import os

from e2e import Scenario, grep, repos, scenario


@scenario
def main(s: Scenario) -> None:
    (s.dir / "config" / "tuicr").mkdir(parents=True)
    repo = repos.git_repo(s.dir / "repo")
    os.chdir(repo)
    res = s.cli_result("--no-update-check", "--theme", "nosuch-theme")
    assert res.rc == 2, f"exit status {res.rc}, expected 2\n{res.out}"
    assert grep("Unknown theme 'nosuch-theme'", res.out) and grep(
        "nord-dark", res.out
    ), f"no theme error with the bundled list:\n{res.out}"
    repos.write(s.dir / "config" / "tuicr" / "config.toml", 'theme = "also-missing"\n')
    tui = s.start("--theme", "nord-dark", "-r", "HEAD~2..HEAD", cwd=repo, keep=True)
    tui.wait_for("Files · 0/8")
    tui.quit()
