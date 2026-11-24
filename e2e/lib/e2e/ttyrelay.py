"""A terminal between the tmux pane and a command, logging how the command's frames arrive.

`argv(log, *cmd)` is the command line for Scenario.start_cmd. The command runs on a pty of its own (its controlling
terminal, so `/dev/tty` is that pty too) the size of the pane; keys pass through and output is copied to the pane, so
the Tui drives it as usual. Every synchronized update the command draws (`CSI ?2026h` ... `CSI ?2026l`) is logged as
one JSON line: its size in bytes and how many reads of the pty it took. `frames(log)` reads them back. With
`da1_only=True` the pty also answers the startup keyboard probe the way tmux does (a Primary Device Attributes reply,
no kitty flags), and the command's terminal-control requests are logged for `controls(log)`. Stdlib only: this file
runs as a script in the pane.
"""

from __future__ import annotations

import bisect
import fcntl
import json
import os
import pty
import select
import sys
import termios
import tty
from pathlib import Path
from typing import TextIO

OPEN, CLOSE = b"\x1b[?2026h", b"\x1b[?2026l"
PROBE = b"\x1b[?u\x1b[c"
DA1_ONLY = b"\x1b[?1;2c"
CONTROLS = {b"\x1b[>4;1m": "modify-other-keys:1", b"\x1b[>4;0m": "modify-other-keys:0"}


def argv(log: str | os.PathLike[str], *cmd: str, da1_only: bool = False) -> list[str]:
    """`cmd` behind the relay, logging its frames to `log`. `da1_only` answers the keyboard probe like tmux."""
    return [sys.executable, __file__, *(["--da1-only"] if da1_only else []), os.fspath(log), *cmd]


def frames(log: str | os.PathLike[str]) -> list[dict[str, int]]:
    """The frames logged so far, oldest first: {"bytes": ..., "reads": ...}."""
    path = Path(log)
    if not path.exists():
        return []
    return [e for e in map(json.loads, path.read_text().splitlines()) if "bytes" in e]


def controls(log: str | os.PathLike[str]) -> list[str]:
    """The terminal-control requests logged so far, oldest first, e.g. "modify-other-keys:1"."""
    path = Path(log)
    if not path.exists():
        return []
    return [e["control"] for e in map(json.loads, path.read_text().splitlines()) if "control" in e]


def _log_frames(stream: bytearray, ends: list[int], done: int, log: TextIO) -> int:
    """Log each frame completed in `stream` after offset `done`; return the offset after the last one logged.
    `ends[i]` is the offset just past read i, so a frame's reads are those holding its first through last byte."""
    while (start := stream.find(OPEN, done)) >= 0 and (close := stream.find(CLOSE, start)) >= 0:
        end = close + len(CLOSE)
        reads = bisect.bisect_right(ends, end - 1) - bisect.bisect_right(ends, start) + 1
        print(json.dumps({"bytes": end - start, "reads": reads}), file=log, flush=True)
        done = end
    return done


def _write_all(fd: int, data: bytes) -> None:
    while data:
        data = data[os.write(fd, data) :]


class Terminal:
    """The command's terminal: answers the keyboard probe like tmux and logs its control requests."""

    def __init__(self, da1_only: bool, log: TextIO) -> None:
        self.da1_only = da1_only
        self.log = log
        self.carry = b""
        self.probe_answered = False
        self.seen: set[bytes] = set()

    def observe(self, data: bytes, master: int) -> None:
        """Consume the command's output: answer the startup probe and log each control request once."""
        window = self.carry + data
        if self.da1_only and not self.probe_answered and PROBE in window:
            _write_all(master, DA1_ONLY)
            self.probe_answered = True
            print(json.dumps({"control": "probe-answered"}), file=self.log, flush=True)
        for seq, name in CONTROLS.items():
            if seq in window and seq not in self.seen:
                self.seen.add(seq)
                print(json.dumps({"control": name}), file=self.log, flush=True)
        self.carry = window[-max(len(PROBE), *(len(s) for s in CONTROLS)) :]


def main(log_path: str, cmd: list[str], da1_only: bool = False) -> int:
    size = fcntl.ioctl(0, termios.TIOCGWINSZ, b"\0" * 8)
    pid, master = pty.fork()
    if pid == 0:
        fcntl.ioctl(0, termios.TIOCSWINSZ, size)
        os.execvp(cmd[0], cmd)
    saved = termios.tcgetattr(0)
    tty.setraw(0)
    stream, ends, done = bytearray(), list[int](), 0
    inputs = [master, 0]
    try:
        with open(log_path, "a") as log:
            terminal = Terminal(da1_only, log)
            while True:
                ready, _, _ = select.select(inputs, [], [])
                if master in ready:
                    try:
                        data = os.read(master, 1 << 16)
                    except OSError:  # EIO once the command has exited (Linux)
                        data = b""
                    if not data:
                        break
                    stream += data
                    ends.append(len(stream))
                    terminal.observe(data, master)
                    _write_all(1, data)
                    done = _log_frames(stream, ends, done, log)
                if 0 in ready:
                    keys = os.read(0, 1 << 12)
                    if keys:
                        _write_all(master, keys)
                    else:
                        inputs.remove(0)
    finally:
        termios.tcsetattr(0, termios.TCSADRAIN, saved)
    return os.waitstatus_to_exitcode(os.waitpid(pid, 0)[1])


if __name__ == "__main__":
    args = sys.argv[1:]
    da1_only = bool(args and args[0] == "--da1-only")
    if da1_only:
        args = args[1:]
    sys.exit(main(args[0], args[1:], da1_only))
