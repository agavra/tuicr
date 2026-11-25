#!/usr/bin/env python3
# Covers #1003: `-r A...B -w` diffs the working tree from the range's own merge base, not from the parent of the
# oldest listed commit. That parent sits before the merge base when the range contains a merge of the base branch,
# so the base branch's later changes showed up as part of the review. `-r A...B` alone already uses the merge base.
from e2e import Scenario, repos, scenario


@scenario
def main(s: Scenario) -> None:
    mb = repos.git_merge_base(s.dir / "mb")  # topic forks from main, both advance
    repos.git(mb.repo, "checkout", "-q", "topic")
    # The oldest listed commit now has a parent before the merge base.
    repos.git(mb.repo, "merge", "-q", "--no-edit", "main")
    repos.write(mb.repo / "topic.txt", "topic change\nuncommitted edit\n")

    for backend in ("libgit2", "cli"):
        name = f"{s.name}_{backend}"
        repos.write(s.path_of(name) / "config/tuicr/config.toml", f'backend = "{backend}"\n')
        tui = s.start("-r", "main...topic", "-w", cwd=mb.repo, name=name, keep=True)
        tui.wait_for(r"▢ A topic\.txt")  # added on the branch, with its uncommitted edit on top
        tui.expect("Files · 0/1").reject(r"mainonly\.txt")  # only the branch's own file, not what main added
        tui.quit()
