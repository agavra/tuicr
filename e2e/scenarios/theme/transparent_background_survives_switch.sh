#!/usr/bin/env bash
set -euo pipefail
. "$(dirname "$0")/../../lib/init.sh"
# Covers #901: transparent_background (default on) must survive runtime theme changes. Baseline painted the opaque
# palette bg (light = 245;243;232) after `:theme light` and during picker previews.
n=$(e2e_name)
sc_diff $n
cnt() { e2e_pane_exists $n || return 1; _tmux capture-pane -e -p -t "$(_e2e_sess $n)" | { grep -ao '48;2;245;243;232' || true; } | wc -l | tr -d ' ' || true; }
echo "opaque light bg cells before: $(cnt)"
sc_cmd $n 'theme light'; e2e_settle $n 0.4
c=$(cnt); echo "opaque light panel_bg cells after :theme light: $c"
[ "$c" -eq 0 ] || { echo ":theme light painted an opaque panel background despite transparent_background"; exit 1; }
sc_cmd $n theme; e2e_wait_for $n 'Switch Theme'; e2e_keys $n j; e2e_settle $n; e2e_keys $n k; e2e_settle $n; e2e_keys $n j; e2e_settle $n
c=$(cnt); echo "after picker preview: $c"
[ "$c" -eq 0 ] || { echo "picker preview painted an opaque panel background"; exit 1; }
e2e_keys $n Escape; sc_quit $n
