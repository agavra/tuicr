#!/usr/bin/env python3
# Runner self-test (expect FAIL, rc 124): a hung scenario is killed at E2E_SCENARIO_TIMEOUT together with its background children.
import subprocess

from e2e import ROOT, scenario


@scenario
def main(s):
    child = subprocess.Popen(["sleep", "300"])  # a background child: it must die with the scenario's process group
    root = str(ROOT)
    root = root[:-2] if root.endswith("/h") else root
    with open(f"{root}/hung_child.pid", "w") as f:
        f.write(str(child.pid) + "\n")
    subprocess.run(["sleep", "300"])
