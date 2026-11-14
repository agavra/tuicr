#!/usr/bin/env python3
"""e2e/run.py - run the end-to-end scenarios against a real tuicr binary.

    TUICR_BIN=target/release/tuicr uv run --python 3.12 e2e/run.py [-l|--list] [filter...]

A filter is a substring of `<area>/<behaviour>` (e.g. `pr/submit`, `tree/`, `theme/picker_preview_revert_keep`);
no filter runs everything. Results: PASS, FAIL (includes timeouts), SKIP (scenario exit 77, e.g. no Mercurial),
and QPASS/QFAIL for quarantined scenarios (a module-level `QUARANTINE = "<reason>"`: run and reported, never gating).
Exit 0 only if at least one scenario ran, none FAILed, and (with E2E_REQUIRE_SKIPS_ZERO=1) none was skipped.

Environment: TUICR_BIN (required); E2E_ROOT (scratch, default a fresh temp dir), E2E_OUT (logs, screenshots,
results.json; default $E2E_ROOT/out), E2E_SOCKET (tmux socket, default unique), E2E_SCENARIO_TIMEOUT (s),
E2E_SCENARIO_DIR (default e2e/scenarios), E2E_KEEP_ROOT=1 (keep scratch dirs of passing scenarios), E2E_REQUIRE_HG=1.
Defaults for the timeout, the tmux socket prefix and the settle quiet period live in e2e/pyproject.toml under
[tool.tuicr-e2e]; the environment variables override them.

Each scenario runs in its own session/process group. On timeout, or when the runner is interrupted, the whole
process tree is killed (the scenario's backgrounded fake servers included); after a normal exit leftovers are reaped
too. Scenario exit status: 0 pass, 77 skip, 124 timeout, 130 interrupted, 128+N killed by signal N.
"""

from __future__ import annotations

import ast
import contextlib
import json
import os
import shutil
import signal
import subprocess
import sys
import tempfile
import time
import tomllib
from pathlib import Path
from typing import Any

HERE = Path(__file__).resolve().parent


def config() -> dict[str, Any]:
    with open(HERE / "pyproject.toml", "rb") as f:
        return tomllib.load(f).get("tool", {}).get("tuicr-e2e", {})


def metadata(script: Path, name: str) -> str | None:
    """A module-level string constant of a scenario (QUARANTINE, EXPECTED), read without running the scenario."""
    try:
        body = ast.parse(script.read_text()).body
    except (SyntaxError, ValueError):
        return None  # the scenario itself will fail to start and be reported FAIL; never abort the run
    for node in body:
        if isinstance(node, ast.Assign) and any(isinstance(t, ast.Name) and t.id == name for t in node.targets):
            try:
                value = ast.literal_eval(node.value)
            except ValueError:
                return None
            if isinstance(value, str):
                return value
    return None


def discover(scen_dir: Path, filters: list[str]) -> list[str]:
    names = sorted(str(p.relative_to(scen_dir).with_suffix("")) for p in scen_dir.rglob("*.py"))
    return [n for n in names if not filters or any(f in n for f in filters)]


def run_scenario(script: Path, timeout: float, log: Path) -> int:
    with open(log, "wb") as out:
        proc = subprocess.Popen(
            [sys.executable, str(script)],
            stdout=out,
            stderr=subprocess.STDOUT,
            start_new_session=True,
            env=dict(os.environ, E2E_SCENARIO=str(script)),
        )

        def kill_group() -> None:
            with contextlib.suppress(ProcessLookupError, PermissionError):
                os.killpg(proc.pid, signal.SIGKILL)

        try:
            rc = proc.wait(timeout)
            if rc < 0:
                rc = 128 - rc
        except subprocess.TimeoutExpired:
            out.write(f"e2e: scenario exceeded {timeout:g}s and was killed (process tree included)\n".encode())
            kill_group()
            proc.wait()
            rc = 124
        except KeyboardInterrupt:
            kill_group()
            proc.wait()
            raise
        kill_group()
        return rc


