#!/usr/bin/env bash
set -euo pipefail
. "$(dirname "$0")/../../lib/init.sh"
# Covers #897: an Azure DevOps PR URL with a query (`?_a=files`) is recognised as an Azure target. Without PAT / az
# the open stops at the Azure auth error (no network needed). Baseline fell through to the Gerrit parser:
# "Malformed Gerrit change target".
n=$(e2e_name); d=$(e2e_dir $n); mkdir -p $d/home; rm -rf $d/repo; mkdir -p $d/repo; git init -q $d/repo
cd $d/repo
# Deterministic: only git on PATH, so `az` is absent on every machine (CI images ship the real Azure CLI).
E2E_RUN_PATH=$(e2e_minimal_path $d/minbin git)
for u in 'https://dev.azure.com/myorg/myproject/_git/myrepo/pullrequest/42' 'https://dev.azure.com/myorg/myproject/_git/myrepo/pullrequest/42?_a=files#discussion'; do
  sc_capture $n --no-update-check pr "$u"
  echo "$u -> $SC_OUT"
  [ "$SC_RC" -ne 0 ] || { echo "tuicr pr unexpectedly succeeded"; exit 1; }
  grep -q 'Azure DevOps integration needs' <<<"$SC_OUT" || { echo "Azure PR URL not recognised: $u"; exit 1; }
done
