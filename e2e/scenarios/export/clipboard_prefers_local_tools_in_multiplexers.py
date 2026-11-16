#!/usr/bin/env python3
# Covers #894 / #170: local clipboard tools win in tmux/Zellij; clipboard = ["osc52"] forces the relay.
from e2e import Scenario, repos, scenario, wait_file


@scenario
def main(s: Scenario) -> None:
    repo = repos.git_repo(s.dir / "repo")
    expected = None
    for mux, override in (("TMUX", False), ("ZELLIJ", False), ("TMUX", True)):
        name = f"{s.name}_{mux}_{override}"
        d = s.path_of(name)
        clipboard = s.fake_clipboard(name)
        relay = d / "relay.txt"
        tool = repos.write(d / "clipbin/tmux", f'#!/bin/sh\ncat > "{relay}"\n')
        tool.chmod(0o755)
        if override:
            repos.write(d / "config/tuicr/config.toml", 'clipboard = ["osc52"]\n')
        tui = s.start("-r", "HEAD~2..HEAD", cwd=repo, name=name, keep=True, env={mux: "e2e"})
        tui.wait_for("Files · 0/8").keys("Escape")
        tui.cursor_on("4 ▌ second line").comment("clipboard ordering")
        tui.cmd("clip").wait_for("Review copied to clipboard")
        if override:
            assert wait_file(relay).read_text() == expected, "relay did not receive the complete export"
            assert not clipboard.copied, "OSC 52 override still invoked a local tool"
        else:
            actual = clipboard.text()
            assert "`README.md:4` - clipboard ordering" in actual, actual
            assert not relay.exists(), "relay ran before the local tool"
            if expected is not None:
                assert actual == expected, "multiplexer changed the export"
            expected = actual
        tui.quit()
