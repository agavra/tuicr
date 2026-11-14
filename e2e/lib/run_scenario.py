#!/usr/bin/env python3
"""run_scenario.py <timeout-seconds> <command...>: run a scenario in its own session/process group.

On timeout, or when the runner is interrupted, the whole process tree is killed (the scenario's backgrounded
fake servers included); after a normal exit leftovers are reaped too. Exit status: the scenario's, 124 on
timeout, 130 when interrupted, 128+N when killed by signal N.
"""

from __future__ import annotations

import contextlib
import os
import signal
import subprocess
import sys
from types import FrameType

timeout, cmd = float(sys.argv[1]), sys.argv[2:]
proc = subprocess.Popen(cmd, start_new_session=True)


def kill_group() -> None:
    with contextlib.suppress(ProcessLookupError, PermissionError):
        os.killpg(proc.pid, signal.SIGKILL)


def interrupted(signum: int, frame: FrameType | None) -> None:
    kill_group()
    proc.wait()
    sys.exit(130)


signal.signal(signal.SIGINT, interrupted)
signal.signal(signal.SIGTERM, interrupted)
try:
    rc = proc.wait(timeout)
    if rc < 0:
        rc = 128 - rc
except subprocess.TimeoutExpired:
    sys.stderr.write(f"e2e: scenario exceeded {timeout:g}s and was killed (process tree included)\n")
    kill_group()
    proc.wait()
    rc = 124
kill_group()
sys.exit(rc)
