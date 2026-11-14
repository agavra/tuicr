#!/usr/bin/env bash
set -euo pipefail
. "$(dirname "$0")/../../lib/init.sh"
# The four built-in classifications (note, suggestion, issue, praise) opted into through `comment_types`: one comment of each
# is saved through the TUI. Each is stored under its own id, exported with its own `[TYPE]` tag at its own file:line, and
# the legend names every type used with its definition.
n=$(e2e_name); d=$(e2e_dir $n); mkdir -p $d/config/tuicr; fx_git_repo $d/repo
cat > $d/config/tuicr/config.toml <<'T'
comment_types = [
  { id = "note", definition = "observation" },
  { id = "suggestion", definition = "improvement" },
  { id = "issue", definition = "must fix" },
  { id = "praise", definition = "well done" },
]
T
out=$d/stdout.txt; rm -f $out
E2E_CWD=$d/repo e2e_start_keep $n sh -c "exec '$TUICR_BIN' --no-update-check --stdout -r HEAD~2..HEAD >'$out' 2>/dev/null"
e2e_wait_for $n 'Files · 0/8'; e2e_keys $n Escape
add() {  # <cursor regex> <tabs to press> <label shown> <text>
  sc_cursor_on $n "$1"
  e2e_keys $n c; e2e_wait_for $n 'Add \[NOTE\]'          # the first configured type is the default
  [ "$2" = 0 ] || { local i; for i in $(seq 1 "$2"); do e2e_keys $n Tab; done; }
  e2e_wait_for $n "Add \\[$3\\]"
  e2e_keys $n "$4"; e2e_keys $n Enter; e2e_wait_for $n "$4"
}
add '4 ▌ second line'     0 NOTE       'plain observation'
add '2 ▌ 中文内容'        1 SUGGESTION 'better approach'
add '1 ▌ pub fn a\(\) -> u32 \{ 2 \}' 2 ISSUE 'broken thing'
add '2 ▌     println!\("hello"\);' 3 PRAISE 'nicely done'
sc_cmd $n clip; e2e_wait_exit $n
for want in 'NOTE.*README\.md:4.*plain observation' 'SUGGESTION.*notes_cjk\.txt:2.*better approach' \
            'ISSUE.*src/lib\.rs:1.*broken thing' 'PRAISE.*src/main\.rs:2.*nicely done'; do
  grep -Eq "^[0-9]+\. \*\*\[$want" $out || { echo "export lacks /$want/:"; cat $out; exit 1; }
done
grep -q 'Comment types: NOTE (observation), SUGGESTION (improvement), ISSUE (must fix), PRAISE (well done)' $out \
  || { echo "legend lacks the four types:"; cat $out; exit 1; }
# stored under their ids (the session was autosaved when each comment was added)
j=$(sc_session_json $n | tr -d ' \n')
for t in note suggestion issue praise; do
  grep -Fq "\"comment_type\":\"$t\"" <<<"$j" || { echo "session lacks comment_type $t:"; echo "$j" | head -c 3000; exit 1; }
done
