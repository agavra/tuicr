#!/usr/bin/env bash
set -euo pipefail
. "$(dirname "$0")/../../lib/init.sh"
# Started outside any repository, tuicr exits 1 before the TUI with a "Not a repository" diagnostic (docs/CLI.md exit codes).
n=$(e2e_name); d=$(e2e_dir $n); mkdir -p $d/plain
cd $d/plain
sc_capture $n --no-update-check
[ "$SC_RC" = 1 ] || { echo "expected exit 1, got $SC_RC"; echo "$SC_OUT"; exit 1; }
grep -q 'Not a repository' <<<"$SC_OUT" || { echo "no repository diagnostic:"; echo "$SC_OUT"; exit 1; }
grep -q 'git, jujutsu, or mercurial repository' <<<"$SC_OUT" || { echo "no hint about supported VCSs:"; echo "$SC_OUT"; exit 1; }
# the same for a revision range: still the repository error, not a revision error
sc_capture $n --no-update-check -r HEAD~1..HEAD
[ "$SC_RC" = 1 ] && grep -q 'Not a repository' <<<"$SC_OUT" || { echo "-r outside a repo: rc=$SC_RC"; echo "$SC_OUT"; exit 1; }
