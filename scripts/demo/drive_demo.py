#!/usr/bin/env python3
"""Drive the tuicr README demo inside a pseudo-terminal.

This script is intended to run under asciinema. It launches tuicr in the
generated fixture repository, sends the demo keystrokes with readable pacing,
then proves `:clip` by printing a short `pbpaste` preview.
"""

from __future__ import annotations

import argparse
import base64
import os
import re
import shutil
import subprocess
import sys
import time
from pathlib import Path

try:
    import pexpect
except ImportError as exc:  # pragma: no cover - exercised by shell checks
    raise SystemExit(
        "error: missing Python module 'pexpect'. Install it with: "
        "python3 -m pip install pexpect"
    ) from exc


SUGGESTION = "Consider covering the retry path with a regression test."
ISSUE = "This timeout is too aggressive for slow CI runners."

# Starship-ish prompt for the fake shell session that bookends the recording.
SHELL_PROMPT = (
    "\x1b[1;36m~/code/auth-service\x1b[0m"
    " \x1b[2mon\x1b[0m"
    " \x1b[1;35mmain\x1b[0m"
    " \x1b[1;32m❯\x1b[0m "
)


class OutputCapture:
    def __init__(self, stream):
        self.stream = stream
        self.parts: list[str] = []
        self.expect_pos = 0

    def write(self, data: str) -> int:
        self.parts.append(data)
        return self.stream.write(data)

    def flush(self) -> None:
        self.stream.flush()

    def text(self) -> str:
        return "".join(self.parts)

    def find_after_last_expect(self, needle: str) -> int:
        idx = self.text().find(needle, self.expect_pos)
        return idx

    def advance_past(self, idx: int, needle_len: int) -> None:
        self.expect_pos = idx + needle_len


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--tuicr", required=True, help="Path to the tuicr binary")
    parser.add_argument("--fixture", required=True, help="Path to the generated fixture repo")
    parser.add_argument("--cols", type=int, default=120, help="PTY columns")
    parser.add_argument("--rows", type=int, default=34, help="PTY rows")
    parser.add_argument("--typing-delay", type=float, default=0.018)
    parser.add_argument("--step-delay", type=float, default=0.6)
    return parser.parse_args()


def line_number(path: Path, needle: str) -> int:
    for idx, line in enumerate(path.read_text().splitlines(), start=1):
        if needle in line:
            return idx
    raise SystemExit(f"error: could not find {needle!r} in {path}")


def pretype(capture: "OutputCapture", command: str, delay: float) -> None:
    """Render a fake shell prompt, then visibly type the given command."""
    capture.write("\r\n" + SHELL_PROMPT)
    capture.flush()
    time.sleep(0.45)
    for ch in command:
        capture.write(ch)
        capture.flush()
        time.sleep(delay)
    capture.write("\r\n")
    capture.flush()
    time.sleep(0.18)


def drain_output(child: pexpect.spawn) -> None:
    # pexpect only mirrors child output to logfile_read during read calls.
    # Without a drain after each send, tuicr's redraws accumulate in the PTY
    # buffer and get mirrored in a burst at the next expect(), which collapses
    # all per-keystroke frames into a single moment in the asciinema cast.
    try:
        while True:
            child.read_nonblocking(size=16384, timeout=0)
    except (pexpect.TIMEOUT, pexpect.EOF):
        pass


def sleep_and_drain(child: pexpect.spawn, delay: float) -> None:
    end = time.time() + delay
    while True:
        remaining = end - time.time()
        if remaining <= 0:
            break
        try:
            child.read_nonblocking(size=16384, timeout=min(remaining, 0.05))
        except pexpect.TIMEOUT:
            pass
        except pexpect.EOF:
            return


def send_text(child: pexpect.spawn, text: str, delay: float) -> None:
    for char in text:
        child.send(char)
        sleep_and_drain(child, delay)


def send_keys(child: pexpect.spawn, keys: str, delay: float) -> None:
    child.send(keys)
    sleep_and_drain(child, delay)


