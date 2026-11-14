"""Shared flows: open a review, a fake clipboard, a minimal PATH."""
from __future__ import annotations

import os
import stat
from pathlib import Path
from typing import Optional

from . import repos
from .tui import Tui
from .waits import wait_file


def open_review(s, *args: str, **start_kw) -> Tui:
    """Fixture git repo (<scratch>/repo), tuicr started in it. Default args: -r HEAD~2..HEAD (8 files, the inline
    commit selector focused). Extra keyword arguments go to Scenario.start (env, cols, ...)."""
    repo = repos.git_repo(s.dir / "repo")
    start_kw.setdefault("cwd", repo)
    return s.start(*(args or ("-r", "HEAD~2..HEAD")), **start_kw)


def diff(s, **start_kw) -> Tui:
    """Open the 2-commit range and move focus from the commit selector to the diff."""
    return open_review(s, **start_kw).wait_for("Files · 0/8").keys("Escape")


class FakeClipboard:
    """pbcopy, xclip and wl-copy that record stdin, first on the child's PATH, so a scenario neither touches the
    developer's clipboard nor depends on one existing. Create it BEFORE starting tuicr; read what was copied with
    `text()` (waits for it) and check `copied` for "nothing was copied"."""

    def __init__(self, s, name: Optional[str] = None):
        d = s.path_of(name or s.name)
        self.path = d / "clipboard.txt"
        bindir = d / "clipbin"
        bindir.mkdir(parents=True, exist_ok=True)
        if self.path.exists():
            self.path.unlink()
        for b in ("pbcopy", "xclip", "wl-copy"):
            f = bindir / b
            f.write_text(f'#!/bin/sh\ncat > "{self.path}"\n')
            f.chmod(f.stat().st_mode | stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH)
        s.pre_path.insert(0, str(bindir))
        s.env["XDG_SESSION_TYPE"] = "x11"

    @property
    def copied(self) -> bool:
        """Something was copied already (the file exists)."""
        return self.path.exists()

    def text(self, timeout: float = 10) -> str:
        """Wait for, then return, what the fake clipboard received."""
        return wait_file(self.path, timeout).read_text()


def minimal_path(directory, *tools: str) -> str:
    """A PATH directory holding only the named tools (symlinks), for scenarios where the absence of a tool (az, gh, ...)
    must be deterministic rather than depend on the machine. Returns the directory; pass it as `path=` to Scenario.cli."""
    import shutil
    directory = Path(directory)
    directory.mkdir(parents=True, exist_ok=True)
    for t in tools:
        p = shutil.which(t)
        if not p:
            raise repos.Failure(f"e2e: {t} not found")
        link = directory / t
        if link.is_symlink() or link.exists():
            link.unlink()
        os.symlink(p, link)
    return str(directory)
