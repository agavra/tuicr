"""The terminal driver: tuicr runs in a private tmux server; a Tui sends keys and reads the screen.

Every query that can fail (no such pane, capture error) raises NoPane: a missing pane is never "text disappeared",
"forbidden text absent" or "process exited". Patterns are regexes matched line by line, like `grep -E` on the captured
screen. Waits and key methods return the Tui, so steps chain: tui.cmd("theme").wait_for("Switch Theme").keys("Enter").
"""

from __future__ import annotations

import contextlib
import json
import math
import re
import shutil
import subprocess
import time
import zlib
from pathlib import Path
from typing import TYPE_CHECKING

from .core import E2E_DIR, OUT, ROOT, SOCKET, TIMEOUT, TOOL_PATH, UNEXPECTED_MARK, Failure, NoPane, child_env, grep
from .waits import polls

if TYPE_CHECKING:
    from .core import Scenario

Rows = int | tuple[int, int]


def tmux(*args: str, check: bool = True) -> subprocess.CompletedProcess[str]:
    """Run tmux on the scenario's private server (socket $E2E_SOCKET, no config, UTF-8)."""
    p = subprocess.run(
        ["tmux", "-L", SOCKET, "-f", "/dev/null", "-u", *args],
        stdin=subprocess.DEVNULL,
        capture_output=True,
        text=True,
    )
    if check and p.returncode != 0:
        raise Failure(f"tmux {' '.join(args)} failed: {p.stderr.strip()}")
    return p


def _anchor(cols: int, rows: int) -> None:
    """An idle session keeps one tmux server alive for the whole run (a new-session sent to a server that is shutting
    down is lost); remain-on-exit is server-wide so even an instantly dying program leaves its pane and exit status."""
    if tmux("has-session", "-t", "e2e-anchor", check=False).returncode == 0:
        return
    tmux("new-session", "-d", "-x", str(cols), "-y", str(rows), "-s", "e2e-anchor", "--", "sleep", "86400")
    tmux("set-option", "-g", "remain-on-exit", "on")


