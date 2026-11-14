#!/usr/bin/env python3
# `ZZ` saves the unsaved reviewed mark, copies the review and quits; `ZQ` quits without copying and without saving it.
from e2e import Scenario, repos, scenario


@scenario
def main(s: Scenario) -> None:
    for k in ("ZZ", "ZQ"):
        name = f"{s.name}_{k}"
        d = s.path_of(name)
        repo = repos.git_repo(d / "repo")
        clip = s.fake_clipboard(name)
        tui = s.start("-r", "HEAD~2..HEAD", cwd=repo, name=name)
        tui.wait_for("Files · 0/8").keys("Escape")
        tui.cursor_on("4 ▌ second line")
        tui.keys("c").wait_for("COMMENT")
        tui.keys(f"note for {k}").keys("Enter").wait_for(f"note for {k}")
        tui.keys("r").wait_for("Files · 1/8")  # the comment autosaved; the mark is the unsaved part
        tui.keys(k)
        tui.wait_exit()
        if k == "ZZ":
            assert s.session_has('"reviewed": true', name), "ZZ did not save the reviewed mark"
            assert "note for ZZ" in clip.text(), "ZZ did not copy the review"
        else:
            assert not s.session_has('"reviewed": true', name), "ZQ saved the reviewed mark"
            assert not (d / "clipboard.txt").exists(), "ZQ copied the review"
