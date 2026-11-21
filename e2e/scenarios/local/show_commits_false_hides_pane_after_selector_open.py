#!/usr/bin/env python3
# Covers fix(commits): `show_commits = false` was applied only at startup, so a multi-commit review opened later from
# the target selector showed the commit pane again. The pane must stay hidden, `)` still cycles commits, and
# `<leader>s` reveals it for the current review.
from e2e import Scenario, repos, scenario


@scenario
def main(s: Scenario) -> None:
    repo = repos.git_repo(s.dir / "repo")
    repos.write(s.dir / "config" / "tuicr" / "config.toml", "show_commits = false\n")
    tui = s.start(cwd=repo, keep=True)  # keep the config written above
    tui.wait_for("Unstaged changes")
    tui.keys("j", "j", "Space", "Space", "Enter")  # rows: unstaged, staged, fix, feat, base -> fix + feat
    tui.wait_for("Files · 0/8").settle(0.4)
    tui.reject(r"┌ Commits")  # the pane stays hidden for the selector-opened review
    tui.keys(")").wait_for(r"Commit Message \([0-9a-f]+\)")  # cycling still works while hidden
    tui.keys(")").wait_gone(r"Commit Message \(")
    tui.keys(";s").wait_for(r"┌ Commits")  # <leader>s reveals it for this review
    tui.quit()
