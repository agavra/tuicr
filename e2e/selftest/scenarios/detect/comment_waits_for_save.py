#!/usr/bin/env python3
# Runner self-test (expect PASS): Tui.comment() waits for evidence the comment was SAVED (editor closed, NORMAL mode, no
# error, the autosaved session holds it), not for the typed text the editor echoes; and it types the text literally
# (the word Enter is text, not a key).
from e2e import Scenario, must_fail, repos, scenario

EXPECTED = "PASS"  # asserted by e2e/selftest/run.py

# A stand-in for tuicr's comment editor: shows the COMMENT chip, reads one line, then behaves as told.
FAKE = r"""
import json, os, sys, time
mode = sys.argv[1]
print(" COMMENT    Enter:save", flush=True)
line = sys.stdin.readline().rstrip("\n")[1:]  # drop the echoed key that opened the editor
done = "\x1b[2J\x1b[H NORMAL                       "
if mode == "saves" and line == "Enter literally":
    d = os.path.join(os.environ["XDG_DATA_HOME"], "tuicr", "reviews", "sessions")
    os.makedirs(d)
    json.dump({"review_comments": [{"content": line}], "files": {}}, open(os.path.join(d, "s.json"), "w"))
    print(done + "Review comment added", flush=True)
elif mode == "error":
    print(done + "Error: Could not save comment", flush=True)
elif mode == "nosave":  # the editor closes, but nothing reaches the session file
    print(done + "Review comment added", flush=True)
else:  # the typed text is on screen, the editor stays open
    print("echo: " + line, flush=True)
time.sleep(60)
"""


@scenario
def main(s: Scenario) -> None:
    fake = repos.write(s.dir / "editor.py", FAKE)
    tuis = {}
    for mode in ("saves", "stuck", "error", "nosave"):
        tuis[mode] = s.start_cmd(["python3", str(fake), mode], name=f"{s.name}_{mode}")
        tuis[mode].wait_for("COMMENT", 5)  # the fake is ready: the key comment() presses is echoed below its chip
    tuis["saves"].comment("Enter literally", timeout=5)  # sent as keys, "Enter" would end the line early: no save
    must_fail(lambda: tuis["stuck"].comment("never saved", timeout=1), "comment() passed with the editor still open")
    tuis["stuck"].check("echo: never saved", "the typed text should be on screen (it proves nothing)")
    must_fail(lambda: tuis["error"].comment("rejected", timeout=1), "comment() passed although saving failed")
    must_fail(lambda: tuis["nosave"].comment("lost", timeout=1), "comment() passed with nothing in the session")
