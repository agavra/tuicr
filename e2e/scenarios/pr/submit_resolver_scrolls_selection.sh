#!/usr/bin/env bash
set -euo pipefail
. "$(dirname "$0")/../../lib/init.sh"
# Covers #911: the submit resolver lists more unmappable comments than fit; the selected row and the footer
# ("Esc: cancel") must stay visible while moving the cursor to the last row. Baseline rendered a plain Paragraph that clipped
# the tail (selection vanished, footer gone). 9 file-level comments on binary files (+1 on the commit message), 26-row tty.
export E2E_ROWS=26
n=$(e2e_name); sc_pr_open $n many_binary_files
for i in 1 2 3 4 5 6 7 8 9; do
  e2e_keys $n '}'; e2e_settle $n 0.2; e2e_keys $n C; e2e_wait_for $n COMMENT; e2e_keys $n -l "note-$i"; e2e_keys $n Enter; e2e_settle $n 0.2
done
sc_cmd $n 'submit comment'; e2e_wait_for $n 'cannot be posted inline'; e2e_settle $n
e2e_keys $n j j j j j j j j j; e2e_settle $n
e2e_screen $n | sed -n '6,24p' | cut -c34-150
e2e_expect $n '> \[x\] Move to summary  Commit Message' 2 || { echo "selected last resolver row is not visible"; exit 1; }
e2e_expect $n 'Esc: cancel' 2 || { echo "resolver footer scrolled away"; exit 1; }
e2e_keys $n Escape; sc_quit $n
