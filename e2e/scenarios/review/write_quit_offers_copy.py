#!/usr/bin/env python3
# `:wq` saves the session and asks whether to copy the review; `n` quits without copying, `y` copies it then quits.
from e2e import Scenario, repos, scenario


@scenario
def main(s: Scenario) -> None:
    for answer in ("n", "y"):
        name = f"{s.name}_{answer}"
        d = s.path_of(name)
        repo = repos.git_repo(d / "repo")
        clip = s.fake_clipboard(name)
        tui = s.start("-r", "HEAD~2..HEAD", cwd=repo, name=name)
        tui.wait_for("Files · 0/8").keys("Escape")
        tui.cursor_on("4 ▌ second line")
        tui.keys("c").wait_for("COMMENT")
        tui.keys(f"note for {answer}").keys("Enter").wait_for(f"note for {answer}")
        tui.cmd("wq")
        tui.wait_for(r"Copy review to clipboard\?")
        assert tui.alive(), ":wq quit before the prompt was answered"
        assert s.session_has(f"note for {answer}", name), "session was not saved by :wq"
        tui.keys(answer)
        tui.wait_exit()
        if answer == "y":
            assert "note for y" in clip.text(), "review not copied"
        else:
            assert not (d / "clipboard.txt").exists(), "answering n still copied"
