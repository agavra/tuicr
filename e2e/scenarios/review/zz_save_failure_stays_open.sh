#!/usr/bin/env bash
set -euo pipefail
. "$(dirname "$0")/../../lib/init.sh"
# `ZZ` reports a failed save and stays open, like `:wq`, instead of quitting and losing the unsaved mark. Header: panel G6-2.
[ "$(id -u)" != 0 ] || { echo "SKIP: root ignores directory permissions"; exit 0; }
n=$(e2e_name); sc_diff $n
d=$(e2e_dir $n)
e2e_keys $n r; e2e_wait_for $n 'Files · 1/8'
find $d/data $d/home -type d \( -name reviews -o -name sessions \) -exec chmod a-w {} +
trap 'chmod -R u+w "$d" 2>/dev/null || true' EXIT
e2e_keys $n ZZ
e2e_wait_for $n 'Save failed'
e2e_keys $n Escape ":q!" Enter; e2e_wait_exit $n
