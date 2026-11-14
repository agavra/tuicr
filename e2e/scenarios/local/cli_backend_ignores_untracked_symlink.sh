#!/usr/bin/env bash
set -euo pipefail
. "$(dirname "$0")/../../lib/init.sh"
# Covers #899: with `backend = "cli"` an untracked symlink must not be followed: content outside the repo
# (here a "secret" file) must not appear in the Unstaged view. Baseline reads through the link and shows it.
n=$(e2e_name); d=$(e2e_dir $n); mkdir -p $d; fx_git_repo $d/repo
printf 'TOP-SECRET-OUTSIDE-REPO\n' > $d/secret.txt
ln -s $d/secret.txt $d/repo/leak_link
mkdir -p $d/config/tuicr; printf 'backend = "cli"\n' > $d/config/tuicr/config.toml
rm -rf $d/{home,data,state}; E2E_KEEP=1 E2E_CWD=$d/repo e2e_start $n "$TUICR_BIN" --no-update-check
e2e_wait_for $n 'Unstaged changes'
e2e_keys $n Enter
e2e_wait_for $n 'untracked\.txt'
e2e_expect $n 'leak_link'
# the link is listed as an added file whose content is the link text, not the file behind it
e2e_expect $n 'leak_link \[A\]'
e2e_keys $n Escape; e2e_keys $n G; e2e_settle $n
e2e_reject $n 'TOP-SECRET' 0.3 || { echo "untracked symlink was followed: outside content shown"; exit 1; }
# scroll the whole diff to be sure
for i in 1 2 3 4 5 6; do e2e_keys $n C-d; done; e2e_settle $n
e2e_reject $n 'TOP-SECRET' 0.3 || { echo "untracked symlink was followed: outside content shown"; exit 1; }
sc_quit $n
