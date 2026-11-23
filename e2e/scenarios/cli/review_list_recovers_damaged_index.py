#!/usr/bin/env python3
# Persistence write-back (p_persist_writeback review): a corrupt or missing `index.json` is rebuilt from sessions/
# instead of reading as empty, so `review list`/`add` keep working and other sessions are not dropped.
import json

from e2e import Scenario, scenario


@scenario
def main(s: Scenario) -> None:
    # Two sessions (two ranges) in one data dir: an index rebuilt from only the session being written would drop one.
    tui = s.diff()
    repo = s.dir / "repo"
    tui.keys("r").wait_for("Files · 1/8").save().quit()
    tui = s.start("-r", "HEAD~1..HEAD", cwd=repo, keep=True)
    tui.wait_for("Files · 0/2").keys("r").wait_for("Files · 1/2").save().quit()
    found = sorted(p for top in ("data", "home") for p in (s.dir / top).rglob("index.json"))
    assert len(found) == 1, f"expected one index.json, found {found}"
    idx = found[0]

    def slugs() -> set[str]:
        out = json.loads(s.cli("review", "list", "--repo", repo))
        assert isinstance(out, list), out
        return {x["slug"] for x in out}

    def indexed() -> int:
        """Session entries in index.json (it must exist and parse)."""
        assert idx.exists(), "index.json was not written back"
        return sum(len(v) for v in json.loads(idx.read_text())["entries"].values())

    both = slugs()
    assert len(both) == 2, f"expected two sessions before damage, got {both}"
    idx.write_text("{corrupt\n")
    assert slugs() == both, "corrupt index.json hid a session"
    corrupt = idx.with_name(idx.name + ".corrupt")
    assert corrupt.exists(), "corrupt index.json was not kept as index.json.corrupt"
    assert indexed() == 2, f"rebuilt index.json lacks a session:\n{idx.read_text()}"
    idx.unlink()
    assert slugs() == both, "missing index.json hid a session"
    idx.unlink()  # the listing rebuilt it; remove it again so the add below has to rebuild it itself
    slug = sorted(both)[0]
    s.cli("review", "add", "--repo", repo, "--session", slug, "after rebuild")
    assert indexed() == 2, f"add rewrote index.json without the other session:\n{idx.read_text()}"
    assert slugs() == both, "add after rebuild dropped the other session"
    contents = [x["content"] for x in json.loads(s.cli("review", "comments", "--repo", repo, "--session", slug))]
    assert contents == ["after rebuild"], f"add after rebuild lost: {contents}"
