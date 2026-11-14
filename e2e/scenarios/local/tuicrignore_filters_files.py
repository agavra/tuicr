#!/usr/bin/env python3
# A repo-root `.tuicrignore` (gitignore syntax) drops matching files from the review at load time: patterns, a directory
# rule and a `!` re-include; a later edit is picked up by `:e`.
from e2e import Scenario, repos, scenario


@scenario
def main(s: Scenario) -> None:
    repo = repos.git_repo(s.dir / "repo")
    repos.write(repo / ".tuicrignore", "*.bin\nnotes_*.txt\nsrc/\n!src/main.rs\n")
    tui = s.start("-r", "HEAD~2..HEAD", cwd=repo)
    tui.wait_for("Files · 0/5")  # 8 files - data.bin - notes_cjk.txt - src/lib.rs
    tui.keys("Escape")
    tui.expect(r"M README\.md").expect(r"tabs\.txt").expect(r"main\.rs")
    tui.reject(r"data\.bin|notes_cjk|lib\.rs")
    repos.write(repo / ".tuicrignore", "*.bin\n")
    tui.cmd("e").wait_for("Files · 0/7").expect(r"notes_cjk\.txt").expect(r"lib\.rs").reject(r"data\.bin")
    tui.quit()
