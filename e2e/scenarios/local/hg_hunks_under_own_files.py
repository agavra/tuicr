#!/usr/bin/env python3
# with files in several `hg status` classes (modified z, added a, removed m),
# each file's hunk must be shown under its own file. Needs real Mercurial (E2E_HG_DIR/hg), not
# Sapling: SKIP without it, FAIL when E2E_REQUIRE_HG=1.
import re

from e2e import Scenario, repos, scenario


@scenario
def main(s: Scenario) -> None:
    s.use_hg()
    repo = repos.hg_repo(s.dir / "repo")
    tui = s.start(cwd=repo)
    tui.wait_for("Unstaged changes")
    tui.keys("Enter")
    tui.wait_for(r"z\.txt")
    tui.wait_for(r"(═══|---).*a\.txt")
    # each file section header must be followed by its own content (a: added, m: removed, z: z_old->z_new)
    scr = tui.screen()
    pairing: list[str] = []
    f = ""
    for line in scr.split("\n"):
        for header, name in (
            (r"a\.txt \[", "a"),
            (r"m\.txt \[", "m"),
            (r"z\.txt \[", "z"),
        ):
            if re.search(header, line):
                f = name
        for word in ("a_added", "m_old", "z_new", "z_old"):
            if word in line:
                pairing.append(f"{word} in {f}")
    print("\n".join(pairing))
    (s.dir / "pairing.txt").write_text("".join(p + "\n" for p in pairing))
    assert all(want in pairing for want in ("a_added in a", "m_old in m", "z_new in z")), (
        f"hunks paired with the wrong files\n{scr}"
    )
    tui.quit()
