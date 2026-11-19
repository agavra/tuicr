"""A terminal between the tmux pane and a command, logging how the command's frames arrive.

`argv(log, *cmd)` is the command line for Scenario.start_cmd. The command runs on a pty of its own (its controlling
terminal, so `/dev/tty` is that pty too) the size of the pane; keys pass through and output is copied to the pane, so
the Tui drives it as usual. Every synchronized update the command draws (`CSI ?2026h` ... `CSI ?2026l`) is logged as
one JSON line: its size in bytes and how many reads of the pty it took. `frames(log)` reads them back. Stdlib only:
this file runs as a script in the pane.
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


def argv(log: str | os.PathLike[str], *cmd: str) -> list[str]:
    """`cmd` behind the relay, logging its frames to `log`."""
    return [sys.executable, __file__, os.fspath(log), *cmd]


def frames(log: str | os.PathLike[str]) -> list[dict[str, int]]:
    """The frames logged so far, oldest first: {"bytes": ..., "reads": ...}."""
    path = Path(log)
    if not path.exists():
        return []
    return [json.loads(line) for line in path.read_text().splitlines()]


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


def main(log_path: str, cmd: list[str]) -> int:
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
    sys.exit(main(sys.argv[1], sys.argv[2:]))
