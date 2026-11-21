"""The terminal driver: tuicr runs in a private tmux server; a Tui sends keys and reads the screen.

Every query that can fail (no such pane, capture error) raises NoPane: a missing pane is never "text disappeared",
"forbidden text absent" or "process exited". Every screen wait and check also fails (DeadPane) once the pane's process
has exited, unless the caller passes allow_dead=True or a wait_exit already accepted the exit; a TUI that died without
a wait_exit accepting it fails the scenario at teardown. Patterns are regexes matched line by line, like `grep -E` on
the captured screen. Waits and key methods return the Tui, so steps chain:
tui.cmd("theme").wait_for("Switch Theme").keys("Enter").
"""

from __future__ import annotations

import contextlib
import json
import math
import os
import re
import select
import shutil
import subprocess
import time
import zlib
from collections.abc import Callable
from pathlib import Path
from typing import TYPE_CHECKING

from .core import (
    E2E_DIR,
    OUT,
    REJECT_WINDOW,
    ROOT,
    SETTLE_QUIET,
    SOCKET,
    TIMEOUT,
    TOOL_PATH,
    UNEXPECTED_MARK,
    DeadPane,
    Failure,
    NoPane,
    child_env,
    grep,
)
from .waits import polls

if TYPE_CHECKING:
    from .core import Scenario

Rows = int | tuple[int, int]
Call = dict  # one parsed fake-forge record, see Tui.forge_calls


def call_method(argv: list[str]) -> str | None:
    """The HTTP method an `api` call of a fake CLI asks for, in any spelling (`--method X`, `--method=X`, `-X X`,
    `-XX`), upper-cased; GET for an `api` call that names none (a body flag without a method is still reported as
    GET: the fakes and tuicr always spell the method out). None for other commands (`pr view`, ...)."""
    for i, a in enumerate(argv):
        if a in ("--method", "-X") and i + 1 < len(argv):
            return argv[i + 1].upper()
        if a.startswith("--method="):
            return a.split("=", 1)[1].upper()
        if a.startswith("-X") and len(a) > 2:
            return a[2:].upper()
    return "GET" if "api" in argv[:2] else None


def _decode(stdin: str | None) -> object:
    """A call's stdin as JSON when it parses, else the raw text (None when the call had none)."""
    if not stdin:
        return stdin
    try:
        return json.loads(stdin)
    except ValueError:
        return stdin


def call_text(c: Call) -> str:
    """What the regex helpers match: the record as one JSON line with stdin DECODED (an object, not an escaped
    string) and non-ASCII kept, e.g. {"tool": "gh", "argv": [...], "stdin": {"body": "x"}}."""
    rec = {"tool": c["tool"], "argv": c["argv"], "stdin": c["body"]}
    if "note" in c:
        rec["note"] = c["note"]
    return json.dumps(rec, ensure_ascii=False)


def read_forge_log(path: Path) -> list[Call]:
    """Parse a fake-forge log. An incomplete trailing line (a fake still writing) is skipped; a corrupt complete line
    fails. Each record gains "index" (its line number, what forge_mark() counts), "body" (stdin decoded) and "method"
    (call_method of argv)."""
    text = path.read_text(errors="replace") if path.exists() else ""
    lines = text.split("\n")[:-1]  # the last element is "" after a final newline, or a partial record
    out: list[Call] = []
    for i, ln in enumerate(lines):
        try:
            rec = json.loads(ln)
        except ValueError:
            raise Failure(f"corrupt record {i + 1} in {path}: {ln[:200]!r}", text) from None
        rec["index"] = i
        rec["body"] = _decode(rec.get("stdin"))
        rec["method"] = call_method(rec.get("argv") or [])
        out.append(rec)
    return out


_TMUX = ["tmux", "-L", SOCKET, "-f", "/dev/null", "-u"]
# Commands the control client runs; anything else (new-session's long argv, set-option -g...) is exec'ed.
_CONTROL_COMMANDS = frozenset(
    {"capture-pane", "display-message", "send-keys", "has-session", "kill-session", "set-buffer", "paste-buffer"}
    | {"resize-window"}
)


