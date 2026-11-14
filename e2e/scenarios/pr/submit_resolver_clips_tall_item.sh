#!/usr/bin/env bash
set -euo pipefail
. "$(dirname "$0")/../../lib/init.sh"
# Covers #911: a resolver item taller than the list (500-char path, 16-row tty) must be clipped, not dropped.
# Baseline's wrapped Paragraph overran the popup and pushed the footer line (`Esc: cancel`) out; the fix clips the item.
export E2E_ROWS=16
n=$(e2e_name); sc_pr_open $n longpath
e2e_keys $n '}'; e2e_settle $n; e2e_keys $n '}'; e2e_settle $n
e2e_keys $n C; e2e_wait_for $n COMMENT; e2e_keys $n -l "long-note"; e2e_keys $n Enter; e2e_settle $n
sc_cmd $n 'submit comment'; e2e_wait_for $n 'cannot be posted inline'; e2e_settle $n
e2e_screen $n | cut -c34-150
e2e_expect $n '> \[x\] Move to summary' 2 || { echo "tall resolver item was dropped instead of clipped"; exit 1; }
e2e_expect $n 'Esc: cancel' 2 || { echo "resolver footer missing"; exit 1; }
e2e_keys $n Escape; sc_quit $n
