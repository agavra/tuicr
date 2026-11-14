#!/usr/bin/env python3
# site install.sh must not prompt (and so not abort) when it cannot actually talk to a terminal:
#  A. stdout is a tty but the process has no controlling tty (setsid) -> `read </dev/tty` used to fail under set -e
#  B. controlling tty exists but stdout is redirected -> prompt text went to a file, nobody could answer
# Runs install.sh against a fake release dir, installing into a temp prefix only.
# install.sh is not in main's tree: it lives on the `site` branch. Source, in order: $E2E_INSTALL_SH (a file), or
# `git show $E2E_INSTALL_REV:install.sh` (default upstream/site, then origin/site); SKIP when none resolves.
from __future__ import annotations

import os
import platform
import pty
import select
import shutil
import signal
import subprocess
import tarfile
import time
from pathlib import Path

from e2e import E2E_DIR, Scenario, repos, scenario


def drain(fd: int, p: subprocess.Popen[bytes], timeout: float) -> str:
    out, end = b"", time.time() + timeout
    while time.time() < end:
        if select.select([fd], [], [], 0.2)[0]:
            try:
                x = os.read(fd, 4096)
            except OSError:
                break
            if not x:
                break
            out += x
        elif p.poll() is not None:
            break
    return out.decode(errors="replace")


@scenario
def main(s: Scenario) -> None:
    d = repos.fresh(s.dir)
    for sub in ("rel", "bin", "home", "stage"):
        (d / sub).mkdir()
    install = d / "install.sh"
    if os.environ.get("E2E_INSTALL_SH"):
        shutil.copy(os.environ["E2E_INSTALL_SH"], install)
    else:
        found = False
        for rev in os.environ.get("E2E_INSTALL_REV", "upstream/site origin/site").split():
            p = subprocess.run(
                ["git", "-C", str(E2E_DIR.parent), "show", f"{rev}:install.sh"],
                stdin=subprocess.DEVNULL,
                stdout=subprocess.PIPE,
                stderr=subprocess.DEVNULL,
            )
            if p.returncode == 0:
                install.write_bytes(p.stdout)
                found = True
                break
        if not found:
            s.skip("install.sh not available (set E2E_INSTALL_SH or fetch the site branch)")
    tos = "apple-darwin" if platform.system() == "Darwin" else "unknown-linux-gnu"
    ta = "aarch64" if platform.machine() in ("arm64", "aarch64") else "x86_64"
    arch = f"tuicr-9.9.9-{ta}-{tos}.tar.gz"
    fake = repos.write(d / "stage/tuicr", "#!/bin/sh\necho fake tuicr\n")
    fake.chmod(0o755)
    with tarfile.open(d / "rel" / arch, "w:gz") as tar:
        tar.add(fake, arcname="tuicr")
    # fake curl: serves $d/rel/<basename of url>; nothing touches the network
    curl = repos.write(
        d / "bin/curl",
        "#!/bin/sh\n"
        'out=; while [ $# -gt 1 ]; do [ "$1" = -o ] && out=$2; shift; done\n'
        f'f={d}/rel/$(basename "$1"); [ -f "$f" ] || {{ echo "fake curl: no $1" >&2; exit 22; }}\n'
        'if [ -n "$out" ]; then cp "$f" "$out"; else cat "$f"; fi\n',
    )
    curl.chmod(0o755)
    fail: list[str] = []

    def env(prefix: Path) -> dict[str, str]:
        # TUICR_INSTALL_YES deliberately unset
        return {
            "PATH": f"{d}/bin:/usr/bin:/bin",
            "HOME": f"{d}/home",
            "TUICR_VERSION": "9.9.9",
            "TUICR_INSTALL_DIR": str(prefix),
        }

    def result(name: str, out: str, rc: int | str | None, prefix: Path) -> None:
        ok = rc == 0 and os.access(prefix / "tuicr", os.X_OK)
        print(f"{name}: rc={rc} installed={(prefix / 'tuicr').exists()}\n{out.strip()}")
        if not ok:
            fail.append(name)

    # A: stdout = pty slave, new session => no controlling tty
    prefix = d / "prefixA"
    m, sl = pty.openpty()
    p = subprocess.Popen(
        ["/bin/sh", str(install)],
        env=env(prefix),
        stdin=subprocess.DEVNULL,
        stdout=sl,
        stderr=sl,
        start_new_session=True,
    )
    os.close(sl)
    out = drain(m, p, 8)
    if p.poll() is None:
        p.kill()
        out += "\n[hung]"
    p.wait()
    os.close(m)
    result("A no controlling tty (setsid)", out, p.returncode, prefix)
    # B: controlling tty, stdout redirected to a file
    prefix = d / "prefixB"
    of = d / "outB"
    pid, fd = pty.fork()
    if pid == 0:
        os.execve("/bin/sh", ["/bin/sh", "-c", f"/bin/sh {install} >{of} 2>&1"], env(prefix))
    t = time.time()
    rc: int | str | None = None
    while time.time() - t < 8:
        try:
            if select.select([fd], [], [], 0.2)[0]:
                os.read(fd, 4096)
        except OSError:
            pass
        w = os.waitpid(pid, os.WNOHANG)
        if w[0]:
            rc = os.waitstatus_to_exitcode(w[1])
            break
    if rc is None:
        os.kill(pid, signal.SIGKILL)
        os.waitpid(pid, 0)
        rc = "hung"
    os.close(fd)
    result("B stdout redirected", of.read_text() if of.exists() else "", rc, prefix)
    assert not fail, f"install.sh failed in: {', '.join(fail)}"