def send_unbracketed_paste(child: pexpect.spawn, text: str) -> None:
    # Mimic Shift+Cmd+V: drip the bytes in line by line instead of one large
    # bracketed-paste chunk. Claude's TUI renders each line as the bytes
    # arrive, which is what we want on camera.
    lines = text.split("\n")
    for i, line in enumerate(lines):
        if line:
            child.send(line)
            sleep_and_drain(child, 0.04)
        if i < len(lines) - 1:
            child.send("\n")
            sleep_and_drain(child, 0.04)


def expect(
    child: pexpect.spawn,
    pattern: str,
    capture: "OutputCapture | None" = None,
    timeout: int = 20,
) -> None:
    # sleep_and_drain pulls bytes off pexpect's internal buffer so they show up
    # in the asciinema cast at the right time, which means child.expect() can
    # no longer see them. Search the mirrored capture first; only fall back to
    # child.expect() when the pattern hasn't arrived yet.
    if capture is not None:
        idx = capture.find_after_last_expect(pattern)
        if idx >= 0:
            capture.advance_past(idx, len(pattern))
            return
    deadline = time.time() + timeout
    while True:
        sleep_and_drain(child, 0.05)
        if capture is not None:
            idx = capture.find_after_last_expect(pattern)
            if idx >= 0:
                capture.advance_past(idx, len(pattern))
                return
        if time.time() >= deadline:
            tail = capture.text()[-2000:] if capture is not None else ""
            raise SystemExit(
                f"error: timed out waiting for {pattern!r}\n\nrecent output:\n{tail}"
            )


def add_line_comment(
    child: pexpect.spawn,
    text: str,
    tab_count: int,
    typing_delay: float,
    step_delay: float,
    capture: "OutputCapture",
) -> None:
    send_keys(child, "c", step_delay)
    expect(child, "Type your comment", capture)
    for _ in range(tab_count):
        send_keys(child, "\t", 0.3)
    send_text(child, text, typing_delay)
    send_keys(child, "\r", step_delay)
    expect(child, "Comment added to line", capture)


def read_pasteboard() -> str:
    try:
        result = subprocess.run(
            ["pbpaste"],
            check=True,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        )
    except (FileNotFoundError, subprocess.CalledProcessError):
        return ""
    return result.stdout


def expected_comments_missing(content: str) -> list[str]:
    missing = [text for text in (SUGGESTION, ISSUE) if text not in content]
    return missing


def copy_to_pasteboard(content: str) -> bool:
    try:
        subprocess.run(
            ["pbcopy"],
            check=True,
            text=True,
            input=content,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        )
    except (FileNotFoundError, subprocess.CalledProcessError):
        return False
    return True


def decode_latest_osc52(output: str) -> str | None:
    matches = re.findall(r"\x1b\]52;c;([A-Za-z0-9+/=]+)(?:\x07|\x1b\\\\)", output)
    if not matches:
        return None
    try:
        return base64.b64decode(matches[-1]).decode("utf-8")
    except Exception:
        return None


def resolve_clipboard_markdown(captured_output: str) -> str:
    content = read_pasteboard()
    missing = expected_comments_missing(content)
    if missing:
        # Headless pseudo-terminals can make tuicr fall back to OSC 52. Mirror
        # that payload into the pasteboard when possible, otherwise use the
        # decoded OSC 52 content directly so we still have agent-ready markdown.
        osc52_content = decode_latest_osc52(captured_output)
        if osc52_content is not None and not expected_comments_missing(osc52_content):
            copy_to_pasteboard(osc52_content)
            content = read_pasteboard()
            if expected_comments_missing(content):
                content = osc52_content
                missing = []
            else:
                missing = []

    if missing:
        raise SystemExit(
            "error: clipboard did not contain the expected tuicr review comments: "
            + ", ".join(repr(text) for text in missing)
        )
    return content


