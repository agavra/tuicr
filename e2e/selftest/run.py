#!/usr/bin/env python3
"""e2e/selftest/run.py - tests of the harness itself: every way the runner could report a false green must be
caught here. Needs only tmux, python3 and git (TUICR_BIN is never started; it defaults to `env`).

    uv run --python 3.12 e2e/selftest/run.py
"""

from __future__ import annotations

import ast
import json
import os
import re
import shutil
import signal
import subprocess
import sys
import tempfile
import tomllib
from pathlib import Path
from typing import IO

here = Path(__file__).resolve().parent
run_py = here / ".." / "run.py"
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
    """Run e2e/run.py with the selftest environment plus extra_env; returns its exit status."""
    env = dict(
        os.environ,
        TUICR_BIN=os.environ.get("TUICR_BIN") or shutil.which("env") or "env",
        E2E_SOCKET=socket,
        E2E_ROOT=str(root),
        E2E_OUT=str(root / "out"),
    )
    env.update(extra_env)
    sink: IO[bytes] | None = open(out, "ab") if out else None
    try:
        return subprocess.run(
            [sys.executable, str(run_py), *args],
            env=env,
            stdin=subprocess.DEVNULL,
            stdout=sink or subprocess.DEVNULL,
            stderr=subprocess.STDOUT if out else subprocess.DEVNULL,
        ).returncode
    finally:
        if sink:
            sink.close()


def expected(script: Path) -> str:
    """The scenario's module-level EXPECTED constant, read without running it."""
    for node in ast.parse(script.read_text()).body:
        if isinstance(node, ast.Assign) and any(isinstance(t, ast.Name) and t.id == "EXPECTED" for t in node.targets):
            return str(ast.literal_eval(node.value))
    return "undeclared"


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
    print(run_txt.read_text(), end="")
    print("-- runner exit status with failing scenarios present")
    check("run.py exits non-zero when scenarios FAIL", 1, rc)
    check("run.py exits non-zero when a scenario hangs", 1, hrc)

    results: dict[str, str] = {}
    for results_json in (root / "out/results.json", root / "h/out/results.json"):
        for entry in json.loads(results_json.read_text()):
            results.setdefault(entry["name"], entry["result"])
    # every selftest scenario declares its expected result (module-level EXPECTED) and must have got it
    declared = 0
    for scen_dir in ("scenarios", "scenarios-hung"):
        for script in sorted((here / scen_dir).rglob("*.py")):
            name = str(script.relative_to(here / scen_dir).with_suffix(""))
            want = expected(script)
            declared += 1
            check(name, want, results.get(name, "missing"))
    check("every selftest scenario declares EXPECTED", len(results), declared)

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

    print("-- a scenario with a syntax error is reported FAIL and the run continues")
    bad_dir = root / "bad-scen"
    (bad_dir / "d").mkdir(parents=True)
    (bad_dir / "d/a_broken.py").write_text("def (:\n")
    (bad_dir / "d/b_good.py").write_text(
        "from e2e import Scenario, scenario\n\n\n@scenario\ndef main(s: Scenario) -> None:\n    pass\n"
    )
    bad_env = {"E2E_SCENARIO_DIR": str(bad_dir), **sub("bad")}
    check("syntax-error run exits 1", 1, run(bad_env))
    got = {e["name"]: e["result"] for e in json.loads((root / "bad/out/results.json").read_text())}
    check("syntax-error scenario FAILs, later one still PASSes", {"d/a_broken": "FAIL", "d/b_good": "PASS"}, got)

    print("-- empty E2E_SOCKET/E2E_OUT fall back to their defaults")
    check(
        "empty socket/out exit 0",
        0,
        run({**scen("scenarios"), **sub("e"), "E2E_SOCKET": "", "E2E_OUT": ""}, "exit_status"),
    )
    check("results land in <root>/out", True, (root / "e/out/results.json").is_file())

    print("-- settle quiet period: TOML default and E2E_SETTLE_QUIET override reach Tui.settle")
    toml_default = tomllib.loads((here / ".." / "pyproject.toml").read_text())["tool"]["tuicr-e2e"]["settle_quiet"]
    settle = {**scen("scenarios-settle"), "E2E_SETTLE_QUIET": ""}  # empty = use the TOML default
    check("settle default comes from TOML", 0, run({**settle, **sub("t1"), "SETTLE_EXPECT": str(toml_default)}))
    check(
        "E2E_SETTLE_QUIET overrides it",
        0,
        run({**settle, **sub("t2"), "E2E_SETTLE_QUIET": "0.6", "SETTLE_EXPECT": "0.6"}),
    )
    check("a wrong expectation fails (probe is live)", 1, run({**settle, **sub("t3"), "SETTLE_EXPECT": "0.123"}))

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
