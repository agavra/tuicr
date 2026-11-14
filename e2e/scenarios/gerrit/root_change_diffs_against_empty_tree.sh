#!/usr/bin/env bash
set -euo pipefail
. "$(dirname "$0")/../../lib/init.sh"
# Covers #884: a parentless (root) Gerrit change diffs against the empty tree. Before, its base was an
# empty string, so the local `git diff <base>..<head>` failed and the change could not be opened.
n=$(e2e_name)
sc_gerrit_open $n root
e2e_wait_for $n 'a.txt' 20 || { cat "$(e2e_dir $n)/gerrit/gerrit.log"; exit 1; }
e2e_settle $n; e2e_keys $n '}'
e2e_wait_for $n 'line two' || { echo "root change diff not shown"; exit 1; }
sc_quit $n
