#!/usr/bin/env bash
set -euo pipefail
. "$(dirname "$0")/../../lib/init.sh"
# Covers #898: GitLab `:submit draft` of two inline comments where the 2nd POST fails. The 1st landed on the forge;
# the retry must post only the 2nd. Baseline leaves both as local drafts and re-posts the 1st (duplicate on the forge).
n=$(e2e_name); d=$(e2e_dir $n); mkdir -p $d
touch $d/failflag
E2E_ENV="E2E_GL_FAIL_FLAG=$d/failflag" sc_forge_open $n https://gitlab.com/octo/demo.git
e2e_keys $n '}'; sc_cursor_on $n '1[╭ ]▌ pub fn greet'
e2e_keys $n c; e2e_wait_for $n COMMENT; e2e_keys $n 'first-note'; e2e_keys $n Enter; e2e_wait_for $n 'first-note'
sc_cursor_on $n '2[╭ ]▌ +println'
e2e_keys $n c; e2e_wait_for $n COMMENT; e2e_keys $n 'second-note'; e2e_keys $n Enter; e2e_wait_for $n 'second-note'
sc_cmd $n 'submit draft'; e2e_wait_for $n 'Inline: 2'; e2e_keys $n y
# barrier: the 2nd POST failed on the forge AND the UI processed the failure (the 1st comment's receipt is recorded)
e2e_wait_forge_log '"note": "injected failure"' 15; e2e_wait_for $n 'Submit failed' 15; e2e_settle $n
posted() { python3 -c "import json,sys;print(len([x for x in json.load(open('$d/gl_drafts.json')) if 'first-note' in json.dumps(x)]))" 2>/dev/null || echo 0; }
[ "$(posted)" = 1 ] || { echo "setup: first comment should have landed once"; exit 1; }
rm -f $d/failflag
sc_cmd $n 'submit draft'; e2e_wait_for $n 'Push pending|Submit review to'
e2e_keys $n y
# barrier: the retry's POST landed on the forge
e2e_wait_grep $d/gl_drafts.json 'second-note' 15; e2e_wait_for $n 'Pushed pending GitLab review' 15
[ "$(posted)" = 1 ] || { echo "retry re-posted the comment that already landed (first-note x$(posted))"; exit 1; }
grep -q 'second-note' $d/gl_drafts.json || { echo "retry did not post the failed comment"; exit 1; }
sc_quit $n
