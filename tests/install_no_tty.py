#!/usr/bin/env python3
"""install.sh must not abort at the prompt when /dev/tty cannot be opened."""
import os, stat, subprocess, sys, tempfile

root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
with tempfile.TemporaryDirectory() as tmp:
    curl = os.path.join(tmp, "curl")
    with open(curl, "w") as f:
        f.write('#!/bin/sh\necho reached-download\nexit 1\n')
    os.chmod(curl, os.stat(curl).st_mode | stat.S_IEXEC)
    env = {"PATH": tmp + ":/usr/bin:/bin", "HOME": tmp, "TUICR_VERSION": "0.0.0"}
    # New session: no controlling terminal, so /dev/tty exists but cannot be opened.
    r = subprocess.run(["/bin/sh", os.path.join(root, "install.sh")], env=env,
                       stdin=subprocess.DEVNULL, capture_output=True, text=True,
                       start_new_session=True)
    if "reached-download" not in r.stdout:
        sys.exit("FAIL: installer did not reach download\n" + r.stdout + r.stderr)
print("ok")
