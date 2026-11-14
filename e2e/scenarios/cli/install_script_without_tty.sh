#!/usr/bin/env bash
set -euo pipefail
. "$(dirname "$0")/../../lib/init.sh"
# site install.sh must not prompt (and so not abort) when it cannot actually talk to a terminal:
#  A. stdout is a tty but the process has no controlling tty (setsid) -> `read </dev/tty` used to fail under set -e
#  B. controlling tty exists but stdout is redirected -> prompt text went to a file, nobody could answer
# Runs install.sh against a fake release dir, installing into a temp prefix only.
# install.sh is not in main's tree: it lives on the `site` branch. Source, in order: $E2E_INSTALL_SH (a file), or
# `git show $E2E_INSTALL_REV:install.sh` (default upstream/site, then origin/site); SKIP when none resolves.
n=$(e2e_name); d=$(e2e_dir $n); rm -rf "$d"; mkdir -p "$d/rel" "$d/bin" "$d/home"
repo=$(cd "$(dirname "$0")/../../.." && pwd)
if [ -n "${E2E_INSTALL_SH:-}" ]; then cp "$E2E_INSTALL_SH" "$d/install.sh"
else
  found=
  for rev in ${E2E_INSTALL_REV:-upstream/site origin/site}; do
    git -C "$repo" show "$rev:install.sh" > "$d/install.sh" 2>/dev/null && { found=1; break; }
  done
  [ -n "$found" ] || e2e_skip "install.sh not available (set E2E_INSTALL_SH or fetch the site branch)"
fi
case $(uname -s) in Darwin) tos=apple-darwin;; *) tos=unknown-linux-gnu;; esac
case $(uname -m) in arm64|aarch64) ta=aarch64;; *) ta=x86_64;; esac
ver=9.9.9; arch=tuicr-$ver-$ta-$tos.tar.gz
mkdir -p "$d/stage"; printf '#!/bin/sh\necho fake tuicr\n' > "$d/stage/tuicr"; chmod +x "$d/stage/tuicr"
tar -czf "$d/rel/$arch" -C "$d/stage" tuicr
# fake curl: serves $d/rel/<basename of url>; nothing touches the network
cat > "$d/bin/curl" <<SH
#!/bin/sh
out=; while [ \$# -gt 1 ]; do [ "\$1" = -o ] && out=\$2; shift; done
f=$d/rel/\$(basename "\$1"); [ -f "\$f" ] || { echo "fake curl: no \$1" >&2; exit 22; }
if [ -n "\$out" ]; then cp "\$f" "\$out"; else cat "\$f"; fi
SH
chmod +x "$d/bin/curl"
export D="$d" ARCH="$arch"
python3 - <<'PY'
import os, pty, select, signal, subprocess, sys, time, shutil
d = os.environ["D"]; arch = os.environ["ARCH"]
fail = []
def env(prefix):
    return {"PATH": d + "/bin:/usr/bin:/bin", "HOME": d + "/home", "TUICR_VERSION": "9.9.9",
            "TUICR_INSTALL_DIR": prefix}   # TUICR_INSTALL_YES deliberately unset
def drain(fd, p, timeout):
    out, end = b"", time.time() + timeout
    while time.time() < end:
        if select.select([fd], [], [], 0.2)[0]:
            try: x = os.read(fd, 4096)
            except OSError: break
            if not x: break
            out += x
        elif p.poll() is not None: break
    return out.decode(errors="replace")
def result(name, out, rc, prefix):
    ok = rc == 0 and os.access(prefix + "/tuicr", os.X_OK)
    print("%s: rc=%s installed=%s\n%s" % (name, rc, os.path.exists(prefix + "/tuicr"), out.strip()))
    if not ok: fail.append(name)
# A: stdout = pty slave, new session => no controlling tty
prefix = d + "/prefixA"
m, s = pty.openpty()
p = subprocess.Popen(["/bin/sh", d + "/install.sh"], env=env(prefix), stdin=subprocess.DEVNULL, stdout=s, stderr=s, start_new_session=True)
os.close(s); out = drain(m, p, 8)
if p.poll() is None: p.kill(); out += "\n[hung]"
p.wait(); result("A no controlling tty (setsid)", out, p.returncode, prefix)
# B: controlling tty, stdout redirected to a file
prefix = d + "/prefixB"; of = d + "/outB"
pid, fd = pty.fork()
if pid == 0:
    os.execve("/bin/sh", ["/bin/sh", "-c", "/bin/sh %s/install.sh >%s 2>&1" % (d, of)], env(prefix))
t = time.time(); rc = None
while time.time() - t < 8:
    try:
        if select.select([fd], [], [], 0.2)[0]: os.read(fd, 4096)
    except OSError: pass
    w = os.waitpid(pid, os.WNOHANG)
    if w[0]: rc = os.waitstatus_to_exitcode(w[1]); break
if rc is None:
    os.kill(pid, signal.SIGKILL); os.waitpid(pid, 0); rc = "hung"
out = open(of).read() if os.path.exists(of) else ""
result("B stdout redirected", out, rc, prefix)
sys.exit(1 if fail else 0)
PY
