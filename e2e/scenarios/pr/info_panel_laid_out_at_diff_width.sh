#!/usr/bin/env bash
set -euo pipefail
. "$(dirname "$0")/../../lib/init.sh"
# Covers #904: the PR description panel must be laid out at the real diff width on the first frame. Baseline wrapped
# the body at a stale (0 / previous) width, so a one-row description showed split over several rows until a later redraw.
n=$(e2e_name)
mkdir -p $(e2e_dir $n)/config/tuicr
E2E_VARIANT=long_pr_body sc_pr_open $n long_pr_body
e2e_settle $n
# narrow terminal wraps the body; widening again must relayout at the new width in the very frame drawn after the resize
e2e_resize $n 90 45; e2e_settle $n 0.4
e2e_screen $n | sed -n '3,7p' | cut -c1-100
e2e_resize $n 160 45; e2e_settle $n 0.4
e2e_screen $n | sed -n '1,12p' | cut -c32-150
e2e_expect $n 'Ship-it-line: .*wide OK' 2 || { echo "description body is not on one row at the real width"; exit 1; }
sc_quit $n
