#!/usr/bin/env python3
# Covers #827: config `copy_on_quit` skips the `:wq` prompt. "never" saves and quits without copying (`:wq` and `ZZ`);
# "always" copies the review and quits without asking.
from e2e import Scenario, repos, scenario


@scenario
def main(s: Scenario) -> None:
    for policy, keys in (("never", ":wq"), ("never", "ZZ"), ("always", ":wq")):
        name = f"{s.name}_{policy}_{keys.strip(':')}"
        d = s.path_of(name)
        repo = repos.git_repo(d / "repo")
        repos.write(d / "config" / "tuicr" / "config.toml", f'copy_on_quit = "{policy}"\n')
        clip = s.fake_clipboard(name)
        tui = s.start("-r", "HEAD~2..HEAD", cwd=repo, name=name, keep=True)
        tui.wait_for("Files · 0/8").keys("Escape")
        tui.cursor_on("4 ▌ second line")
        tui.comment(f"note for {name}")
        tui.keys("r").wait_for("Files · 1/8")  # the unsaved part the quit has to write
        if keys == ":wq":
            tui.cmd("wq")
        else:
            tui.keys("ZZ")
        tui.wait_exit()  # no "Copy review to clipboard?" prompt holds it open
        assert s.session_reviewed("README.md", name), f"{keys} did not save the session under {policy!r}"
        assert s.session_comments(name) == [f"note for {name}"], s.session_comments(name)
        if policy == "always":
            assert f"note for {name}" in clip.text(), "copy_on_quit = always did not copy the review"
        else:
            assert not clip.copied, f"copy_on_quit = never still copied on {keys}"
