#!/usr/bin/env python3
# Blank/whitespace-only clipboard_command warns and falls back to normal clipboard tools instead of running `sh -c ""`.
from e2e import Scenario, repos, scenario


@scenario
def main(s: Scenario) -> None:
    repo = repos.git_repo(s.dir / "repo")
    for mode, raw in (("empty", '""'), ("whitespace", '"   "')):
        name = f"{s.name}_{mode}"
        d = s.path_of(name)
        clipboard = s.fake_clipboard(name)
        repos.write(d / "config/tuicr/config.toml", f"clipboard_command = {raw}\n")
        tui = s.start("-r", "HEAD~2..HEAD", cwd=repo, name=name, keep=True)
        tui.wait_for("Files · 0/8").keys("Escape")
        tui.cursor_on("4 ▌ second line").comment("blank clipboard command fallback")
        tui.cmd("clip").wait_for("Review copied to clipboard")
        assert clipboard.copied, f"{mode}: blank clipboard_command skipped fallback clipboard"
        actual = clipboard.text()
        assert "`README.md:4` - blank clipboard command fallback" in actual, actual
        tui.quit()
