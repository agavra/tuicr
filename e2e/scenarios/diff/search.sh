#!/usr/bin/env bash
set -euo pipefail
. "$(dirname "$0")/../../lib/init.sh"
# Diff search: / regex, match counter, n / N, Esc clears highlight, invalid regex error.
n=$(e2e_name); sc_diff $n
e2e_keys $n '/'; e2e_keys $n 'println|second'; e2e_keys $n Enter
e2e_wait_for $n '\[1/[0-9]+\]'; e2e_expect $n '\[1/3\]'
e2e_keys $n n; e2e_wait_for $n '\[2/3\]'
e2e_keys $n n n; e2e_wait_for $n '\[1/3\]'      # wraps
e2e_keys $n N; e2e_wait_for $n '\[3/3\]'
e2e_keys $n Escape; e2e_wait_gone $n '\[[0-9]/3\]'
e2e_keys $n n; e2e_wait_for $n '\[[0-9]/3\]'    # pattern kept
# invalid regex reports an error and keeps the previous search
e2e_keys $n '/'; e2e_keys $n '(unclosed'; e2e_keys $n Enter
e2e_wait_for $n 'Invalid regex'
e2e_keys $n n; e2e_wait_for $n '\[[0-9]/3\]'     # the previous pattern is still active
sc_quit $n
