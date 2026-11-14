#!/usr/bin/env bash
set -euo pipefail
. "$(dirname "$0")/../../lib/init.sh"
# Remote review threads: unresolved shown by default, :comments all adds resolved, hide removes, navigator lists them.
n=$(e2e_name); sc_pr_open $n
e2e_expect $n 'remote open thread: please reword'; e2e_expect $n '\[github @bob\]'
e2e_reject $n 'remote resolved thread' 1
e2e_expect $n 'Comments · 1'
sc_cmd $n 'comments all'
e2e_wait_for $n 'remote resolved thread: fine now'; e2e_expect $n 'Comments · 2'
sc_cmd $n 'comments hide'
e2e_wait_gone $n 'remote open thread'; e2e_wait_gone $n 'remote resolved thread'
sc_cmd $n 'comments unresolved'
e2e_wait_for $n 'remote open thread'; e2e_wait_gone $n 'remote resolved thread'
# remote threads are read-only: editing the thread at the cursor is refused
e2e_keys $n m; e2e_settle $n; e2e_keys $n i
e2e_wait_for $n 'read only|read-only'
sc_quit $n
