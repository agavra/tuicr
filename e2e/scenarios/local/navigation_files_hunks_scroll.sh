#!/usr/bin/env bash
# Local review: file tree, scroll, hunk nav, file nav. Uses a -r range for multi-file/hunk content.
set -euo pipefail
. "$(dirname "$0")/../../lib/init.sh"
n=$(e2e_name); sc_diff $n
cur() { e2e_wait_for $n "┌ $1 ─+ "; }   # diff pane title names the file under the cursor
e2e_expect $n '▢ M README\.md'; e2e_expect $n '▢ D gone\.txt'; e2e_expect $n '▼ src/'; e2e_expect $n 'lib\.rs'
cur 'README\.md'
e2e_keys $n '}'; cur 'data\.bin'
e2e_keys $n '}' '}'; cur 'new_name\.txt'
e2e_keys $n '{'; cur 'gone\.txt'
e2e_keys $n G; cur 'src/main\.rs'
e2e_keys $n g; cur 'README\.md'
# j moves the cursor row; k moves it back
e2e_keys $n j j j; e2e_expect $n '▶ +3 +first line'
e2e_keys $n k; e2e_expect $n '▶ +2 +│'
# hunk nav inside src/main.rs (single hunk) and across files
e2e_keys $n G; e2e_keys $n '['; e2e_expect $n '▶.*(@@ -1,3 \+1,3 @@|═══ src/main)'
# g lands on README's hunk header; ] skips the hunk-less data.bin to gone.txt, then notes_cjk; [ steps back
e2e_keys $n g; cur 'README\.md'; e2e_keys $n ']'; cur 'gone\.txt'
e2e_keys $n ']'; cur 'notes_cjk\.txt'
e2e_keys $n '['; cur 'gone\.txt'
e2e_keys $n '['; cur 'README\.md'
# (paging and hunk steps inside one file: local/navigation_hunks_and_paging_in_long_file)
sc_quit $n
