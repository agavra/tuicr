#!/usr/bin/env python3
# after hiding the file list (<leader>e) its old rectangle must stop being a click target.
# Baseline kept the stale hit area, so clicking where the tree used to be selected a file instead of landing in the diff.
from e2e import scenario


@scenario
def main(s):
    tui = s.diff()
    tui.type(";e").wait_gone("Files · ")
    tui.click(5, 9).settle()  # SGR mouse press+release at (col 5, row 9) = where `gone.txt` sat in the tree
    tui.expect("┌ README.md ").reject("┌ gone.txt ")
    # the click lands in the diff: cursor row moved onto `1   # demo`
    tui.check("▶ +1 +# demo", "click did not move the diff cursor")
    tui.quit()
