#!/usr/bin/env python3
# Covers #886: `review list --repo forge:<host>/<owner…>/<repo>` (the path tuicr stores for PR
# sessions) keeps every owner segment. For a Gerrit project `platform/build/tools` on a host the selector parser
# cannot tell is Gerrit (no "gerrit" in the name), the generic last-two-segments rule used to give owner `build`, so
# the selector never matched the saved PR session. Also: an `owner/.git` repo segment is not a selector.
import json

from e2e import Scenario, scenario


@scenario
def main(s: Scenario) -> None:
    host = "review.example.test"
    tui = s.gerrit_open("root", host=host, project="platform/build/tools")
    tui.wait_for("a.txt", 20)
    tui.settle().keys("}").cursor_on("line two")
    tui.comment("identity note")
    tui.save()
    all_sessions = s.cli("review", "list", "--all")
    print("--- all sessions")
    print(all_sessions)
    slug = json.loads(all_sessions)[0]["slug"]
    print(f"slug={slug}")
    r = s.cli_result("review", "list", "--repo", f"forge:{host}/platform/build/tools")
    print(f"--- forge:{host}/platform/build/tools\n{r.out}")
    assert r.rc == 0, f"review list --repo forge:{host}/platform/build/tools exited {r.rc}"
    assert slug in r.out, f"forge:{host}/platform/build/tools did not select the saved Gerrit PR session"
    bad = s.cli_result(
        "review", "list", "--repo", f"forge:{host}/platform/build/.git"
    )  # may be an error: only "no session selected" matters
    print(f"--- forge:{host}/platform/build/.git\n{bad.out}")
    assert slug not in bad.out, "owner/.git selector matched a session"
    tui.quit()
