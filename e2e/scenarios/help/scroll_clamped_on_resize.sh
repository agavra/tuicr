#!/usr/bin/env bash
set -euo pipefail
. "$(dirname "$0")/../../lib/init.sh"
# Covers #905: scroll the help to its end in a small terminal, then enlarge the terminal. The scroll offset
# must be clamped to the new (smaller) maximum; baseline keeps the old offset and the popup shows a blank tail.
n=$(e2e_name)
E2E_ROWS=20 sc_diff $n
e2e_keys $n '?'; e2e_wait_for $n 'HELP'
e2e_keys $n G; e2e_settle $n
e2e_resize $n 160 70; e2e_settle $n 0.4
e2e_screen $n > $E2E_OUT/$n.after.txt

# the popup must not end with a run of blank rows: count blank interior rows right above its bottom border
last=$(e2e_screen $n | grep -nE '└─{90,}┘ +│$' | head -1 | cut -d: -f1)
[ -n "$last" ] || { echo "popup bottom border not found after resize"; e2e_screen $n; exit 1; }
echo "last=$last"; blank=0; i=$((last-1))
while [ $i -gt 0 ]; do
  row=$(e2e_screen $n | sed -n ${i}p)
  # popup interior blank = only box-drawing chars/spaces between the popup's borders
  if grep -Eq '│ {90,}│' <<<"$row"; then blank=$((blank+1)); i=$((i-1)); else break; fi
done
echo "blank rows above popup bottom: $blank"
[ $blank -le 3 ] || { echo "help popup shows a blank tail after resize ($blank rows)"; exit 1; }
e2e_expect $n 'Commands|:q'