def main(argv: list[str]) -> int:
    list_only = any(a in ("-l", "--list") for a in argv)
    filters = [a for a in argv if a not in ("-l", "--list")]
    scen_dir = Path(os.environ.get("E2E_SCENARIO_DIR") or HERE / "scenarios").resolve()
    selected = discover(scen_dir, filters)
    if not selected:
        print(f"e2e: no scenario matches '{' '.join(filters)}' under {scen_dir}", file=sys.stderr)
        return 2
    if list_only:
        print("\n".join(selected))
        return 0

    tuicr_bin = os.environ.get("TUICR_BIN")
    if not tuicr_bin:
        print("e2e: set TUICR_BIN to the tuicr binary", file=sys.stderr)
        return 2
    if not (os.path.isfile(tuicr_bin) and os.access(tuicr_bin, os.X_OK)):
        print(f"e2e: TUICR_BIN={tuicr_bin} is not an executable file", file=sys.stderr)
        return 2
    os.environ["TUICR_BIN"] = os.path.abspath(tuicr_bin)
    os.environ["PYTHONPATH"] = str(HERE / "lib") + (
        os.pathsep + os.environ["PYTHONPATH"] if "PYTHONPATH" in os.environ else ""
    )

    cfg = config()
    timeout = float(os.environ.get("E2E_SCENARIO_TIMEOUT") or cfg.get("scenario_timeout", 120))
    os.environ["E2E_SETTLE_QUIET"] = os.environ.get("E2E_SETTLE_QUIET") or str(cfg.get("settle_quiet", 0.25))
    own_root = not os.environ.get("E2E_ROOT")
    if own_root:
        os.environ["E2E_ROOT"] = tempfile.mkdtemp(prefix="tuicr-e2e.", dir=os.environ.get("TMPDIR", "/tmp"))
    root = Path(os.environ["E2E_ROOT"])
    # empty counts as unset, as in run.sh
    socket = os.environ["E2E_SOCKET"] = (
        os.environ.get("E2E_SOCKET") or f"{cfg.get('socket_prefix', 'tuicr-e2e')}-{os.getpid()}"
    )
    out_dir = Path(os.environ.get("E2E_OUT") or root / "out")
    os.environ["E2E_OUT"] = str(out_dir)
    out_dir.mkdir(parents=True, exist_ok=True)

    counts = dict.fromkeys(("PASS", "FAIL", "SKIP", "QPASS", "QFAIL"), 0)
    results: list[dict[str, object]] = []

    def cleanup() -> None:
        subprocess.run(["tmux", "-L", socket, "kill-server"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        if own_root and not os.environ.get("E2E_KEEP_ROOT") and counts["FAIL"] == 0:
            for child in root.iterdir():
                if child.name != "out":
                    shutil.rmtree(child, ignore_errors=True)

    def write_results() -> None:
        (out_dir / "results.json").write_text(json.dumps(results, indent=2) + "\n")

    write_results()
    signal.signal(signal.SIGTERM, lambda *_: (_ for _ in ()).throw(KeyboardInterrupt()))
    try:
        for name in selected:
            script = scen_dir / f"{name}.py"
            log = out_dir / f"{name}.log"
            log.parent.mkdir(parents=True, exist_ok=True)
            start = time.monotonic()
            rc = run_scenario(script, timeout, log)
            secs = int(time.monotonic() - start)
            quarantine = metadata(script, "QUARANTINE")
            if quarantine is not None:
                res = "QPASS" if rc == 0 else "QFAIL"
                print(f"{res} {name} ({secs}s)  quarantined: {quarantine}")
            elif rc == 0:
                res = "PASS"
                print(f"PASS {name} ({secs}s)")
            elif rc == 77:
                res = "SKIP"
                reason = next((ln for ln in log.read_text(errors="replace").splitlines() if ln.startswith("SKIP")), "")
                print(f"SKIP {name}  {reason}")
            else:
                res = "FAIL"
                print(f"FAIL {name} ({secs}s, rc={rc})  log: {log}")
            counts[res] += 1
            results.append({"name": name, "result": res, "seconds": secs, "log": str(log)})
            write_results()
    except KeyboardInterrupt:
        print("\ne2e: interrupted")
        counts["FAIL"] += 1  # keep the scratch dir
        cleanup()
        return 130

    cleanup()
    summary = ", ".join(f"{n} {k}" for k, n in counts.items())
    print(f"== {len(selected)} scenarios: {summary}  (results: {out_dir / 'results.json'})")
    if counts["FAIL"]:
        return 1
    if os.environ.get("E2E_REQUIRE_SKIPS_ZERO") and counts["SKIP"]:
        print(f"e2e: {counts['SKIP']} scenario(s) skipped but E2E_REQUIRE_SKIPS_ZERO=1", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
