#!/usr/bin/env python3
# config.toml: `file_list_position = "right"` puts the file list right of the diff, `leader = ","` replaces `;` for the sidebar
# toggle, `q_quits = true` makes a bare `q` quit; `show_file_list = false` starts with the list hidden.
from e2e import Scenario, repos, scenario


@scenario
def main(s: Scenario) -> None:
    repo = repos.git_repo(s.dir / "repo")
    repos.write(
        s.dir / "config" / "tuicr" / "config.toml",
        'file_list_position = "right"\nleader = ","\nq_quits = true\n',
    )
    tui = s.start("-r", "HEAD~2..HEAD", cwd=repo, keep=True)
    tui.wait_for("Files · 0/8").keys("Escape")
    scr = tui.screen()
    line = next(ln for ln in scr.split("\n") if "Files · 0/8" in ln)
    col = line.index("Files · 0/8") + 1
    assert col > 100, f"file list is not on the right (column {col})\n{scr}"
    tui.keys(",e").wait_gone("Files · 0/8")  # the configured one hides the list
    tui.keys(",e").wait_for("Files · 0/8")
    tui.keys("q").wait_exit()  # bare q quits
    hidden = s.name + "_hidden"
    repos.write(
        s.path_of(hidden) / "config" / "tuicr" / "config.toml",
        "show_file_list = false\n",
    )
    tui2 = s.start("-r", "HEAD~2..HEAD", name=hidden, cwd=repo, keep=True)
    tui2.wait_for("second line").reject("Files · 0/8")
    tui2.keys(";e").wait_for("Files · 0/8")
    tui2.keys("q").wait_for("q no longer quits")  # q_quits is off here: a hint, no quit
    assert tui2.alive(), "q quit with q_quits unset"
    tui2.quit()
