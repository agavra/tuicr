"""Shared core of the fake forge CLIs (gh, glab, tea, bkt).

An adapter module (`fakeforge/<tool>.py`) exposes `handle(fake)`. The core owns
everything that is not forge-specific:

* the call log: one JSON object per invocation in $E2E_FORGE_LOG
  ({"tool", "argv", "stdin", "note"?}); scenarios assert on it and the harness
  fails a scenario if any call carries an "UNEXPECTED" note;
* fixture data lookup under $E2E_DATA/<forge>/ with variant overlays;
* small per-run state files next to the log (stateful forges, e.g. drafts);
* argument helpers and the exit conventions (97 = unexpected call).
"""

from __future__ import annotations

import json
import os
import re
import sys
import time
from typing import Any, NoReturn

UNEXPECTED_RC = 97
FORGE_DIRS = {"gh": "github", "glab": "gitlab", "tea": "gitea", "bkt": "bitbucket"}


class Fake:
    def __init__(self, tool: str, argv: list[str]) -> None:
        self.tool, self.argv = tool, argv
        self.data_dir = os.path.join(os.environ["E2E_DATA"], FORGE_DIRS[tool])
        self.log = os.environ.get("E2E_FORGE_LOG")
        self.stdin: str | None = None

    # ---- call log -------------------------------------------------------
    def start(self, read_stdin: bool = False) -> None:
        """Read stdin if the call carries a body, then log the call."""
        if read_stdin:
            self.stdin = sys.stdin.read()
        self.record()

    def record(self, note: str | None = None) -> None:
        if self.log:
            entry: dict[str, Any] = {"tool": self.tool, "argv": self.argv, "stdin": self.stdin}
            if note:
                entry["note"] = note
            with open(self.log, "a") as f:
                f.write(json.dumps(entry) + "\n")

    # ---- exits ----------------------------------------------------------
    def out(self, body: Any) -> NoReturn:
        sys.stdout.write(body if isinstance(body, str) else json.dumps(body))
        sys.exit(0)

    def fail(self, message: str, rc: int = 1) -> NoReturn:
        """A simulated forge/network failure (the scenario asked for it)."""
        sys.stderr.write(message + "\n")
        sys.exit(rc)

    def unexpected(self, message: str) -> NoReturn:
        """A call the scenario's fixture does not cover: logged, exit 97."""
        sys.stderr.write(f"FAKE {self.tool}: UNEXPECTED CALL: {message}\nargv={self.argv}\n")
        self.record("UNEXPECTED: " + message)
        sys.exit(UNEXPECTED_RC)

    # ---- argument helpers -------------------------------------------------
    def flag(self, name: str) -> str | None:
        return self.argv[self.argv.index(name) + 1] if name in self.argv else None

    def positional(self, args: list[str], value_flags: tuple[str, ...] | list[str]) -> list[str]:
        """Positional arguments of `args`, skipping each flag in value_flags and its value."""
        out: list[str] = []
        i = 0
        while i < len(args):
            if args[i] in value_flags:
                i += 2
                continue
            out.append(args[i])
            i += 1
        return out

    @staticmethod
    def page(query: str) -> str:
        m = re.search(r"(?:^|&)page=(\d+)", query)
        return m.group(1) if m else "1"

    # ---- fixture data -----------------------------------------------------
    def variant(self) -> str | None:
        """$E2E_VARIANT_FILE (if it exists) overrides $E2E_VARIANT: a scenario can change what the forge serves."""
        f = os.environ.get("E2E_VARIANT_FILE")
        if f and os.path.exists(f):
            return open(f).read().strip() or None
        return os.environ.get("E2E_VARIANT")

    def data_path(self, rel: str) -> str:
        """data/<forge>/<rel>, shadowed by data/<forge>/<variant>/<rel> when present."""
        v = self.variant()
        if v and os.path.exists(os.path.join(self.data_dir, v, rel)):
            return os.path.join(self.data_dir, v, rel)
        return os.path.join(self.data_dir, rel)

    def load(self, rel: str) -> str:
        with open(self.data_path(rel)) as f:
            return f.read()

    def load_json(self, rel: str) -> Any:
        return json.loads(self.load(rel))

    def has(self, rel: str) -> bool:
        return os.path.exists(self.data_path(rel))

    def serve(self, rel: str, default: Any = None) -> NoReturn:
        """Print fixture `rel` and exit 0; `default` if the fixture is absent; else an unexpected call."""
        if self.has(rel):
            self._hold(rel)
            self.out(self.load(rel))
        if default is not None:
            self.out(default)
        self.unexpected("no fixture data " + self.data_path(rel))

    def _hold(self, rel: str) -> None:
        """$E2E_HOLD_DIR: data calls made while <dir>/gate exists block until <dir>/release (async-race scenarios)."""
        hold = os.environ.get("E2E_HOLD_DIR")
        if (
            hold
            and os.path.exists(os.path.join(hold, "gate"))
            and rel in ("files.json", "commits.json", "pr_diff.patch")
        ):
            self.record("HELD " + rel)
            for _ in range(600):
                if os.path.exists(os.path.join(hold, "release")):
                    break
                time.sleep(0.1)
            self.record("RELEASED " + rel)

    # ---- per-run state (next to the log) ----------------------------------
    def state_path(self, name: str) -> str:
        assert self.log, "E2E_FORGE_LOG is not set"
        return os.path.join(os.path.dirname(self.log), name)

    def state_json(self, name: str, default: Any) -> Any:
        p = self.state_path(name)
        return json.load(open(p)) if os.path.exists(p) else default

    def save_state(self, name: str, value: Any) -> None:
        json.dump(value, open(self.state_path(name), "w"))
