#!/usr/bin/env bash
set -euo pipefail
. "$(dirname "$0")/../../lib/init.sh"
# Covers #907: a user grammar syntect cannot compile (here `push: [[]]`) is skipped with a warning; tuicr still starts
# and the valid user grammar next to it still highlights. Baseline panics inside syntect at startup.
n=$(e2e_name); d=$(e2e_dir $n); mkdir -p $d; r=$d/repo; rm -rf $r; mkdir -p $r; git init -q -b main $r
sy=$d/config/tuicr/syntaxes; mkdir -p $sy
printf '%%YAML 1.2\n---\nname: PushEmpty\nscope: source.pushempty\nfile_extensions: [pushempty]\ncontexts:\n  main:\n    - match: a\n      push: [[]]\n' > $sy/PushEmpty.sublime-syntax
printf 'hello\n' > $r/a.txt; printf 'aaa\n' > $r/b.pushempty
E2E_KEEP=1 E2E_CWD=$r e2e_start $n "$TUICR_BIN" --no-update-check --file a.txt
e2e_wait_for $n 'a\.txt' 8 || { echo "tuicr did not start with a malformed user grammar present"; e2e_screen $n; exit 1; }
e2e_screen $n | sed -n 1,6p | cut -c1-150
e2e_alive $n || { echo "tuicr died at startup"; exit 1; }
sc_quit $n
