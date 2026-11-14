#!/usr/bin/env bash
set -euo pipefail
. "$(dirname "$0")/../../lib/init.sh"
# `--file F --line N` opens the unchanged file with the cursor on line N (centered); a comment made there is exported
# with its `F:N` anchor by `:clip` under `--stdout`.
n=$(e2e_name); d=$(e2e_dir $n); mkdir -p $d; fx_long_file_repo $d/repo
out=$d/stdout.txt; rm -f $out
E2E_CWD=$d/repo e2e_start $n sh -c "exec '$TUICR_BIN' --no-update-check --stdout --file long.txt --line 70 >'$out' 2>/dev/null"
e2e_wait_for $n '▶ +70 ▌ line 70'
scr=$(e2e_screen $n); grep -Eq '^ +49 ▌ line 49' <<<"$scr" && grep -Eq '^ +9[01] ▌ line 9[01]' <<<"$scr" \
  || { echo "line 70 is not centered in the viewport"; echo "$scr"; exit 1; }
e2e_keys $n c; e2e_wait_for $n COMMENT; e2e_keys $n 'annotated at seventy'; e2e_keys $n Enter; e2e_wait_for $n 'annotated at seventy'
sc_cmd $n clip; e2e_wait_exit $n
grep -Eq 'long\.txt:70.*annotated at seventy' $out || { echo "export lacks the anchored comment:"; cat $out; exit 1; }
