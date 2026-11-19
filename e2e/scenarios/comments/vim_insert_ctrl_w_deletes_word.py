#!/usr/bin/env python3
# Issue #822: with `comment_vim = true`, Ctrl-W in the Insert-mode comment editor deletes the word before the cursor.
from e2e import Scenario, repos, scenario


@scenario
def main(s: Scenario) -> None:
    repo = repos.git_repo(s.dir / "repo")
    repos.write(s.dir / "config" / "tuicr" / "config.toml", "comment_vim = true\n")
    tui = s.start("-r", "HEAD~2..HEAD", cwd=repo, keep=True)
    tui.wait_for("Files · 0/8").keys("Escape")
    tui.cursor_on("4 ▌ second line")
    tui.keys("c").wait_for("INSERT")  # vim editor active
    tui.keys("keepme dropme").wait_for("keepme dropme")
    tui.keys("C-w").wait_gone("dropme")
    tui.wait_for("keepme")
    tui.keys("Escape").keys("Enter").keys("Enter")  # Normal mode: Enter arms, a second Enter saves
    tui.keys("m").wait_for("keepme")  # jump to the saved comment
    tui.save()
    assert s.session_has("keepme"), f"comment not saved\n{s.session_json()}"
    assert not s.session_has("dropme"), f"Ctrl-W did not delete the word\n{s.session_json()}"
    tui.quit()
