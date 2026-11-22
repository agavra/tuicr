#!/usr/bin/env python3
# Covers #548 / #787: `minimal_ui = true` drops the brand header and the idle footer and opens on the first file, so
# with the file list and commit pane hidden only the diff is on screen. `:set nominimal` brings the chrome back and
# `:set minimal!` hides it again; with the file list open, minimal mode still leaves the diff panel unframed.
from e2e import Scenario, repos, scenario

FILE_HEADER = r"═══ README\.md \[M\]"


@scenario
def main(s: Scenario) -> None:
    repo = repos.git_repo(s.dir / "repo")
    repos.write(
        s.dir / "config" / "tuicr" / "config.toml",
        "minimal_ui = true\nshow_file_list = false\nshow_commits = false\n",
    )
    tui = s.start("-r", "HEAD~2..HEAD", cwd=repo, keep=True)
    tui.wait_for(FILE_HEADER).settle(0.4)

    rows = tui.screen().split("\n")
    assert rows[0].lstrip().startswith("▶ ═══ README.md [M]"), f"minimal UI does not open on the first file\n{rows[0]}"
    assert rows[-1].strip() == "", f"idle footer is not blank\n{rows[-1]}"
    tui.reject(r"^ tuicr ").reject(r"git:main · 2 commits")  # no brand header
    tui.reject(r"^ NORMAL ").reject(r"^│")  # no footer, no frame
    tui.reject(r"Files · ").reject(r"┌ Commits")  # diff only

    tui.cmd("set nominimal").wait_for(r"^ tuicr ").wait_for(r"^ NORMAL ")

    tui.cmd("set minimal!").wait_gone(r"^ tuicr ").wait_gone(r"^ NORMAL ").wait_for(FILE_HEADER)

    tui.keys(";e").wait_for("Files · 0/8").settle(0.4)  # file list open: the diff panel stays unframed
    tui.reject(r"┌ README\.md")
    tui.cmd("set nominimal").wait_for(r"┌ README\.md").wait_for(r"^ tuicr ")
    tui.quit()
