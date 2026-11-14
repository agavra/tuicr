#!/usr/bin/env bash
set -euo pipefail
. "$(dirname "$0")/../../lib/init.sh"
# Covers #912: an external edit of the persisted session that clears a hunk's reviewed mark must be merged by the
# running TUI (three-way) instead of being overwritten by its next save. Baseline resurrected the hunk mark on `:w`.
n=$(e2e_name)
sc_diff $n
sc_cursor_on $n '4 ▌ second line'
e2e_keys $n R; e2e_settle $n
sc_save $n
marked=$(e2e_screen $n)
f=$(find "$(e2e_dir $n)/data" "$(e2e_dir $n)/home" -path '*/sessions/*.json' | head -1); echo "session: $f"
hunks() { python3 -c "import json,sys; d=json.load(open(sys.argv[1])); print(sum(len(v.get('reviewed_hunks') or []) for v in d['files'].values()))" "$f"; }
echo "hunk marks after :w: $(hunks)"
[ "$(hunks)" -ge 1 ] || { echo "R did not persist a hunk mark"; exit 1; }
python3 - "$f" <<'PY'
import json, sys
p = sys.argv[1]; d = json.load(open(p))
for v in d['files'].values(): v['reviewed_hunks'] = []
json.dump(d, open(p + '.tmp', 'w')); import os; os.replace(p + '.tmp', p)
PY
# barrier: the running TUI polls the session file (review_watch_interval_ms = 1000) and redraws the merged mark
e2e_wait_screen_change $n "$marked" 10 || { echo "TUI never merged the external edit"; exit 1; }
e2e_settle $n
sc_save $n
echo "hunk marks after external clear + :w: $(hunks)"
[ "$(hunks)" -eq 0 ] || { echo "TUI save resurrected the externally cleared hunk mark"; exit 1; }
sc_quit $n