class Tui:
    def __init__(self, scenario: Scenario, name: str) -> None:
        self.s, self.name, self.session = scenario, name, f"e2e-{name}"
        self.dir: Path = ROOT / name
        self.forge_log = self.dir / "forge.log"

    @classmethod
    def start(
        cls,
        s: Scenario,
        argv: list[str],
        name: str,
        cwd: str | Path | None = None,
        env: dict[str, str] | None = None,
        keep: bool = False,
        cols: int = 160,
        rows: int = 45,
    ) -> Tui:
        """Fresh home/config/data/state/tmp/work (unless keep), then a new pane running argv in cwd."""
        t = cls(s, name)
        d = t.dir
        d.mkdir(parents=True, exist_ok=True)
        if t.forge_log.exists():  # remember UNEXPECTED calls before the restart truncates the log
            s.forge_unexpected += [ln for ln in t.forge_log.read_text().splitlines() if UNEXPECTED_MARK in ln]
        for sub in ("home", "config", "data", "state", "tmp", "work"):
            if not keep:
                shutil.rmtree(d / sub, ignore_errors=True)
            (d / sub).mkdir(parents=True, exist_ok=True)
        t.forge_log.write_text("")
        exit_file = d / "exit.status"
        if exit_file.exists():
            exit_file.unlink()
        s.forge_logs.append(t.forge_log)
        path = ":".join([*s.pre_path, f"{E2E_DIR}/bin", TOOL_PATH])
        cenv = child_env(
            d,
            path,
            {
                "TERM": "xterm-256color",
                "GIT_CONFIG_NOSYSTEM": "1",
                "GIT_CONFIG_GLOBAL": f"{d}/home/.gitconfig",
                **s.env,
                **(env or {}),
            },
        )
        _anchor(cols, rows)
        tmux("kill-session", "-t", t.session, check=False)
        tmux(
            "new-session",
            "-d",
            "-x",
            str(cols),
            "-y",
            str(rows),
            "-s",
            t.session,
            "-c",
            str(cwd or d / "work"),
            "--",
            "sh",
            "-c",
            '"$@"; rc=$?; echo $rc > "$0"; exit $rc',
            str(exit_file),
            "env",
            "-i",
            *(f"{k}={v}" for k, v in cenv.items()),
            *argv,
        )
        s.tuis.append(t)
        if tmux("has-session", "-t", t.session, check=False).returncode != 0:
            raise Failure(f"tmux session for '{t.name}' vanished right after start")
        return t

    # ---- input -----------------------------------------------------------------------------------------------------
    def keys(self, *keys: str) -> Tui:
        """tmux send-keys: key names (Enter, Escape, C-c, Tab...) or text; several arguments are sent in order."""
        tmux("send-keys", "-t", self.session, *keys)
        return self

    def type(self, text: str) -> Tui:
        """tmux send-keys -l: text sent literally, never parsed as key names (escape sequences, ';e')."""
        tmux("send-keys", "-t", self.session, "-l", text)
        return self

    def click(self, col: int, row: int, button: int = 0) -> Tui:
        """SGR mouse press + release at 1-based (col, row); button 0 left, 1 middle, 2 right."""
        return self.type(f"\x1b[<{button};{col};{row}M\x1b[<{button};{col};{row}m")

    def paste(self, text: str) -> Tui:
        """Bracketed paste, through a tmux buffer."""
        tmux("set-buffer", "-b", "e2e", text)
        tmux("paste-buffer", "-p", "-b", "e2e", "-t", self.session)
        return self

    def resize(self, cols: int, rows: int) -> Tui:
        tmux("resize-window", "-t", self.session, "-x", str(cols), "-y", str(rows))
        return self

    def kill(self) -> Tui:
        """Kill the tmux session (to test what a vanished pane does to the waits)."""
        tmux("kill-session", "-t", self.session)
        return self

    # ---- reading the screen ----------------------------------------------------------------------------------------
    def _no_pane(self) -> NoPane:
        return NoPane(f"no pane for '{self.name}' (tmux session gone or never started)")

    def screen(self, colors: bool = False, join: bool = True) -> str:
        """The pane text, trailing newlines stripped. colors=True keeps the SGR escapes (-e); join=False does not
        re-join wrapped lines (-J off). Raises NoPane when the pane cannot be captured."""
        args = ["capture-pane", "-p", *(["-e"] if colors else []), *(["-J"] if join else []), "-t", self.session]
        p = tmux(*args, check=False)
        if p.returncode != 0:
            raise self._no_pane()
        return p.stdout.rstrip("\n")

    def cursor_row(self) -> str:
        """The row(s) carrying the cursor marker ▶."""
        return "\n".join(ln for ln in self.screen().split("\n") if "▶" in ln)

    def colors_hash(self, *rows: Rows) -> int:
        """Checksum of the screen WITH colours (to compare themes). Rows are 1-based: an int or a (first, last) pair
        per argument ('1p;3,30p' in sed is colors_hash(1, (3, 30))); no argument hashes the whole screen."""
        lines = self.screen(colors=True, join=False).split("\n")
        picked = (
            lines
            if not rows
            else [ln for r in rows for ln in (lines[r - 1 : r] if isinstance(r, int) else lines[r[0] - 1 : r[1]])]
        )
        return zlib.crc32("\n".join(picked).encode())

    def _display(self, fmt: str, allow_empty: bool = False) -> str:
        """tmux display-message for the pane. tmux 3.8 answers a missing target with status 0 and empty output, so an
        empty answer means "no pane" too, unless the format can legitimately be empty (allow_empty)."""
        p = tmux("display-message", "-p", "-t", self.session, fmt, check=False)
        out = p.stdout.strip()
        if p.returncode != 0 or (not out and not allow_empty):
            raise self._no_pane()
        return out

    def pane_exists(self) -> bool:
        try:
            return bool(self._display("#{pane_id}"))
        except NoPane:
            return False

    def alive(self) -> bool:
        """The pane exists and its process still runs. A missing pane raises NoPane, never "not alive"."""
        return self._display("#{pane_dead}") == "0"

    def exit_status(self) -> str:
        """Exit status of the finished process ("" while it runs). tmux 3.4 can leave #{pane_dead_status} empty for a
        dead pane, so the start wrapper also records it in a file."""
        self._display("#{pane_id}")  # the pane must exist: an empty status below is then "still running", not "gone"
        st = self._display("#{pane_dead_status}", allow_empty=True)
        f = self.dir / "exit.status"
        return st or (f.read_text().strip() if f.exists() else "")

    # ---- waits and assertions --------------------------------------------------------------------------------------
    def wait_for(self, pattern: str, timeout: float = TIMEOUT) -> Tui:
        """Poll (100 ms) until a screen line matches; otherwise fail with the screen."""
        for _ in range(polls(timeout, 0.1)):
            if grep(pattern, self.screen()):
                return self
            time.sleep(0.1)
        raise Failure(f"timed out after {timeout:g}s waiting for /{pattern}/ in '{self.name}'. Screen:", self.screen())

    expect = wait_for  # render may lag, so expecting text is waiting for it

    def wait_gone(self, pattern: str, timeout: float = TIMEOUT) -> Tui:
        """The pane exists and no line matches (a missing pane fails)."""
        for _ in range(polls(timeout, 0.1)):
            if not grep(pattern, self.screen()):
                return self
            time.sleep(0.1)
        raise Failure(f"/{pattern}/ still on screen in '{self.name}' after {timeout:g}s", self.screen())

    def reject(self, pattern: str, window: float = 0.4) -> Tui:
        """The text stays absent for the whole window (>= 1 capture). Only meaningful after a positive barrier proved
        the app reached the state in which the text would show."""
        for _ in range(max(1, math.floor(window * 10 + 0.999))):
            scr = self.screen()
            if grep(pattern, scr):
                raise Failure(f"unexpected /{pattern}/ on screen in '{self.name}':", scr)
            time.sleep(0.1)
        return self

    def check(self, pattern: str, message: str) -> Tui:
        """The current screen (no polling) has a matching line. For state a wait already proved."""
        scr = self.screen()
        if not grep(pattern, scr):
            raise Failure(f"{message} (no /{pattern}/ on screen)", scr)
        return self

    def settle(self, quiet: float = 0.25, timeout: float = TIMEOUT) -> Tui:
        """Wait until the screen has not changed for `quiet` seconds (polls every 50 ms). For "the UI finished reacting"
        when nothing specific is to appear; prefer wait_for."""
        need, stable, prev = int(quiet * 20 + 0.5), 0, self.screen()
        for _ in range(int(timeout * 20)):
            time.sleep(0.05)
            cur = self.screen()
            if cur == prev:
                stable += 1
                if stable >= need:
                    return self
            else:
                stable, prev = 0, cur
        raise Failure(f"screen of '{self.name}' never settled within {timeout:g}s")

    def wait_screen_change(self, before: str, timeout: float = TIMEOUT) -> Tui:
        """The screen differs from one captured earlier (barrier for "the app reacted"); timeout may be fractional."""
        for _ in range(polls(timeout, 0.05)):
            if self.screen() != before:
                return self
            time.sleep(0.05)
        raise Failure(f"screen of '{self.name}' did not change within {timeout:g}s")

    def wait_exit(self, timeout: float = TIMEOUT, status: int | str = 0) -> Tui:
        """Wait for the process to end; its status must match (default 0; "any" accepts every status)."""
        for _ in range(polls(timeout, 0.1)):
            if self._display("#{pane_dead}") == "1":
                st = self.exit_status()
                if status == "any" or st == str(status):
                    return self
                raise Failure(
                    f"'{self.name}' exited with status {st or '?'}, expected {status}. Screen:", self.screen()
                )
            time.sleep(0.1)
        raise Failure(f"'{self.name}' did not exit within {timeout:g}s", self.screen())

    # ---- shared UI flows -------------------------------------------------------------------------------------------
    def quit(self) -> Tui:
        """Force-quit (`:q!`) and wait for a clean exit."""
        return self.keys("Escape", ":q!", "Enter").wait_exit()

    def cmd(self, command: str) -> Tui:
        """Run a ':' command: Escape first, so it also works from a sub-mode."""
        return self.keys("Escape").keys(f":{command}", "Enter")

    def save(self) -> Tui:
        """`:w`, then wait for the confirmation "Saved to <path>"."""
        return self.cmd("w").wait_for("Saved to")

    def comment(self, text: str, key: str = "c") -> Tui:
        """Open the comment editor with `key` (c line, C file, ;c review), type text, submit, wait for the rendered
        comment and for composing to end (NORMAL)."""
        self.keys(key).wait_for("COMMENT")
        self.keys(text).keys("Enter")
        return self.wait_for(text).wait_for("NORMAL")

    def cursor_on(self, pattern: str, steps: int = 80) -> Tui:
        """Press j until the cursor row (▶) matches the regex (at most `steps` presses)."""
        scr = self.screen()
        for _ in range(steps):
            cur = "\n".join(ln for ln in scr.split("\n") if "▶" in ln)
            if grep(pattern, cur):
                return self
            self.keys("j")
            try:  # the cursor moving redraws; at the end of the diff it cannot move, so stop waiting after 0.5 s
                self.wait_screen_change(scr, 0.5)
            except NoPane:
                raise
            except Failure:
                pass
            self.settle(
                0.1
            )  # the first changed frame can be a partial redraw: judge the cursor row on a settled screen
            scr = self.screen()
        raise Failure(f"cursor never reached /{pattern}/", scr)

    # ---- fake forge call log (one JSON object per line) ------------------------------------------------------------
    def forge_log_text(self) -> str:
        return self.forge_log.read_text() if self.forge_log.exists() else ""

    def forge_calls(self) -> list[dict]:
        return [json.loads(ln) for ln in self.forge_log_text().splitlines()]

    def forge_count(self, pattern: str) -> int:
        """Number of logged calls (raw JSON lines) matching the regex."""
        rx = re.compile(pattern)
        return sum(1 for ln in self.forge_log_text().splitlines() if rx.search(ln))

    def forge_expect(self, pattern: str) -> Tui:
        """A logged call matches the regex now (no polling: wait_forge_log first if the call is asynchronous)."""
        if not self.forge_count(pattern):
            raise Failure(f"forge log lacks /{pattern}/", self.forge_log_text())
        return self

    def forge_reject(self, pattern: str) -> Tui:
        if self.forge_count(pattern):
            raise Failure(f"forge log has /{pattern}/", self.forge_log_text())
        return self

    def wait_forge_log(self, pattern: str, timeout: float = TIMEOUT, count: int = 1) -> Tui:
        """At least `count` logged calls match the regex (a barrier, not a sleep)."""
        for _ in range(polls(timeout, 0.1)):
            if self.forge_count(pattern) >= count:
                return self
            time.sleep(0.1)
        raise Failure(f"fewer than {count} forge calls matching /{pattern}/ within {timeout:g}s", self.forge_log_text())

    def posts(self) -> list[dict]:
        """Every review POST in the fake-gh log: {"path": "repos/...", "payload": <stdin json>}."""
        out = []
        for c in self.forge_calls():
            a = c["argv"]
            if c["tool"] == "gh" and "--method" in a and a[a.index("--method") + 1] == "POST":
                out.append(
                    {"path": next(x for x in a if x.startswith("repos/")), "payload": json.loads(c["stdin"] or "null")}
                )
        return out

    def forge_posts(self, tool: str) -> list[dict]:
        """Every mutating call of `tool`: {"argv": [...], "payload": <body json or None>}. A call mutates when its argv
        has a POST method (`--method POST`, `-X POST`, `--method=POST`) or is a `pr approve`."""
        out = []
        for c in self.forge_calls():
            a = c["argv"]
            post = (
                any(a[i] in ("--method", "-X") and a[i + 1] == "POST" for i in range(len(a) - 1))
                or "--method=POST" in a
            )
            if c["tool"] == tool and (post or a[:2] == ["pr", "approve"]):
                body = c.get("stdin") or (
                    a[a.index("--input") + 1] if "--input" in a else None
                )  # bkt: body as argument
                out.append({"argv": a, "payload": json.loads(body) if body else None})
        return out

    # ---- teardown --------------------------------------------------------------------------------------------------
    def stop(self) -> None:
        """Keep a screenshot and the forge log in $E2E_OUT, kill the session."""
        with contextlib.suppress(Failure):
            (OUT / f"{self.name}.screen.txt").write_text(self.screen() + "\n")
        if self.forge_log.exists() and self.forge_log.stat().st_size:
            shutil.copy(self.forge_log, OUT / f"{self.name}.forge.log")
        tmux("kill-session", "-t", self.session, check=False)