class _Control:
    """One tmux control-mode client (`tmux -C attach`, output notifications off) per scenario process: each command
    is a line written to it, answered by a `%begin <time> <n> <flags>` ... `%end|%error <time> <n> <flags>` block, so a
    capture or a key costs no process launch. Commands are tmux-quoted; an argument the line syntax cannot carry
    (a newline, or a trailing `;` that tmux's argv parsing reads as a separator) is exec'ed instead. When the client
    cannot be started, dies or stalls, the command is exec'ed, so the answer is the same as without it."""

    def __init__(self) -> None:
        self.proc: subprocess.Popen[bytes] | None = None
        self.enabled = False  # set once this process made sure the anchor session exists
        self.buf = b""

    @staticmethod
    def line(args: tuple[str, ...]) -> tuple[str, int] | None:
        """(command line, number of commands) for args, or None when it must be exec'ed."""
        words, count, first = [], 1, True
        for a in args:
            if a == ";":
                words.append(";")
                count, first = count + 1, True
                continue
            if first and a not in _CONTROL_COMMANDS:
                return None
            if any(c in a for c in "\n\r\0") or a.endswith(";"):
                return None
            words.append("'" + a.replace("'", "'\\''") + "'")
            first = False
        return " ".join(words), count

    def _open(self) -> subprocess.Popen[bytes] | None:
        if self.proc is not None and self.proc.poll() is None:
            return self.proc
        self.proc, self.buf = None, b""
        try:
            p = subprocess.Popen(
                [*_TMUX, "-C", "attach-session", "-t", "e2e-anchor"],
                stdin=subprocess.PIPE,
                stdout=subprocess.PIPE,
                stderr=subprocess.DEVNULL,
            )
        except OSError:
            self.enabled = False
            return None
        self.proc = p
        if self._send("refresh-client -f no-output", 1) is None:  # also proves the client attached
            self.enabled = False
            return None
        return p

    def _readline(self, p: subprocess.Popen[bytes], deadline: float) -> str | None:
        assert p.stdout is not None
        while b"\n" not in self.buf:
            left = deadline - time.monotonic()
            if left <= 0 or not select.select([p.stdout], [], [], left)[0]:
                return None
            chunk = os.read(p.stdout.fileno(), 65536)
            if not chunk:
                return None
            self.buf += chunk
        ln, self.buf = self.buf.split(b"\n", 1)
        return ln.decode("utf-8", errors="replace")

    def _send(self, line: str, count: int) -> tuple[int, str, str] | None:
        """Run one command line; (status, stdout, stderr) like an exec'ed tmux, or None (the client is closed)."""
        p = self.proc
        assert p is not None and p.stdin is not None
        deadline = time.monotonic() + 2 * TIMEOUT
        out: list[str] = []
        try:
            p.stdin.write(line.encode() + b"\n")
            p.stdin.flush()
            while count:
                block: list[str] | None = None
                tag, mine, tail = [""], False, [""]
                while True:
                    ln = self._readline(p, deadline)
                    if ln is None:
                        raise OSError("tmux control client closed or stalled")
                    if block is None:
                        head = ln.split(" ")
                        if head[0] == "%begin" and len(head) == 4:
                            block, tag, mine = [], head[1:3], head[3] == "1"
                        continue  # a notification between blocks
                    tail = ln.split(" ")
                    if len(tail) == 4 and tail[0] in ("%end", "%error") and tail[1:3] == tag:
                        break
                    block.append(ln)
                if not mine:  # a block of a command this client did not send (the attach itself)
                    continue
                count -= 1
                if tail[0] == "%error":  # tmux stops a command list at its first error
                    return 1, "".join(x + "\n" for x in out), "".join(x + "\n" for x in block)
                out += block
        except OSError:
            p.kill()
            p.wait()
            self.proc = None
            return None
        return 0, "".join(x + "\n" for x in out), ""

    def run(self, args: tuple[str, ...]) -> tuple[int, str, str] | None:
        if not self.enabled or (parsed := self.line(args)) is None or self._open() is None:
            return None
        return self._send(*parsed)


_control = _Control()


