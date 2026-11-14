#!/usr/bin/env bash
set -euo pipefail
. "$(dirname "$0")/../../lib/init.sh"
# Covers #912: a reviewed mark made on the whole range must survive narrowing to one commit and widening again.
# Baseline re-registered the file with the subset's content hash, which invalidated the mark (Files x/N fell back to 0).
n=$(e2e_name); d=$(e2e_dir $n); mkdir -p $d; r=$d/repo; rm -rf $r; mkdir -p $r; git init -q -b main $r
printf 'one\n' > $r/a.txt; printf 'x\n' > $r/b.txt; _fx_git $r add -A; _fx_git $r commit -qm c1
printf 'one\ntwo\n' > $r/a.txt; printf 'x2\n' > $r/b.txt; _fx_git $r commit -qam c2
printf 'one\ntwo\nthree\n' > $r/a.txt; _fx_git $r commit -qam c3
E2E_CWD=$r e2e_start $n "$TUICR_BIN" --no-update-check -r HEAD~2..HEAD
e2e_wait_for $n 'Files · 0/2'; e2e_keys $n Escape
sc_cursor_on $n 'three'; e2e_keys $n '{'; e2e_settle $n
e2e_keys $n r; e2e_wait_for $n 'Files · 1/2'
e2e_keys $n ')'; e2e_wait_for $n 'Commit Message \('; e2e_settle $n
e2e_expect $n 'Files · 1/' || { echo "reviewed mark lost after narrowing to one commit"; exit 1; }
e2e_keys $n ')'; e2e_wait_gone $n 'Commit Message \('; e2e_settle $n
e2e_expect $n 'Files · 1/2' || { echo "reviewed mark lost after widening back"; exit 1; }
sc_quit $n
