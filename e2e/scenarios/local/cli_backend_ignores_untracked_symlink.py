#!/usr/bin/env python3
# Covers #899: with `backend = "cli"` an untracked symlink must not be followed: content outside the repo
# (here a "secret" file) must not appear in the Unstaged view. Baseline reads through the link and shows it.
import shutil

from e2e import Scenario, repos, scenario


@scenario
def main(s: Scenario) -> None:
    d = s.dir
    repo = repos.git_repo(d / "repo")
    repos.write(d / "secret.txt", "TOP-SECRET-OUTSIDE-REPO\n")
    (repo / "leak_link").symlink_to(d / "secret.txt")
    repos.write(d / "config/tuicr/config.toml", 'backend = "cli"\n')
    for sub in ("home", "data", "state"):
        shutil.rmtree(d / sub, ignore_errors=True)
    tui = s.start(cwd=repo, keep=True)
    tui.wait_for("Unstaged changes")
    tui.keys("Enter")
    tui.wait_for(r"untracked\.txt")
    tui.expect("leak_link")
    # the link is listed as an added file whose content is the link text, not the file behind it
    tui.expect(r"leak_link \[A\]")
    tui.keys("Escape").keys("G").settle()
    tui.reject("TOP-SECRET", 0.3)  # untracked symlink was followed: outside content shown
    # scroll the whole diff to be sure
    for _ in range(6):
        tui.keys("C-d")
    tui.settle()
    tui.reject("TOP-SECRET", 0.3)  # untracked symlink was followed: outside content shown
    tui.quit()
