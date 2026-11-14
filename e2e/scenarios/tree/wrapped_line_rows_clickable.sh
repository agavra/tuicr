#!/usr/bin/env bash
set -euo pipefail
. "$(dirname "$0")/../../lib/init.sh"
# with wrap on, a wrapped line cut off by the viewport bottom is still drawn, so its rows must be clickable.
# Baseline's row->line map stopped before the partly visible line: clicking its visible rows did nothing.
n=$(e2e_name); d=$(e2e_dir $n); rm -rf "$d"; mkdir -p "$d/repo"; r="$d/repo"
git init -q -b main "$r"; echo base > "$r/base.txt"; _fx_git "$r" add -A; _fx_git "$r" commit -qm base
python3 - "$r/big.txt" <<'PY'
import sys
L = ["short line %02d" % i for i in range(1, 37)]
open(sys.argv[1], "w").write("\n".join(L) + "\n" + " ".join("word%03d" % i for i in range(200)) + "\nlast\n")
PY
_fx_git "$r" add big.txt
E2E_CWD="$r" e2e_start $n "$TUICR_BIN" --no-update-check -w
e2e_wait_for $n 'big.txt \[A\]'; e2e_keys $n Escape
e2e_keys $n ":set wrap" Enter; e2e_wait_for $n 'word014'
e2e_screen $n | grep -E '▶' | grep -q 'big.txt \[A\]' || { echo "precondition: cursor not on the header"; e2e_screen $n; exit 1; }
# click the last visible diff row (bottom of the cut-off wrapped line 37)
e2e_keys $n -l "$(printf '\033[<0;60;43M\033[<0;60;43m')"; e2e_settle $n
e2e_expect $n '▶ +37 +▌ word000' || { echo "click on the partly visible wrapped line did not select it"; exit 1; }
sc_quit $n
