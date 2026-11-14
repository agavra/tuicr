#!/usr/bin/env bash
set -euo pipefail
. "$(dirname "$0")/../../lib/init.sh"
# Pull-request export before any submit: a draft written in the TUI is exported with the PR's URL and head SHA (so an agent
# can fetch the PR; the fixture's head is 222...2, shown short), the numbered comment anchored at its file:line, and no POST is made to the forge.
n=$(e2e_name); sc_fake_clipboard $n
sc_pr_open $n
sc_pr_comment $n 'export this draft'
sc_cmd $n clip
clip=$(sc_clipboard $n)
grep -qx 'URL: https://github.com/octo/demo/pull/1' <<<"$clip" || { echo "export lacks the PR URL:"; echo "$clip"; exit 1; }
grep -qx 'Head: 22222222' <<<"$clip" || { echo "export lacks the PR head sha:"; echo "$clip"; exit 1; }
grep -qx '## Local tuicr Comments' <<<"$clip" || { echo "export lacks the local comments heading:"; echo "$clip"; exit 1; }
grep -qx '1\. `src/greet\.rs:1` - export this draft' <<<"$clip" || { echo "export lacks the numbered, anchored draft:"; echo "$clip"; exit 1; }
# the unresolved remote thread is listed separately, after the local draft
grep -qx '## Existing GitHub Comments' <<<"$clip" || { echo "export lacks the remote-thread section:"; echo "$clip"; exit 1; }
[ -z "$(sc_posts $n)" ] || { echo "a POST happened during export"; sc_posts $n; exit 1; }
sc_quit $n
