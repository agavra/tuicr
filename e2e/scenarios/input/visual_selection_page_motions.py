#!/usr/bin/env python3
# Covers #996: in visual mode, Ctrl-d/Ctrl-u and Ctrl-f/Ctrl-b/PageDown/PageUp move by half and
# full pages and extend the selection. Before, only j/k and the arrows did anything there.
from e2e import Scenario, repos, scenario


@scenario
def main(s: Scenario) -> None:
    repo = repos.empty_repo(s.dir / "repo")
    repos.write(repo / "lines.txt", "seed\n")
    repos.git(repo, "add", "-A")
    repos.git(repo, "commit", "-qm", "initial")
    repos.write(repo / "lines.txt", "".join(f"row-{n:03}\n" for n in range(1, 201)))

    clipboard = s.fake_clipboard()
    tui = s.start(cwd=repo, keep=True)
    tui.wait_for("Unstaged changes").keys("Enter")
    tui.wait_for("Files · 0/1")

    def selected() -> int:
        tui.keys("y").settle()
        rows = [line for line in clipboard.text().splitlines() if line.startswith("row-")]
        assert rows[0] == "row-001", f"selection lost its anchor: {rows[:3]}"
        return len(rows)

    def after(*keys: str) -> int:
        tui.keys("g", "g").settle()
        tui.cursor_on(r"row-001")
        tui.keys("v", *keys).settle()
        return selected()

    half = after("C-d")
    assert half > 2, f"Ctrl-d did not extend the selection: {half} rows"
    for key in ("C-f", "PageDown"):
        full = after(key)
        assert full > half, f"{key} did not extend by a full page: {full} rows"
        for back in ("C-b", "PageUp"):
            assert after(key, back) < full, f"{back} did not shrink the selection after {key}"
    assert after("C-d", "C-u") < half, "Ctrl-u did not shrink the selection"
    tui.quit()
