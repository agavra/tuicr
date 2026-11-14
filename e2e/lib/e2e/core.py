"""Run environment, isolation, failure reporting and composed cleanup.

A scenario is a script whose body is a function decorated with @scenario. The decorator owns the process exit status:
0 pass, 1 fail, 77 skip, 124 timeout (set by run_scenario.py), 130/143 interrupted.

Importing this module sets up the harness environment (git/hg/locale isolation, scratch dirs), so it needs TUICR_BIN.
"""
from __future__ import annotations

import fnmatch
import json
import os
import re
import shutil
import signal
import subprocess
import sys
import tempfile
import traceback
from pathlib import Path
from typing import Callable, Dict, List, Optional

LIB = Path(__file__).resolve().parent
E2E_DIR = LIB.parent.parent  # e2e/
TUICR_BIN = os.environ.get("TUICR_BIN") or sys.exit("set TUICR_BIN to the tuicr binary")
COLS = int(os.environ.get("E2E_COLS", "160"))
ROWS = int(os.environ.get("E2E_ROWS", "45"))
TIMEOUT = float(os.environ.get("E2E_TIMEOUT", "10"))
UNEXPECTED_MARK = '"note": "UNEXPECTED'


class Failure(Exception):
    """An assertion or wait that did not hold. `evidence` (a screen, a log) is printed under the message."""

    def __init__(self, message: str, evidence: Optional[str] = None):
        super().__init__(message)
        self.evidence = evidence


class NoPane(Failure):
    """The tmux pane is gone: never mistaken for "text disappeared" or "process exited"."""


class Skip(Exception):
    pass


def grep(pattern: str, text: str) -> bool:
    """`grep -Eq pattern <<<text`: does any line match? (patterns are Python regexes, matched line by line)"""
    rx = re.compile(pattern)
    return any(rx.search(line) for line in text.split("\n"))


def must_fail(fn: Callable, message: str, exc=Failure):
    """Assert that fn() raises `exc` (a Failure by default; NoPane for the missing-pane checks). For harness self-tests:
    a call that returns, or fails with something else, fails the scenario with `message`."""
    try:
        fn()
    except exc:
        return
    except Exception as e:  # noqa: BLE001
        raise AssertionError(f"{message} (raised {type(e).__name__}: {e}, not {exc.__name__})") from e
    raise AssertionError(message)


def _setup_env():
    """Harness-side isolation, exported to this process (fixture git/hg/jj commands inherit it)."""
    root = os.environ.get("E2E_ROOT") or tempfile.mkdtemp(prefix="tuicr-e2e.", dir=os.environ.get("TMPDIR", "/tmp"))
    os.environ.update(E2E_DIR=str(E2E_DIR), E2E_ROOT=root,
                      E2E_SOCKET=os.environ.get("E2E_SOCKET", f"tuicr-e2e-{os.getpid()}"),
                      E2E_OUT=os.environ.get("E2E_OUT", f"{root}/out"))
    Path(os.environ["E2E_OUT"]).mkdir(parents=True, exist_ok=True)
    os.environ.update(GIT_CONFIG_GLOBAL="/dev/null", GIT_CONFIG_NOSYSTEM="1", GIT_TERMINAL_PROMPT="0",
                      GIT_AUTHOR_NAME="E2E", GIT_AUTHOR_EMAIL="e2e@example.invalid",
                      GIT_COMMITTER_NAME="E2E", GIT_COMMITTER_EMAIL="e2e@example.invalid",
                      HGRCPATH="/dev/null", HGPLAIN="1", HGUSER="e2e")
    # UTF-8 locale for tuicr and tmux: en_US.UTF-8 where installed, else C.UTF-8 (CI images).
    loc = os.environ.get("E2E_LOCALE")
    if not loc:
        have = subprocess.run(["locale", "-a"], stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
                              universal_newlines=True).stdout.lower().split()
        loc = "en_US.UTF-8" if {"en_us.utf-8", "en_us.utf8"} & set(have) else "C.UTF-8"
    os.environ.update(LANG=loc, LC_ALL=loc)
    # PATH of the child: fake forge CLIs first, then system dirs, then the dirs of the tools the harness needs.
    tool_path = ["/usr/bin", "/bin"]
    for tool in ("git", "python3"):
        p = shutil.which(tool)
        if not p:
            sys.exit(f"e2e: required tool '{tool}' not found")
        tool_path.append(os.path.dirname(p))
    if os.environ.get("E2E_EXTRA_PATH"):
        tool_path.append(os.environ["E2E_EXTRA_PATH"])
    return loc, ":".join(tool_path)


