#!/usr/bin/env python3
# `ZZ` reports a failed save and stays open, like `:wq`, instead of quitting and losing the unsaved mark. Header:
# panel G6-2.
import os
from pathlib import Path

from e2e import Scenario, scenario


def chmod_tree(root: Path, add: int, remove: int, only: tuple[str, ...] = ()) -> None:
    dirs = [p for p, _, _ in os.walk(root) if not only or Path(p).name in only]
    for p in dirs:
        os.chmod(p, (os.stat(p).st_mode | add) & ~remove)


@scenario
def main(s: Scenario) -> None:
    if os.getuid() == 0:
        s.skip("root ignores directory permissions")
    tui = s.diff()
    d = s.dir
    tui.keys("r").wait_for("Files · 1/8")
    for top in (d / "data", d / "home"):
        chmod_tree(top, 0, 0o222, ("reviews", "sessions"))
    s.on_exit(chmod_tree, d, 0o200, 0)
    tui.keys("ZZ")
    tui.wait_for("Save failed")
    tui.keys("Escape", ":q!", "Enter").wait_exit()
