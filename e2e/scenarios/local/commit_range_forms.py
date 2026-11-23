#!/usr/bin/env python3
# `-r A...B` diffs B against the merge base of A and B; `-r A..B` diffs the two trees as they are (so what only A
# has shows as deleted); an explicit multi-commit `-r HEAD~2..HEAD` reviews both commits and not the one before.
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
    # two-dot, explicit shas: base is an ancestor of topic, so this is the same as the merge-base form
    tui = s.start("-r", f"{mb.base}..{mb.topic}", cwd=mb.repo)
    tui.wait_for(r"topic\.txt").expect("topic change").reject("mainonly")
    tui.keys(":q!", "Enter").wait_exit()
    # two-dot across diverged branches: main's own file is deleted on the way to topic (three-dot would hide it)
    tui = s.start("-r", "main..topic", cwd=mb.repo)
    tui.wait_for("Files · 0/3").expect(r"▢ D mainonly\.txt").expect(r"▢ A topic\.txt")
    tui.keys(":q!", "Enter").wait_exit()
    # multi-commit range on the 3-commit repo
    repo = repos.git_repo(s.dir / "repo")
    tui = s.start("-r", "HEAD~2..HEAD", cwd=repo)
    tui.wait_for("Files · 0/8").expect("fix: greeting").expect("feat: touch every file kind").reject(r"\bbase\b")
    tui.keys(":q!", "Enter").wait_exit()
