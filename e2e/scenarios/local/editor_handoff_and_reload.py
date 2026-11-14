#!/usr/bin/env python3
# `e` hands the file at the cursor's line to $EDITOR. A terminal editor (vim family) takes the screen and the diff
# reloads
# when it exits; a windowed one (code) is launched with --goto and the diff only picks the edit up on `:e`.
# The "editors" are stubs that record their arguments and append a line to the file.
from e2e import Scenario, Tui, grep, repos, scenario, wait_file


@scenario
def main(s: Scenario) -> None:
    d = s.dir
    repo = repos.git_repo(d / "repo")
    for ed in ("vim", "code"):
        stub = repos.write(
            d / "ed" / ed,
            f'#!/bin/sh\nprintf "%s\\n" "$*" > "{d}/ed/{ed}.args"\n'
            f'for last; do :; done\nf=${{last%:*}}\nprintf "edited by {ed}\\n" >> "$f"\n',
        )
        stub.chmod(0o755)

    def open_ed(editor: str, name: str | None = None) -> Tui:
        tui = s.start("-w", cwd=repo, env={"EDITOR": f"{d}/ed/{editor}"}, name=name)
        tui.wait_for("unstaged line").keys("Escape").cursor_on("unstaged line")
        return tui

    tui = open_ed("vim")
    tui.keys("e")
    tui.wait_for("edited by vim")  # the diff was reloaded after the editor exited
    args = (d / "ed/vim.args").read_bytes().decode()
    assert grep(r"^\+5 .*/README\.md$", args), f"vim got: {args}"
    tui.quit()
    win = open_ed("code", f"{s.name}_windowed")
    win.keys("e")
    wait_file(d / "ed/code.args")
    args = (d / "ed/code.args").read_bytes().decode()
    assert grep(r"^--goto .*/README\.md:5$", args), f"code got: {args}"
    win.reject("edited by code", 1)  # a windowed editor does not trigger a reload
    win.cmd("e").wait_for("edited by code")  # `:e` does
    win.quit()
