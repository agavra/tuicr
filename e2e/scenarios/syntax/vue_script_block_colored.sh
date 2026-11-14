#!/usr/bin/env bash
set -euo pipefail
. "$(dirname "$0")/../../lib/init.sh"
# Covers #907: `--file App.vue` colors the <script> block (container grammar, whole-file highlight).
# Baseline's FileBackend skipped enhance_with_full_file_highlight, so the Vue file stayed plain (a single foreground colour).
n=$(e2e_name); d=$(e2e_dir $n); mkdir -p $d/repo; r=$d/repo; rm -rf $r; mkdir -p $r; git init -q -b main $r
printf '<template>\n  <div class="a">{{ msg }}</div>\n</template>\n<script>\nconst answer = 42;\nexport default { name: "x" }\n</script>\n' > $r/App.vue
E2E_CWD=$r e2e_start $n "$TUICR_BIN" --no-update-check --file App.vue
e2e_wait_for $n 'const answer'
# colouring is applied at render time: poll until the script line carries token colours (or give up)
colors=0
for _ in $(seq 100); do
  line=$(_tmux capture-pane -e -p -J -t "$(_e2e_sess $n)" | grep -a 'const' | head -1) || { echo 'no pane'; exit 1; }
  colors=$(grep -ao '38;2;[0-9;]*' <<<"$line" | sort -u | wc -l)
  [ "$colors" -ge 7 ] && break; sleep 0.1
done
echo "distinct fg colours on the script line: $colors"
# gutter (3 colours) + plain text colour = 4 on an uncoloured line; coloured tokens add several more
[ "$colors" -ge 7 ] || { echo "Vue <script> line is not syntax coloured under --file"; exit 1; }
sc_quit $n