def tmux(*args: str, check: bool = True) -> subprocess.CompletedProcess[str]:
    """Run tmux on the scenario's private server (socket $E2E_SOCKET, no config, UTF-8), through the control client
    when it can carry the command (see _Control), else as its own process."""
    res = _control.run(args)
    if res is not None:
        p = subprocess.CompletedProcess([*_TMUX, *args], *res)
    else:
        p = subprocess.run([*_TMUX, *args], stdin=subprocess.DEVNULL, capture_output=True, text=True)
    if check and p.returncode != 0:
        raise Failure(f"tmux {' '.join(args)} failed: {p.stderr.strip()}")
    return p


def _anchor(cols: int, rows: int) -> None:
    """An idle session keeps one tmux server alive for the whole run (a new-session sent to a server that is shutting
    down is lost); remain-on-exit is server-wide so even an instantly dying program leaves its pane and exit status.
    The control client attaches to it."""
    if _control.enabled and _control.proc is not None and _control.proc.poll() is None:
        return
    if tmux("has-session", "-t", "e2e-anchor", check=False).returncode != 0:
        tmux("new-session", "-d", "-x", str(cols), "-y", str(rows), "-s", "e2e-anchor", "--", "sleep", "86400")
        tmux("set-option", "-g", "remain-on-exit", "on")
    _control.enabled = True


