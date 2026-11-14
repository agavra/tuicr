#!/usr/bin/env bash
set -euo pipefail
. "$(dirname "$0")/../../lib/init.sh"
# with files in several `hg status` classes (modified z, added a, removed m),
# each file's hunk must be shown under its own file. Needs real Mercurial (E2E_HG_DIR/hg), not
# Sapling: SKIP without it, FAIL when E2E_REQUIRE_HG=1.
e2e_use_hg
n=$(e2e_name); d=$(e2e_dir $n); mkdir -p $d; fx_hg_repo $d/repo
E2E_CWD=$d/repo e2e_start $n "$TUICR_BIN" --no-update-check
e2e_wait_for $n 'Unstaged changes'
e2e_keys $n Enter
e2e_wait_for $n 'z\.txt'
e2e_wait_for $n '(═══|---).*a\.txt'
# each file section header must be followed by its own content (a: added, m: removed, z: z_old->z_new)
scr=$(e2e_screen $n)
echo "$scr" | awk '/a\.txt \[/{f="a"} /m\.txt \[/{f="m"} /z\.txt \[/{f="z"} /a_added/{print "a_added in " f} /m_old/{print "m_old in " f} /z_new/{print "z_new in " f} /z_old/{print "z_old in " f}' | tee $d/pairing.txt
grep -qx 'a_added in a' $d/pairing.txt && grep -qx 'm_old in m' $d/pairing.txt && grep -qx 'z_new in z' $d/pairing.txt \
  || { echo "hunks paired with the wrong files"; echo "$scr"; exit 1; }
sc_quit $n
