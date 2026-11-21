#!/usr/bin/env python3
# Covers #899: with `backend = "cli"` an untracked symlink must not be followed: content outside the repo
# (here a "secret" file) must not appear in the Unstaged view. Baseline reads through the link and shows it.
from e2e import Scenario, flows, repos, scenario


@scenario
def main(s: Scenario) -> None:
    d = s.dir
    repo = repos.git_repo(d / "repo")
    repos.write(d / "secret.txt", "TOP-SECRET-OUTSIDE-REPO\n")
    (repo / "leak_link").symlink_to("../secret.txt")  # relative: short enough to show whole
    repos.write(d / "config/tuicr/config.toml", 'backend = "cli"\n')
    git = flows.GitCalls(s)
    tui = s.start(cwd=repo, keep=True)  # keep the config written above
    tui.wait_for("Unstaged changes")
    assert git.cli_backend(), f'backend = "cli" did not take effect; git calls: {git.calls()}'
    tui.keys("Enter")
    tui.wait_for(r"untracked\.txt")
    # the link is listed as an added file whose content is the link text (its target path), not the file behind it
    tui.expect(r"leak_link \[A\]")
    tui.expect(r"1 ▌ \.\./secret\.txt")
    tui.reject("TOP-SECRET")  # untracked symlink was followed: outside content shown
    tui.quit()
