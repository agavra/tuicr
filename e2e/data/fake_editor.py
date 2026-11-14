#!/usr/bin/env python3
"""Fake $EDITOR for the PR snapshot scenarios: records what tuicr handed over, then exits.

Writes editor.argv next to this script: ARGV, FILE (the realpath of the snapshot), one DIR line per directory from
the snapshot root's `snapshots` component down (mode, owner, our uid) and FILEMODE.
"""

import os
import stat
import sys

out = os.path.join(os.path.dirname(os.path.abspath(__file__)), "editor.argv")
p = sys.argv[-1] if os.path.exists(sys.argv[-1]) else next((a for a in sys.argv[1:] if os.path.exists(a)), "")
with open(out, "w") as f:
    f.write("ARGV " + " ".join(sys.argv[1:]) + "\n")
    f.write("FILE " + os.path.realpath(p) + "\n")
    rp = os.path.realpath(p)
    parts = rp.split(os.sep)
    i = max(k for k, x in enumerate(parts) if x == "snapshots")
    for k in range(i, len(parts) - 1):
        q = os.sep.join(parts[: k + 1])
        st = os.stat(q)
        f.write(f"DIR {q} {stat.S_IMODE(st.st_mode):o} uid={st.st_uid} me={os.getuid()}\n")
    f.write(f"FILEMODE {stat.S_IMODE(os.stat(rp).st_mode):o}\n")
