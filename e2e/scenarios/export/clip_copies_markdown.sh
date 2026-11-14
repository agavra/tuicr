#!/usr/bin/env bash
set -euo pipefail
. "$(dirname "$0")/../../lib/init.sh"
# Plain `:clip` (no --stdout) copies the review markdown to the system clipboard, confirms it, and leaves the TUI running.
n=$(e2e_name); d=$(e2e_dir $n); mkdir -p $d; fx_git_repo $d/repo
sc_fake_clipboard $n
E2E_CWD=$d/repo e2e_start $n "$TUICR_BIN" --no-update-check -r HEAD~2..HEAD
e2e_wait_for $n 'Files · 0/8'; e2e_keys $n Escape
sc_cursor_on $n '4 ▌ second line'
e2e_keys $n c; e2e_wait_for $n COMMENT; e2e_keys $n 'clip me please'; e2e_keys $n Enter; e2e_wait_for $n 'clip me please'
sc_cmd $n clip
clip=$(sc_clipboard $n)
grep -q '^I reviewed your code' <<<"$clip" && grep -q '^## Local tuicr Comments' <<<"$clip" && grep -qx '1\. `README.md:4` - clip me please' <<<"$clip" \
  || { echo "clipboard lacks the structured export:"; echo "$clip"; exit 1; }
e2e_wait_for $n 'Review copied to clipboard'                        # the status line confirms the copy
e2e_alive $n || { echo "tuicr exited after :clip"; exit 1; }
e2e_expect $n 'clip me please'                        # still showing the review
sc_quit $n
