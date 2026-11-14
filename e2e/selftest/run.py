#!/usr/bin/env python3
"""e2e/selftest/run.py - tests of the harness itself: every way the runner could report a false green must be
caught here. Needs only tmux, python3 and git (TUICR_BIN is never started; it defaults to `env`).

    e2e/selftest/run.py
"""

from __future__ import annotations

import os
import re
import shutil
import signal
import subprocess
import sys
import tempfile
from pathlib import Path
from typing import IO

here = Path(__file__).resolve().parent
run_sh = here / ".." / "run.sh"
root = Path(tempfile.mkdtemp(prefix="tuicr-e2e-selftest.", dir=os.environ.get("TMPDIR", "/tmp")))
socket = f"st-{os.getpid()}"
failed = False


def check(label: str, expected: object, got: object) -> None:
    global failed
    if expected == got:
        print(f"ok   {label} ({got})")
    else:
        print(f"FAIL {label}: expected {expected}, got {got}")
        failed = True


def run(extra_env: dict[str, str], *args: str, out: Path | None = None) -> int:
    """Run e2e/run.sh with the selftest environment plus extra_env; returns its exit status."""
    env = dict(
        os.environ,
        TUICR_BIN=os.environ.get("TUICR_BIN") or shutil.which("env") or "env",
        E2E_SOCKET=socket,
        E2E_ROOT=str(root),
        E2E_OUT=str(root / "out"),
    )
    env.update(extra_env)
    env["E2E_QUARANTINE_FILE"] = str(here / "quarantine.tsv")
    sink: IO[bytes] | None = open(out, "ab") if out else None
    try:
        return subprocess.run(
            [str(run_sh), *args],
            env=env,
            stdin=subprocess.DEVNULL,
            stdout=sink or subprocess.DEVNULL,
            stderr=subprocess.STDOUT if out else subprocess.DEVNULL,
        ).returncode
    finally:
        if sink:
            sink.close()


def scen(name: str) -> dict[str, str]:
    return {"E2E_SCENARIO_DIR": str(here / name)}


def main() -> None:
    run_txt = root / "run.txt"
    # The PASS scenarios get a generous budget; the hung one runs on its own with a short one, so a tight timeout never
    # races a healthy scenario.
    rc = run({**scen("scenarios"), "E2E_SCENARIO_TIMEOUT": os.environ.get("SELFTEST_TIMEOUT", "60")}, out=run_txt)
    hrc = run(
        {
            **scen("scenarios-hung"),
            "E2E_ROOT": str(root / "h"),
            "E2E_OUT": str(root / "h/out"),
            "E2E_SCENARIO_TIMEOUT": "4",
        },
        out=run_txt,
    )
    with open(root / "out/results.tsv", "a") as f:
        f.write((root / "h/out/results.tsv").read_text())
    print(run_txt.read_text(), end="")
    print("-- runner exit status with failing scenarios present")
    check("run.sh exits non-zero when scenarios FAIL", 1, rc)
    check("run.sh exits non-zero when a scenario hangs", 1, hrc)

    results = {}
    for line in (root / "out/results.tsv").read_text().splitlines():
        cols = line.split("\t")
        results.setdefault(cols[0], cols[1])
    for line in (here / "expect.tsv").read_text().splitlines():
        if line.strip():
            name, want = line.split("\t")
            check(name, want, results.get(name, "missing"))

    print("-- failures name the scenario file:line and carry their evidence")
    for scen_name, rx in (
        (
            "failing_plain_assert_fails",
            r"assertion failed: arithmetic broke\s+at \S*failing_plain_assert_fails\.py:\d+",
        ),
        (
            "unexpected_forge_call_fails_scenario",
            r"unexpected call:\s+at \S*unexpected_forge_call_fails_scenario\.py:\d+",
        ),
        ("unexpected_cli_call_fails_scenario", r"unexpected call:\s+at \S*unexpected_cli_call_fails_scenario\.py:\d+"),
        (
            "unexpected_named_cli_call_fails_scenario",
            r"unexpected call:\s+at \S*unexpected_named_cli_call_fails_scenario\.py:\d+",
        ),
    ):
        log = root / "out/detect" / f"{scen_name}.log"
        text = log.read_text() if log.exists() else ""
        check(f"{scen_name} log has file:line", True, bool(re.search(rx, text)))

    print("-- hung scenario's background child must be dead")
    global failed
    try:
        pid = int((root / "hung_child.pid").read_text().strip())
    except (OSError, ValueError):
        pid = 0
    alive = False
    if pid:
        try:
            os.kill(pid, 0)
            alive = True
        except ProcessLookupError:
            pass
    if alive:
        print(f"FAIL child {pid} still running")
        os.kill(pid, signal.SIGKILL)
        failed = True
    else:
        print("ok   child process tree was killed")
    if "rc=124" in run_txt.read_text():
        print("ok   timeout reported as rc=124")
    else:
        print("FAIL timeout not reported as rc=124")
        failed = True

    print("-- a filter that selects nothing must fail")
    check("zero selected scenarios exits 2", 2, run(scen("scenarios"), "no/such/scenario"))

    print("-- all-green run exits 0; E2E_REQUIRE_SKIPS_ZERO turns a skip into a failure")
    sub = lambda d: {"E2E_ROOT": str(root / d), "E2E_OUT": str(root / d / "out")}  # noqa: E731
    check("passing selection exits 0", 0, run({**scen("scenarios"), **sub("g")}, "missing_pane", "exit_status"))
    check("a skip alone exits 0", 0, run({**scen("scenarios"), **sub("s")}, "skip_is_reported"))
    check(
        "a skip with E2E_REQUIRE_SKIPS_ZERO=1 exits 1",
        1,
        run({**scen("scenarios"), **sub("s2"), "E2E_REQUIRE_SKIPS_ZERO": "1"}, "skip_is_reported"),
    )

    print("-- both x.sh and x.py for one behaviour is a configuration error")
    dup = root / "dup/area"
    dup.mkdir(parents=True)
    (dup / "same.sh").write_text("#!/usr/bin/env bash\nexit 0\n")
    (dup / "same.py").write_text("raise SystemExit(0)\n")
    check("a behaviour in both bash and python exits 2", 2, run({"E2E_SCENARIO_DIR": str(root / "dup")}))

    print("-- tracked e2e/ file names must be valid on Windows")
    listing = subprocess.run(
        ["git", "-C", str(here / ".."), "ls-files", "-z", "--", "."], stdout=subprocess.PIPE, text=True
    ).stdout
    bad_re = re.compile(r'[<>:"\\|?*\x00-\x1f]|[. ]$')
    bad = [p for p in listing.split("\0") if p and any(bad_re.search(part) for part in p.split("/"))]
    if not bad:
        print("ok   no tracked e2e/ name has Windows-invalid characters")
    else:
        print("FAIL Windows-invalid file names:")
        print("\n".join(bad))
        failed = True


try:
    main()
finally:
    subprocess.run(["tmux", "-L", socket, "kill-server"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    shutil.rmtree(root, ignore_errors=True)
print("selftest: FAILED" if failed else "selftest: all checks passed")
sys.exit(1 if failed else 0)
