"""Polling waits on state outside the screen: files, commands. Each polls every 100 ms and fails with evidence."""

from __future__ import annotations

import subprocess
import time
from collections.abc import Callable
from pathlib import Path
from typing import cast

from .core import TIMEOUT, Failure, grep


def polls(timeout: float, step: float) -> int:
    return max(1, int(timeout / step + 0.5))


def wait_file(path: str | Path, timeout: float = TIMEOUT) -> Path:
    """A non-empty file appears (e2e_wait_file). Returns the path."""
    p = Path(path)
    for _ in range(polls(timeout, 0.1)):
        if p.exists() and p.stat().st_size > 0:
            return p
        time.sleep(0.1)
    raise Failure(f"{p} did not appear within {timeout:g}s")


def wait_grep(path: str | Path, pattern: str, timeout: float = TIMEOUT) -> Path:
    """The file exists and a line matches the regex (e2e_wait_grep)."""
    p = Path(path)
    for _ in range(polls(timeout, 0.1)):
        if p.exists() and grep(pattern, p.read_text(errors="replace")):
            return p
        time.sleep(0.1)
    raise Failure(
        f"no line matching /{pattern}/ in {p} within {timeout:g}s",
        p.read_text(errors="replace") if p.exists() else None,
    )


def wait_cmd(timeout: float, check: list[str] | Callable[[], object]) -> None:
    """Poll until the command (an argv list, output discarded) exits 0, or the callable returns truthy."""
    for _ in range(polls(timeout, 0.1)):
        if isinstance(check, list):
            argv = cast("list[str]", check)  # ty narrows `list | Callable` imprecisely after isinstance
            p = subprocess.run(argv, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            ok = p.returncode == 0
        else:
            ok = bool(check())
        if ok:
            return
        time.sleep(0.1)
    what = getattr(check, "__name__", None) if callable(check) else " ".join(check)
    raise Failure(f"condition never held within {timeout:g}s: {what}")
