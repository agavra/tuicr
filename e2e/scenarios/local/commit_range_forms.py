#!/usr/bin/env python3
# -r A..B, -r A...B (merge-base forms) and the default -r HEAD~2..HEAD.
from e2e import Scenario, repos, scenario


@scenario
def main(s: Scenario) -> None:
    mb = repos.git_merge_base(s.dir / "mb")
    tui = s.start("-r", "main...topic", cwd=mb.repo)
    tui.wait_for(r"topic\.txt").expect("topic change").reject("mainonly")
    tui.keys(":q!", "Enter").wait_exit()
    # the other side
    tui = s.start("-r", "topic...main", cwd=mb.repo)
    tui.wait_for(r"mainonly\.txt").expect("main change").reject(r"topic\.txt")
    tui.keys(":q!", "Enter").wait_exit()
    # two-dot, explicit shas
    tui = s.start("-r", f"{mb.base}..{mb.topic}", cwd=mb.repo)
    tui.wait_for(r"topic\.txt").expect("topic change")
    tui.keys(":q!", "Enter").wait_exit()
    # multi-commit range on the 3-commit repo
    repo = repos.git_repo(s.dir / "repo")
    tui = s.start("-r", "HEAD~2..HEAD", cwd=repo)
    tui.wait_for("Files · 0/8").expect("fix: greeting").expect("feat: touch every file kind").reject(
        r"1f95fd7|\bbase\b"
    )
    tui.keys(":q!", "Enter").wait_exit()
