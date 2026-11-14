#!/usr/bin/env bash
# A changed binary file must be labelled as binary, not "(no changes)".
set -euo pipefail
. "$(dirname "$0")/../../lib/init.sh"
n=$(e2e_name); sc_diff $n
e2e_expect $n '═══ data\.bin \[M\]'
e2e_expect $n '\(binary file\)' 
sc_quit $n
