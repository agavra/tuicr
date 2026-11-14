#!/usr/bin/env python3
# Covers #906: CJK in the header title and in a commit row's branch chip are measured by display width.
# Baseline: chip `[ブランチ]` is cut by byte count (`[ブラ...]`) and its row overflows the box (`· just n│`); the header measures
# by chars so the repo@branch slug is clipped early.
from e2e import Scenario, repos, scenario


@scenario
def main(s: Scenario) -> None:
    r = repos.git_repo(s.dir / "日本語リポジトリ")
    repos.git(r, "branch", "機能/日本語ブランチ名前長い")
    repos.git(r, "checkout", "-q", "機能/日本語ブランチ名前長い")
    repos.git(r, "branch", "ブランチ", "HEAD~1")
    tui = s.start("-r", "HEAD~2..HEAD", cwd=r, cols=100)
    tui.wait_for("Commits").settle(0.4)
    rows = tui.screen().split("\n")
    assert "[ブランチ]" in rows[3], f"branch chip truncated by bytes\n{rows[3]}"
    assert "just now│" in rows[3], f"commit row overflows its box\n{rows[3]}"
    assert "ブランチ名前長い/c" in rows[0], (
        f"header title measured by chars: slug clipped early\n{rows[0]}"
    )
    tui.quit()
