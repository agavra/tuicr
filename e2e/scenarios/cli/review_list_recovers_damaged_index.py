#!/usr/bin/env python3
# Persistence write-back (p_persist_writeback review): a corrupt or missing `index.json` is rebuilt from sessions/
# instead of reading as empty, so `review list`/`add` keep working and other sessions are not dropped.
import json

from e2e import Scenario, grep, scenario


@scenario
def main(s: Scenario) -> None:
    tui = s.diff()
    repo = s.dir / "repo"
    tui.keys("r").wait_for("Files · 1/8").save().quit()
    found = sorted(p for top in ("data", "home") for p in (s.dir / top).rglob("index.json"))
    assert found, "no index.json written"
    idx = found[0]

    def listing() -> list[dict[str, object]]:
        return json.loads(s.cli("review", "list", "--repo", repo))

    assert len(listing()) == 1, "expected one session before damage"
    idx.write_text("{corrupt\n")
    assert len(listing()) == 1, "corrupt index.json hid the session"
    corrupt = idx.with_name(idx.name + ".corrupt")
    assert corrupt.exists(), "corrupt index.json was not kept as index.json.corrupt"
    idx.unlink()
    assert len(listing()) == 1, "missing index.json hid the session"
    slug = listing()[0]["slug"]
    s.cli("review", "add", "--repo", repo, "--session", slug, "after rebuild")
    assert grep("after rebuild", s.cli("review", "comments", "--repo", repo, "--session", slug)), (
        "add after rebuild lost"
    )
