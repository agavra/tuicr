#!/usr/bin/env bash
set -euo pipefail
. "$(dirname "$0")/../../lib/init.sh"
# Covers #917: Gitea clamps `limit` (MaxResponseItems). With 45 PRs and a server cap of 20/page, the PR list's
# load-more must continue after the rows already shown. Baseline derived the page as already_loaded / page_size (20/30 + 1 = 1),
# refetched page 1 and showed rows 1..20 twice.
n=$(e2e_name); d=$(e2e_dir $n); mkdir -p $d; fx_pr_repo $d/repo; rm -f $d/gt_reviews.json
git -C $d/repo remote set-url origin https://gitea.example.com/octo/demo.git
E2E_ENV="E2E_GT_ROWS=45 E2E_GT_CAP=20" E2E_CWD=$d/repo e2e_start $n "$TUICR_BIN" --no-update-check
e2e_wait_for $n 'Pull Requests'; e2e_settle $n; e2e_keys $n Tab; e2e_wait_for $n 'GT row 1 '
e2e_keys $n j j j j j j j j j j j j j j j j j j j; e2e_settle $n   # most of the way down in one burst
i=0; until scr=$(e2e_screen $n) && grep -E '▸' <<<"$scr" | grep -q 'load more'; do e2e_keys $n j; e2e_settle $n 0.1; i=$((i+1)); [ $i -lt 80 ] || { echo "never reached the load-more row"; exit 1; }; done
echo "load-more row reached after $i moves"
e2e_keys $n Enter
# barrier: the next page's first row (21) is listed right below the load-more row
e2e_wait_for $n 'GT row 21 '
for j in $(seq 60); do e2e_keys $n j; done; e2e_settle $n
# the list is scrolled; collect every row seen by paging up/down is overkill: duplicates show as row numbers repeating
scr=$(e2e_screen $n)
nums=$(grep -o 'GT row [0-9]*' <<<"$scr" | sort | uniq -d | head -3)
[ -z "$nums" ] || { echo "duplicate rows after load-more: $nums"; exit 1; }
e2e_expect $n 'GT row (2[1-9]|3[0-9]|4[0-5])' || { echo "load-more did not reach rows past the first page"; exit 1; }
sc_quit $n