class Tui:
    def __init__(self, scenario: Scenario, name: str) -> None:
        self.s, self.name, self.session = scenario, name, f"e2e-{name}"
        self.dir: Path = ROOT / name
        self.forge_log = self.dir / "forge.log"
        self.exit_ok = False  # a wait_exit accepted the process's exit (screen checks of the dead pane are fine then)
        self.retired = False  # a restart under the same name took over the pane

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
        for old in s.tuis:  # a restart under the same name: judge the old process before its pane is killed
            if old.session == t.session and not old.retired:
                old._note_death()
                old.retired = True
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
        self.exit_ok = True  # killed on purpose: not a death to report
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

    def _capture(self, colors: bool = False, join: bool = True) -> tuple[str, bool]:
        """(screen text, pane is dead), read with one tmux call so both describe the same moment."""
        args = ["capture-pane", "-p", *(["-e"] if colors else []), *(["-J"] if join else []), "-t", self.session]
        p = tmux(*args, ";", "display-message", "-p", "-t", self.session, "#{pane_dead}", check=False)
        text, _, flag = p.stdout.rstrip("\n").rpartition("\n")
        if p.returncode != 0 or flag not in ("0", "1"):
            raise self._no_pane()
        return text.rstrip("\n"), flag == "1"

    def _live(self, what: str, allow_dead: bool = False) -> str:
        """The screen, for a wait or check. A dead pane fails (DeadPane) unless allow_dead or a wait_exit accepted the
        exit: after a crash the alternate screen is gone, so absent text and a frozen screen prove nothing."""
        scr, dead = self._capture()
        if dead and not (allow_dead or self.exit_ok):
            raise DeadPane(
                f"'{self.name}' exited (status {self.exit_status() or '?'}) while {what}; pass allow_dead=True or "
                "wait_exit first if the exit is expected. Screen:",
                scr,
            )
        return scr

    @staticmethod
    def diff_pane(scr: str) -> str:
        """The screen with the file tree (and the comment navigator below it) blanked out: the columns of the box
        titled `┌ Files`. The tree draws `▶` too (collapsed directories), so cursor checks look only at the rest."""
        lines = scr.split("\n")
        box = next(((ln.index("┌ Files"), ln) for ln in lines if "┌ Files" in ln), None)
        if box is None:
            return scr
        start, top = box
        end = top.find("┐", start)
        if end < 0:
            return scr
        return "\n".join(ln[:start] + " " * (end + 1 - start) + ln[end + 1 :] for ln in lines)

    def cursor_row(self) -> str:
        """The row(s) carrying the diff cursor marker ▶ (the file tree's columns are ignored)."""
        return "\n".join(ln for ln in self.diff_pane(self.screen()).split("\n") if "▶" in ln)

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
    def wait_for(self, pattern: str, timeout: float = TIMEOUT, allow_dead: bool = False) -> Tui:
        """Poll (100 ms) until a screen line matches; otherwise fail with the screen."""
        what = f"waiting for /{pattern}/"
        for _ in range(polls(timeout, 0.1)):
            if grep(pattern, self._live(what, allow_dead)):
                return self
            time.sleep(0.1)
        raise Failure(
            f"timed out after {timeout:g}s waiting for /{pattern}/ in '{self.name}'. Screen:",
            self._live(what, allow_dead),
        )

    expect = wait_for  # render may lag, so expecting text is waiting for it

    def wait_gone(self, pattern: str, timeout: float = TIMEOUT, allow_dead: bool = False) -> Tui:
        """The pane exists, its process runs, and no line matches (a missing or dead pane fails)."""
        what = f"waiting for /{pattern}/ to go"
        for _ in range(polls(timeout, 0.1)):
            if not grep(pattern, self._live(what, allow_dead)):
                return self
            time.sleep(0.1)
        raise Failure(f"/{pattern}/ still on screen in '{self.name}' after {timeout:g}s", self._live(what, allow_dead))

    def reject(self, pattern: str, window: float = REJECT_WINDOW, allow_dead: bool = False) -> Tui:
        """The text stays absent for the whole window (>= 1 capture; default 0.04 * E2E_TIMEOUT, E2E_REJECT_WINDOW
        overrides). Only meaningful after a POSITIVE SCREEN barrier (wait_for) proved the app reached the state in which
        the text would show; a forge-log wait is not one (the call is logged before tuicr renders its answer)."""
        for _ in range(max(1, math.floor(window * 10 + 0.999))):
            scr = self._live(f"rejecting /{pattern}/", allow_dead)
            if grep(pattern, scr):
                raise Failure(f"unexpected /{pattern}/ on screen in '{self.name}':", scr)
            time.sleep(0.1)
        return self

    def check(self, pattern: str, message: str, allow_dead: bool = False) -> Tui:
        """The current screen (no polling) has a matching line. For state a wait already proved."""
        scr = self._live(f"checking /{pattern}/", allow_dead)
        if not grep(pattern, scr):
            raise Failure(f"{message} (no /{pattern}/ on screen)", scr)
        return self

    def settle(self, quiet: float = SETTLE_QUIET, timeout: float = TIMEOUT, allow_dead: bool = False) -> Tui:
        """Wait until the screen has not changed for `quiet` seconds (polls every 50 ms). For "the UI finished reacting"
        when nothing specific is to appear; prefer wait_for. A dead pane has "settled" for good, so it fails."""
        what = "settling"
        need, stable, prev = max(1, int(quiet * 20 + 0.5)), 0, self._live(what, allow_dead)
        for _ in range(max(1, int(timeout * 20))):
            time.sleep(0.05)
            cur = self._live(what, allow_dead)
            if cur == prev:
                stable += 1
                if stable >= need:
                    return self
            else:
                stable, prev = 0, cur
        raise Failure(f"screen of '{self.name}' never settled within {timeout:g}s")

    def wait_screen_change(self, before: str, timeout: float = TIMEOUT, allow_dead: bool = False) -> Tui:
        """The screen differs from one captured earlier (barrier for "the app reacted"); timeout may be fractional.
        Changing to the screen of a dead process is not a reaction: it fails."""
        for _ in range(polls(timeout, 0.05)):
            if self._live("waiting for the screen to change", allow_dead) != before:
                return self
            time.sleep(0.05)
        raise Failure(f"screen of '{self.name}' did not change within {timeout:g}s")

    def wait_exit(self, timeout: float = TIMEOUT, status: int | str = 0) -> Tui:
        """Wait for the process to end; its status must match (default 0; "any" accepts every status)."""
        for _ in range(polls(timeout, 0.1)):
            if self._display("#{pane_dead}") == "1":
                st = self.exit_status()
                if status == "any" or st == str(status):
                    self.exit_ok = True
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

    def comment(self, text: str, key: str = "c", timeout: float = TIMEOUT) -> Tui:
        """Open the comment editor with `key` (c line, C file, ;c review), type text literally, submit with Enter, and
        wait for evidence the comment was SAVED (see wait_saved_comment). The typed text on screen proves nothing: the
        editor shows it before Enter is handled."""
        self.keys(key).wait_for("^ COMMENT ", timeout)
        self.type(text).keys("Enter")
        return self.wait_saved_comment(text, timeout)

    def _saved_comments(self) -> list[str]:
        """Comment texts in every session file this TUI has saved (a file being replaced is skipped)."""
        out: list[str] = []
        for f in self.s.session_files(self.name):
            try:
                sess = json.loads(f.read_text())
            except (OSError, ValueError):
                continue
            out += [c.get("content") for c in sess.get("review_comments") or []]
            for fr in (sess.get("files") or {}).values():
                out += [c.get("content") for c in fr.get("file_comments") or []]
                out += [c.get("content") for cs in (fr.get("line_comments") or {}).values() for c in cs]
        return out

    def wait_saved_comment(self, text: str, timeout: float = TIMEOUT) -> Tui:
        """The comment editor closed with `text` saved: COMMENT mode and its `Enter:save` hint are gone, the app is
        back in NORMAL mode with no error on the status line, and the autosaved session holds a comment `text`."""
        self.wait_gone(r"^ COMMENT |Enter:save", timeout).wait_for("^ NORMAL ", timeout)
        scr = self._live("checking the comment was saved")
        if grep(r"Error:|autosave failed|Comment cannot be empty", scr):
            raise Failure(f"'{self.name}': the comment was not saved", scr)
        for _ in range(polls(timeout, 0.1)):
            if text in self._saved_comments():
                return self
            time.sleep(0.1)
        raise Failure(
            f"'{self.name}': no saved session holds the comment {text!r} within {timeout:g}s",
            "\n".join(map(str, self._saved_comments())),
        )

    def cursor_on(self, pattern: str, steps: int = 80, allow_dead: bool = False) -> Tui:
        """Press j until the diff cursor row (▶, outside the file tree) matches the regex (at most `steps` presses)."""
        what = f"moving the cursor to /{pattern}/"
        scr = self._live(what, allow_dead)
        for _ in range(steps):
            cur = "\n".join(ln for ln in self.diff_pane(scr).split("\n") if "▶" in ln)
            if grep(pattern, cur):
                return self
            self.keys("j")
            try:  # the cursor moving redraws; at the end of the diff it cannot move, so stop waiting after 0.5 s
                self.wait_screen_change(scr, 0.5, allow_dead)
            except (NoPane, DeadPane):
                raise
            except Failure:
                pass
            self.settle(0.1, allow_dead=allow_dead)  # the first changed frame can be partial: judge a settled screen
            scr = self._live(what, allow_dead)
        raise Failure(f"cursor never reached /{pattern}/", scr)

    # ---- fake forge call log (one JSON object per line) ------------------------------------------------------------
    # A record is a CALL ({"tool", "argv", "stdin"}) or a NOTE about one (it also has "note": UNEXPECTED..., HELD...,
    # RELEASED..., injected failure). Every reader below sees calls only unless notes=True (then notes only), and only
    # records logged after `since` (a forge_mark()). Regexes match call_text(): stdin decoded, non-ASCII kept.
    def forge_log_text(self) -> str:
        return self.forge_log.read_text() if self.forge_log.exists() else ""

    def forge_mark(self) -> int:
        """A position in the forge log: pass it as since= so a check only sees calls made after this step."""
        return len(read_forge_log(self.forge_log))

    def forge_calls(
        self, where: Callable[[Call], object] | None = None, since: int = 0, notes: bool = False
    ) -> list[Call]:
        """Parsed records (see read_forge_log: tool, argv, stdin, body = stdin decoded, method, index, note?) logged at
        or after `since`, calls only (notes=True: notes only), filtered by the predicate `where`."""
        return [
            c
            for c in read_forge_log(self.forge_log)
            if c["index"] >= since and ("note" in c) == notes and (where is None or where(c))
        ]

    def _matching(self, pattern: str, since: int, notes: bool) -> list[Call]:
        rx = re.compile(pattern)
        return self.forge_calls(lambda c: rx.search(call_text(c)), since, notes)

    def _log_evidence(self) -> str:
        return "\n".join(call_text(c) for c in read_forge_log(self.forge_log))

    def forge_count(self, pattern: str, since: int = 0, notes: bool = False) -> int:
        """Number of logged calls (notes=True: notes) whose call_text matches the regex."""
        return len(self._matching(pattern, since, notes))

    def forge_expect(self, pattern: str, since: int = 0, notes: bool = False) -> Tui:
        """A logged call matches the regex now (no polling: wait_forge_log first if the call is asynchronous)."""
        if not self.forge_count(pattern, since, notes):
            raise Failure(f"forge log lacks /{pattern}/ (since record {since})", self._log_evidence())
        return self

    def forge_reject(self, pattern: str, since: int = 0, notes: bool = False) -> Tui:
        if self.forge_count(pattern, since, notes):
            raise Failure(f"forge log has /{pattern}/ (since record {since})", self._log_evidence())
        return self

    def wait_forge_log(
        self, pattern: str, timeout: float = TIMEOUT, count: int = 1, since: int = 0, notes: bool = False
    ) -> Tui:
        """At least `count` logged calls match the regex (a barrier for the call, NOT for the screen: the call is
        logged before tuicr renders the answer)."""
        for _ in range(polls(timeout, 0.1)):
            if self.forge_count(pattern, since, notes) >= count:
                return self
            time.sleep(0.1)
        raise Failure(f"fewer than {count} forge calls matching /{pattern}/ within {timeout:g}s", self._log_evidence())

    def mutations(self, tool: str | None = None, since: int = 0) -> list[Call]:
        """Every call that changes forge state: an `api` call whose method (any spelling) is not GET, a GraphQL
        `mutation`, or a `pr approve`/`pr review`/`pr comment`/`pr merge` command."""

        def mutating(c: Call) -> bool:
            a = c["argv"]
            if tool is not None and c["tool"] != tool:
                return False
            if c["method"] not in (None, "GET"):
                return True
            if any(x.startswith("query=") and x[len("query=") :].lstrip().startswith("mutation") for x in a):
                return True
            return a[:1] == ["pr"] and a[1:2] in (["approve"], ["review"], ["comment"], ["merge"])

        return self.forge_calls(mutating, since)

    def posts(self, since: int = 0) -> list[dict]:
        """Every review POST in the fake-gh log (method in any spelling): {"path": "repos/...", "payload": <stdin
        json>}."""
        out = []
        for c in self.forge_calls(lambda c: c["tool"] == "gh" and c["method"] == "POST", since):
            path = next((x for x in c["argv"] if x.startswith("repos/")), None)
            if path is None:
                raise Failure("gh POST without a repos/ path", call_text(c))
            out.append({"path": path, "payload": json.loads(c["stdin"] or "null")})
        return out

    def forge_posts(self, tool: str, since: int = 0) -> list[dict]:
        """Every POST-like call of `tool`: {"argv": [...], "payload": <body json or None>}. A call counts when its
        method (any spelling) is POST or it is a `pr approve`. For PUT/PATCH/DELETE and GraphQL mutations too use
        mutations()."""
        out = []
        for c in self.forge_calls(
            lambda c: c["tool"] == tool and (c["method"] == "POST" or c["argv"][:2] == ["pr", "approve"]), since
        ):
            a = c["argv"]
            body = c.get("stdin") or (a[a.index("--input") + 1] if "--input" in a else None)  # bkt: body as argument
            out.append({"argv": a, "payload": json.loads(body) if body else None})
        return out

    # ---- teardown --------------------------------------------------------------------------------------------------
    def _note_death(self) -> None:
        """Record in the scenario a process that exited without a wait_exit accepting it (any status)."""
        if self.exit_ok or self.retired:
            return
        with contextlib.suppress(Failure):
            if self._display("#{pane_dead}") == "1":
                self.s.deaths.append(f"'{self.name}' exited with status {self.exit_status() or '?'} unobserved")

    def stop(self) -> None:
        """Note an unobserved exit, keep a screenshot and the forge log in $E2E_OUT, kill the session."""
        self._note_death()
        with contextlib.suppress(Failure):
            (OUT / f"{self.name}.screen.txt").write_text(self.screen() + "\n")
        if self.forge_log.exists() and self.forge_log.stat().st_size:
            shutil.copy(self.forge_log, OUT / f"{self.name}.forge.log")
        tmux("kill-session", "-t", self.session, check=False)
