#!/usr/bin/env bash
set -euo pipefail
. "$(dirname "$0")/../../lib/init.sh"
# Covers #891: (a) a discussion whose position.head_sha differs from the MR head is outdated: hidden by the
# default `:comments unresolved`, shown muted with an "outdated" badge under `:comments all`. (b) a general comment that
# got a reply (positionless discussion with individual_note=false) stays visible instead of being dropped.
# Baseline: outdated thread shown as active by default, replied general comment dropped.
n=$(e2e_name); d=$(e2e_dir $n); mkdir -p $d; cat > $d/disc.json <<'JSON'
[{"id":"stale","individual_note":false,"notes":[{"id":11,"body":"STALE-INLINE-NOTE","author":{"username":"carol"},"resolved":false,"system":false,
   "position":{"position_type":"text","head_sha":"1111111111111111111111111111111111111111","new_path":"src/greet.rs","new_line":1}}]},
 {"id":"fresh","individual_note":false,"notes":[{"id":12,"body":"FRESH-INLINE-NOTE","author":{"username":"dave"},"resolved":false,"system":false,
   "position":{"position_type":"text","head_sha":"2222222222222222222222222222222222222222","new_path":"src/greet.rs","new_line":2}}]},
 {"id":"gen","individual_note":false,"notes":[{"id":13,"body":"GENERAL-ROOT-NOTE","author":{"username":"erin"},"resolved":false,"system":false},
   {"id":14,"body":"GENERAL-REPLY-NOTE","author":{"username":"frank"},"resolved":false,"system":false}]}]
JSON
E2E_ENV="E2E_GL_DISCUSSIONS=$d/disc.json" sc_forge_open $n https://gitlab.com/octo/demo.git
# general comments render in the PR description pane (top); check before `}` scrolls them away
e2e_wait_for $n 'GENERAL-ROOT-NOTE' 10 || { echo "replied general comment dropped"; e2e_screen $n; exit 1; }
e2e_keys $n '}'
e2e_wait_for $n 'FRESH-INLINE-NOTE' 10 || { echo "current-head thread missing"; e2e_screen $n; exit 1; }
e2e_reject $n 'STALE-INLINE-NOTE' || { echo "outdated discussion shown as active by default"; e2e_screen $n; exit 1; }
sc_cmd $n 'comments all'
e2e_wait_for $n 'STALE-INLINE-NOTE' 5 || { echo "outdated thread absent under :comments all"; e2e_screen $n; exit 1; }
e2e_screen $n | grep -Eq '@carol outdated\]' || { echo "no outdated badge"; e2e_screen $n; exit 1; }
sc_quit $n