def run_claude_paste(
    markdown: str,
    capture: "OutputCapture",
    env: dict,
    cols: int,
    rows: int,
) -> None:
    """Spawn `claude --bare`, paste the markdown via bracketed paste, exit."""
    claude_bin = shutil.which("claude")
    if claude_bin is None:
        capture.write("\r\n[claude not found on PATH]\r\n")
        capture.flush()
        return

    claude = pexpect.spawn(
        claude_bin,
        ["--bare"],
        env=env,
        dimensions=(rows, cols),
        encoding="utf-8",
        codec_errors="replace",
        timeout=30,
    )
    claude.logfile_read = capture

    try:
        # Wait for the input box to render. Claude draws a Unicode-bordered
        # prompt; any of these characters indicates the UI is ready.
        try:
            claude.expect(r"[╭╰>│]", timeout=15)
        except (pexpect.TIMEOUT, pexpect.EOF):
            pass
        sleep_and_drain(claude, 0.9)

        # Send the markdown WITHOUT bracketed-paste markers. With markers,
        # Claude collapses long pastes into "[Pasted text +N lines]". Without
        # them (the equivalent of Shift+Cmd+V in the terminal), Claude treats
        # the bytes as raw input and renders every line inline.
        send_unbracketed_paste(claude, markdown.rstrip())
        sleep_and_drain(claude, 4.5)

        # Quit without submitting: two Ctrl-C presses (the first shows the
        # "press again to exit" hint, the second exits).
        claude.send("\x03")
        sleep_and_drain(claude, 0.5)
        claude.send("\x03")
        try:
            claude.expect(pexpect.EOF, timeout=5)
        except (pexpect.TIMEOUT, pexpect.EOF):
            pass
    finally:
        if claude.isalive():
            claude.terminate(force=True)


def main() -> int:
    args = parse_args()
    fixture = Path(args.fixture).resolve()
    tuicr = Path(args.tuicr).resolve()
    issue_line = line_number(fixture / "src" / "auth.rs", "Duration::from_secs(15)")

    env = os.environ.copy()
    env.setdefault("TERM", "xterm-256color")
    env.setdefault("COLORTERM", "truecolor")
    env.pop("NO_COLOR", None)
    env.setdefault("CLICOLOR_FORCE", "1")
    env.setdefault("FORCE_COLOR", "1")

    capture = OutputCapture(sys.stdout)
    pretype(capture, "tuicr", args.typing_delay)

    child = pexpect.spawn(
        str(tuicr),
        ["--theme", "tokyo-night-storm", "--no-update-check"],
        cwd=str(fixture),
        env=env,
        dimensions=(args.rows, args.cols),
        encoding="utf-8",
        codec_errors="replace",
        timeout=30,
    )
    child.logfile_read = capture

    try:
        expect(child, "Handle rate-limit retry responses", capture)
        expect(child, "Shorten session token timeout", capture)
        sleep_and_drain(child, args.step_delay)

        send_keys(child, " ", args.step_delay)
        send_keys(child, " ", args.step_delay)
        send_keys(child, "\r", args.step_delay)

        # The diff view scrolls in further down via the /search below, so we
        # cannot wait for `should_retry_status` here — it is only visible after
        # the search jumps to it. Just give tuicr time to render the diff.
        sleep_and_drain(child, args.step_delay * 2)

        send_keys(child, "/", 0.25)
        send_text(child, "should_retry", args.typing_delay)
        send_keys(child, "\r", args.step_delay)
        sleep_and_drain(child, args.step_delay)
        add_line_comment(
            child,
            SUGGESTION,
            tab_count=1,
            typing_delay=args.typing_delay,
            step_delay=args.step_delay,
            capture=capture,
        )

        send_keys(child, ":", 0.25)
        send_text(child, str(issue_line), args.typing_delay)
        send_keys(child, "\r", args.step_delay)
        sleep_and_drain(child, args.step_delay)
        add_line_comment(
            child,
            ISSUE,
            tab_count=2,
            typing_delay=args.typing_delay,
            step_delay=args.step_delay,
            capture=capture,
        )

        send_keys(child, ":", 0.25)
        send_text(child, "clip", args.typing_delay)
        send_keys(child, "\r", args.step_delay)
        sleep_and_drain(child, 1.2)

        send_keys(child, ":", 0.25)
        send_text(child, "q!", args.typing_delay)
        send_keys(child, "\r", args.step_delay)
        child.expect(pexpect.EOF, timeout=10)

        markdown = resolve_clipboard_markdown(capture.text())
        pretype(capture, "claude", args.typing_delay)
        run_claude_paste(markdown, capture, env, args.cols, args.rows)
        time.sleep(1.2)
        return 0
    finally:
        if child.isalive():
            child.terminate(force=True)


if __name__ == "__main__":
    raise SystemExit(main())
