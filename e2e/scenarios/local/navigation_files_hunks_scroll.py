#!/usr/bin/env python3
# Local review: file tree, scroll, hunk nav, file nav. Uses a -r range for multi-file/hunk content.
from e2e import Scenario, scenario


@scenario
def main(s: Scenario) -> None:
    tui = s.diff()

    def cur(name: str) -> None:  # diff pane title names the file under the cursor
        tui.wait_for(f"┌ {name} ─+ ")

    tui.expect(r"▢ M README\.md").expect(r"▢ D gone\.txt").expect("▼ src/").expect(r"lib\.rs")
    cur(r"README\.md")
    tui.keys("}")
    cur(r"data\.bin")
    tui.keys("}", "}")
    cur(r"new_name\.txt")
    tui.keys("{")
    cur(r"gone\.txt")
    tui.keys("G")
    cur(r"src/main\.rs")
    tui.keys("g")
    cur(r"README\.md")
    # j moves the cursor row; k moves it back
    tui.keys("j", "j", "j").expect("▶ +3 +first line")
    tui.keys("k").expect("▶ +2 +│")
    # hunk nav inside src/main.rs (single hunk) and across files
    tui.keys("G").keys("[").expect(r"▶.*(@@ -1,3 \+1,3 @@|═══ src/main)")
    # g lands on README's hunk header; ] skips the hunk-less data.bin to gone.txt, then notes_cjk; [ steps back
    tui.keys("g")
    cur(r"README\.md")
    tui.keys("]")
    cur(r"gone\.txt")
    tui.keys("]")
    cur(r"notes_cjk\.txt")
    tui.keys("[")
    cur(r"gone\.txt")
    tui.keys("[")
    cur(r"README\.md")
    # (paging and hunk steps inside one file: local/navigation_hunks_and_paging_in_long_file)
    tui.quit()
