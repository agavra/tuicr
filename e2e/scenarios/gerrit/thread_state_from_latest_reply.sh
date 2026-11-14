#!/usr/bin/env bash
set -euo pipefail
. "$(dirname "$0")/../../lib/init.sh"
# Covers #884: a Gerrit thread's resolution comes from its LATEST reply. Thread 1 (open root, reply
# resolves it) must be hidden by the default unresolved-only filter; thread 2 (resolved root, reply reopens it)
# must show. Before, the root alone decided: the two were swapped.
n=$(e2e_name)
sc_gerrit_open $n threads
e2e_wait_for $n 'a.txt' 20 || { cat "$(e2e_dir $n)/gerrit/gerrit.log"; exit 1; }
e2e_settle $n; e2e_keys $n '}'
e2e_wait_for $n 'a.txt:2' || { echo "reopened thread (line 2) not shown"; exit 1; }
e2e_settle $n
scr=$(e2e_screen $n)
! grep -q 'a.txt:1' <<<"$scr" || { echo "resolved-by-reply thread (line 1) is shown"; echo "$scr"; exit 1; }
sc_quit $n