LOCALE, TOOL_PATH = _setup_env()
ROOT = Path(os.environ["E2E_ROOT"])
OUT = Path(os.environ["E2E_OUT"])
SOCKET = os.environ["E2E_SOCKET"]


def child_env(d: Path, path: str, extra: Optional[Dict[str, str]] = None) -> Dict[str, str]:
    """The isolated environment tuicr runs with (`env -i ...`): its own HOME/XDG dirs, the fake forges first on PATH."""
    env = dict(HOME=f"{d}/home", XDG_CONFIG_HOME=f"{d}/config", XDG_DATA_HOME=f"{d}/data",
               XDG_STATE_HOME=f"{d}/state", TMPDIR=f"{d}/tmp", PATH=path, LANG=LOCALE, LC_ALL=LOCALE,
               E2E_FORGE_LOG=f"{d}/forge.log", E2E_DATA=f"{E2E_DIR}/data")
    env.update(extra or {})
    return env


class CliResult:
    """A finished non-interactive tuicr run: exit status and stdout+stderr (trailing newlines stripped)."""

    def __init__(self, rc: int, out: str):
        self.rc, self.out = rc, out

    def __repr__(self):
        return f"CliResult(rc={self.rc}, out={self.out!r})"


class Scenario:
    """One scenario run: its name, scratch dir, the TUIs it started, its environment and cleanups."""

    def __init__(self, script: str):
        self.script = Path(os.environ.get("E2E_SCENARIO", script))
        self.name = f"{self.script.parent.name}_{self.script.stem}"   # <area>_<behaviour>
        self.dir = ROOT / self.name                                   # scratch dir (repos, files)
        self.tuis: list = []                                          # every Tui started, stopped at exit
        self.env: Dict[str, str] = {}      # extra variables for every child tuicr (E2E_ENV, but a dict)
        self.pre_path: List[str] = []      # directories put first on the child's PATH (E2E_PRE_PATH)
        self.cols, self.rows = COLS, ROWS  # size of the next pane
        self.forge_logs: List[Path] = []
        self.forge_unexpected: List[str] = []   # UNEXPECTED calls from logs a restart truncated
        self.allow_unexpected = False
        self.gerrit_proc = None
        self._cleanups: list = []

    def path_of(self, name: str) -> Path:
        """Scratch dir of a named TUI (the scenario's own name by default)."""
        return ROOT / name

    # ---- lifecycle -----------------------------------------------------------------------------------------------
    def on_exit(self, fn: Callable, *args):
        """Register cleanup; runs in reverse order after the TUIs are stopped. Never install signal/atexit handlers."""
        self._cleanups.append((fn, args))

    def skip(self, reason: str):
        raise Skip(reason)

    def allow_unexpected_forge_calls(self, why: str = ""):
        """A scenario that provokes calls the fakes have no fixture for, on purpose, declares it (say why)."""
        self.allow_unexpected = True

    def _unexpected_calls(self) -> List[str]:
        bad = list(self.forge_unexpected)
        for log in dict.fromkeys(self.forge_logs):
            if log.exists():
                bad += [l for l in log.read_text().splitlines() if UNEXPECTED_MARK in l]
        return bad

    # ---- starting tuicr (interactive, in a tmux pane) --------------------------------------------------------------
    def start(self, *args, name: Optional[str] = None, cwd=None, env: Optional[Dict[str, str]] = None, keep: bool = False,
              cols: Optional[int] = None, rows: Optional[int] = None):
        """Start `tuicr --no-update-check <args>` in a tmux pane and return its Tui.

        cwd: working directory (default <scratch>/work); env: extra child variables; keep=True keeps home/data/... so
        a restart sees the saved sessions; name: a second TUI in one scenario (own scratch dir and pane)."""
        return self.start_cmd([TUICR_BIN, "--no-update-check", *map(str, args)], name=name, cwd=cwd, env=env, keep=keep,
                              cols=cols, rows=rows)

    def start_cmd(self, argv: List[str], *, name: Optional[str] = None, cwd=None, env: Optional[Dict[str, str]] = None,
                  keep: bool = False, cols: Optional[int] = None, rows: Optional[int] = None):
        """Start an arbitrary command in the pane (same isolated environment as `start`)."""
        from .tui import Tui
        return Tui.start(self, [str(a) for a in argv], name or self.name, cwd=cwd, env=env, keep=keep,
                         cols=cols or self.cols, rows=rows or self.rows)

    # ---- non-interactive tuicr -----------------------------------------------------------------------------------
    def _cli_env(self, name: Optional[str], env, path):
        d = self.path_of(name or self.name)
        for sub in ("home", "config", "data", "state", "tmp"):
            (d / sub).mkdir(parents=True, exist_ok=True)
        self.forge_logs.append(d / "forge.log")   # the central UNEXPECTED check covers CLI runs too
        return child_env(d, path or f"{E2E_DIR}/bin:{TOOL_PATH}", {**self.env, **(env or {})})

    def cli(self, *args, input: Optional[str] = None, env: Optional[Dict[str, str]] = None, path: Optional[str] = None,
            name: Optional[str] = None, check: bool = True) -> str:
        """Run tuicr with the scenario's isolated env and return stdout (stderr goes to the log).

        A non-zero exit fails the scenario unless check=False. path replaces the child's PATH (see minimal_path)."""
        argv = [TUICR_BIN, *map(str, args)]
        p = subprocess.run(argv, input=input,
                           stdin=None if input is not None else subprocess.DEVNULL,
                           env=self._cli_env(name, env, path), stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                           universal_newlines=True)
        sys.stderr.write(p.stderr)
        if check and p.returncode != 0:
            raise Failure(f"`tuicr {' '.join(argv[1:])}` exited {p.returncode}",
                          f"stdout:\n{p.stdout}\nstderr:\n{p.stderr}")
        return p.stdout

    def cli_result(self, *args, env: Optional[Dict[str, str]] = None, path: Optional[str] = None,
                   name: Optional[str] = None) -> CliResult:
        """Run tuicr with stdin closed; stdout+stderr and the exit status. Never fails."""
        p = subprocess.run([TUICR_BIN, *map(str, args)], stdin=subprocess.DEVNULL, env=self._cli_env(name, env, path),
                           stdout=subprocess.PIPE, stderr=subprocess.STDOUT, universal_newlines=True)
        return CliResult(p.returncode, p.stdout.rstrip("\n"))

    # ---- saved sessions ------------------------------------------------------------------------------------------
    def session_files(self, name: Optional[str] = None) -> List[Path]:
        """Saved session json files: $XDG_DATA_HOME on Linux, $HOME/Library/Application Support on macOS."""
        d = self.path_of(name or self.name)
        return sorted(p for top in (d / "data", d / "home") if top.exists() for p in top.rglob("*.json")
                      if fnmatch.fnmatch(str(p), "*/sessions/*.json"))

    def session_json(self, name: Optional[str] = None) -> str:
        """The text of every saved session file, concatenated ("" when there is none)."""
        return "".join(p.read_text() for p in self.session_files(name))

    def session(self, name: Optional[str] = None) -> dict:
        """The one saved session, parsed. Zero or several sessions fail."""
        files = self.session_files(name)
        if len(files) != 1:
            raise Failure(f"expected exactly one saved session, found {len(files)}", "\n".join(map(str, files)))
        return json.loads(files[0].read_text())

    def session_has(self, pattern: str, name: Optional[str] = None) -> bool:
        """A saved session mentions the pattern (a regex, matched line by line)."""
        return grep(pattern, self.session_json(name))

    # ---- fixtures and flows (thin forwarding, so a scenario reads `s.diff()`) ------------------------------------
    def open(self, *args, **kw):
        from . import flows
        return flows.open_review(self, *args, **kw)

    def diff(self, **kw):
        from . import flows
        return flows.diff(self, **kw)

    def pr_open(self, variant: str = "threads_and_commits", **kw):
        from . import forge
        return forge.pr_open(self, variant, **kw)

    def forge_open(self, url: str, variant: Optional[str] = None, **kw):
        from . import forge
        return forge.forge_open(self, url, variant, **kw)

    def gerrit_open(self, kind: str, **kw):
        from . import forge
        return forge.gerrit_open(self, kind, **kw)

    def gerrit_stop(self):
        from . import forge
        forge.gerrit_stop(self)

    def use_hg(self):
        from . import repos
        repos.use_hg(self)

    def use_jj(self):
        from . import repos
        repos.use_jj(self)

    def fake_clipboard(self, name: Optional[str] = None):
        from . import flows
        return flows.FakeClipboard(self, name or self.name)

    def minimal_path(self, directory, *tools: str) -> str:
        from . import flows
        return flows.minimal_path(directory, *tools)


