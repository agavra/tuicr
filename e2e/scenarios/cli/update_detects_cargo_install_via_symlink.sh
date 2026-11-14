#!/usr/bin/env bash
set -euo pipefail
. "$(dirname "$0")/../../lib/init.sh"
# Covers #909: a Cargo-installed tuicr whose home is reached through a symlink ($HOME -> real dir; the running
# executable resolves to the real path) must be detected as a Cargo install and updated through `cargo install`. Baseline
# compared the canonical executable against the un-canonicalised ~/.cargo, saw a "direct" install and went to GitHub
# (blocked here by a dead proxy, so it fails with a network error and never calls cargo).
n=$(e2e_name); d=$E2E_ROOT/$n; rm -rf "$d"; mkdir -p "$d/real/.cargo/bin" "$d/fake"
ln -s "$d/real" "$d/linkhome"
cp "$TUICR_BIN" "$d/real/.cargo/bin/tuicr"
printf '#!/bin/sh\necho "cargo $*" >> %s/cargo.log\nexit 0\n' "$d" > "$d/fake/cargo"; chmod +x "$d/fake/cargo"
: > "$d/cargo.log"
set +e
out=$(env -i HOME="$d/linkhome" PATH="$d/fake:/usr/bin:/bin" https_proxy=http://127.0.0.1:9 HTTPS_PROXY=http://127.0.0.1:9 \
  "$d/linkhome/.cargo/bin/tuicr" update 2>&1); rc=$?
set -e
echo "rc=$rc"; echo "$out"; echo "cargo log:"; cat "$d/cargo.log"
grep -q 'cargo install tuicr' "$d/cargo.log" || { echo "Cargo install behind a symlinked home was not detected (cargo never ran)"; exit 1; }
