#!/usr/bin/env bash
set -euo pipefail
. "$(dirname "$0")/../../lib/init.sh"
# Covers #902: an old-side range over a context line + a deleted line (README L1-L2) maps to an inline GitHub
# comment (start_line 1, line 2, side LEFT). Baseline calls it "range spans both diff sides" and sends it to the resolver.
n=$(e2e_name); sc_pr_open $n
e2e_keys $n '}'; sc_cursor_on $n 'old line'
e2e_keys $n v; e2e_keys $n k; e2e_settle $n; e2e_keys $n c; e2e_wait_for $n COMMENT 3
e2e_keys $n 'range-old'; e2e_keys $n Enter; e2e_wait_for $n 'L1-L2'
sc_cmd $n 'submit comment'; e2e_wait_for $n 'Submit review to GitHub|cannot be posted inline'
e2e_reject $n 'cannot be posted inline' || { echo "old-side range over context+deletion went to the resolver"; exit 1; }
e2e_wait_for $n 'Submit review to GitHub'; e2e_keys $n y
e2e_wait_for $n 'Submitted GitHub review' 15
sc_posts $n | python3 -c '
import json,sys
p=json.loads(sys.stdin.readline())["payload"]
c=p["comments"]; print(c)
assert len(c)==1 and c[0]["side"]=="LEFT" and c[0]["start_line"]==1 and c[0]["line"]==2, c'
sc_quit $n
