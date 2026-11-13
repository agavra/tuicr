#!/usr/bin/env python3
"""install.sh prompt guards: each of the two conditions is exercised separately.

A. stdout is a tty but there is no controlling tty (/dev/tty unopenable)
   -> must skip the prompt and proceed to download.
B. controlling tty exists but stdout is not a tty -> must skip the prompt.
C. control: both available -> must prompt (so A/B are not vacuous).
"""
import os, pty, select, signal, stat, subprocess, sys, tempfile, time

root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
script = os.path.join(root, "install.sh")
tmp = tempfile.mkdtemp()
curl = os.path.join(tmp, "curl")
with open(curl, "w") as f:
    f.write('#!/bin/sh\necho reached-download\nexit 1\n')
os.chmod(curl, os.stat(curl).st_mode | stat.S_IEXEC)
env = {"PATH": tmp + ":/usr/bin:/bin", "HOME": tmp, "TUICR_VERSION": "0.0.0"}


def drain(fd, proc_done, timeout=10):
    out, end = b"", time.time() + timeout
    while time.time() < end:
        if select.select([fd], [], [], 0.2)[0]:
            try:
                d = os.read(fd, 4096)
            except OSError:
                break
            if not d:
                break
            out += d
        elif proc_done():
            break
    return out.decode(errors="replace")


def run_pty(cmd, stdin_data=None):
    """Run cmd as session leader with the pty as controlling tty."""
    pid, fd = pty.fork()
    if pid == 0:
        os.execve("/bin/sh", ["/bin/sh", "-c", cmd], env)
    if stdin_data:
        time.sleep(1)
        os.write(fd, stdin_data)
    def done():
        return os.waitpid(pid, os.WNOHANG)[0] != 0
    out = drain(fd, lambda: False, 5)
    try:
        os.kill(pid, signal.SIGKILL)
    except OSError:
        pass
    try:
        os.waitpid(pid, 0)
    except OSError:
        pass
    return out


def check(name, cond, out):
    if not cond:
        sys.exit("FAIL: %s\n%s" % (name, out))


# A: stdout is a pty slave, new session => no controlling tty.
m, s = pty.openpty()
p = subprocess.Popen(["/bin/sh", script], env=env, stdin=subprocess.DEVNULL,
                     stdout=s, stderr=s, start_new_session=True)
os.close(s)
outA = drain(m, lambda: p.poll() is not None)
p.wait()
check("A: tty-open guard (must reach download without prompt)",
      "reached-download" in outA and "Continue?" not in outA, outA)

# B: controlling tty present, stdout redirected to a file.
of = os.path.join(tmp, "outB")
run_pty("/bin/sh %s >%s 2>&1" % (script, of))
outB = open(of).read()
check("B: stdout-tty guard (must reach download without prompt)",
      "reached-download" in outB and "Continue?" not in outB, outB)

# C: control, both available => prompts; 'n' aborts before download.
outC = run_pty("/bin/sh %s" % script, b"n\n")
check("C: control prompt", "Continue?" in outC and "reached-download" not in outC, outC)
print("ok")
