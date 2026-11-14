# e2e/lib/init.sh - source this from every scenario:  . "$(dirname "$0")/../../lib/init.sh"
#
# Responsibilities, one file each:
#   core.sh    run environment, isolation, composed cleanup, failure bookkeeping
#   tmux.sh    the terminal driver: start tuicr, send keys, read/poll the screen
#   assert.sh  assertions on screen, fake-forge call log, files
#   repos.sh   fixture repository builders (git, hg, jj)
#   flows.sh   shared UI flows (open a review, run a ':' command, move the cursor, save)
#   forge.sh   fake-forge fixtures (PR #1 via gh, Gerrit REST server) and payload readers
E2E_SCENARIO=${E2E_SCENARIO:-$0}
_e2e_lib="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
for _f in core tmux assert repos flows forge; do . "$_e2e_lib/$_f.sh"; done
unset _f