def _report(exc: BaseException):
    """Print what failed, where in the scenario, and the evidence."""
    frames = [f for f in traceback.extract_tb(exc.__traceback__) if not f.filename.startswith(str(LIB))]
    where = "".join(f"  at {_rel(f.filename)}:{f.lineno}: {f.line}\n" for f in frames)
    if isinstance(exc, Failure):
        sys.stderr.write(f"e2e: {exc}\n{where}")
        if exc.evidence:
            sys.stderr.write(exc.evidence.rstrip("\n") + "\n")
    elif isinstance(exc, AssertionError):
        sys.stderr.write(f"e2e: assertion failed: {exc}\n{where}")
    else:
        traceback.print_exception(type(exc), exc, exc.__traceback__)


def _rel(path: str) -> str:
    try:
        return os.path.relpath(path, E2E_DIR)
    except ValueError:
        return path


def scenario(fn):
    """Run fn(Scenario) as the script's main and exit with the harness status conventions."""
    import __main__
    s = Scenario(getattr(__main__, "__file__", sys.argv[0]))
    signal.signal(signal.SIGTERM, lambda *_: sys.exit(143))
    rc = 0
    try:
        fn(s)
    except Skip as e:
        print(f"SKIP: {e}")
        rc = 77
    except KeyboardInterrupt:
        rc = 130
    except SystemExit as e:
        rc = e.code if isinstance(e.code, int) else 1
        if isinstance(e.code, str):
            sys.stderr.write(e.code + "\n")
    except BaseException as e:  # noqa: BLE001 - every other outcome is a scenario failure, reported with its location
        _report(e)
        rc = 1
    finally:
        for tui in s.tuis:
            tui.stop()
        for cleanup, args in reversed(s._cleanups):
            try:
                cleanup(*args)
            except Exception as e:  # noqa: BLE001 - cleanup must not mask the result
                sys.stderr.write(f"e2e: cleanup {getattr(cleanup, '__name__', cleanup)} failed: {e}\n")
        # No scenario may pass while a fake forge saw a call its fixtures do not cover.
        bad = s._unexpected_calls()
        if rc == 0 and bad and not s.allow_unexpected:
            sys.stderr.write("e2e: fake forge saw an unexpected call:\n"
                             f"  at {_rel(str(s.script))}:{fn.__code__.co_firstlineno}: {fn.__name__}() finished with it unreported\n"
                             + "\n".join(bad) + "\n")
            rc = 1
    sys.exit(rc)
