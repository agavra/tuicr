#!/usr/bin/env python3
# Covers #893: head keeps moving (fake gh: a new head after every data call).
# Fixed: the open fails with "changed while it was being loaded; try again" and no PR is shown.
# Baseline: opens PR #1 anyway with a mismatched head/diff pair.
from e2e import TUICR_BIN, Scenario, repos, scenario


@scenario
def main(s: Scenario) -> None:
    repo = repos.pr_repo(s.dir / "repo")
    stderr = s.dir / "stderr.txt"
    # stderr (where the exit error goes once the TUI is torn down) is captured to a file
    tui = s.start_cmd(
        ["bash", "-c", '"$0" --no-update-check pr 1 2>"$1"', TUICR_BIN, str(stderr)],
        cwd=repo,
        env={"E2E_HEAD_MODE": "every"},
    )
    tui.wait_exit(20, 1)  # else: PR opened despite the head moving on every request
    err = stderr.read_text()
    assert "changed while it was being loaded; try again" in err, f"no head-moved error:\n{err}"
    # fetched twice (one retry), then gave up
    fetches = tui.forge_count('"pr", "diff"')
    assert fetches == 2, f"expected exactly one retry, got {fetches} fetches"
