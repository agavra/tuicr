#!/usr/bin/env python3
# Covers #909: a Cargo-installed tuicr whose home is reached through a symlink ($HOME -> real dir; the running
# executable resolves to the real path) must be detected as a Cargo install and updated through `cargo install`.
# Baseline
# compared the canonical executable against the un-canonicalised ~/.cargo, saw a "direct" install and went to GitHub
# (blocked here by a dead proxy, so it fails with a network error and never calls cargo).
import os
import shutil
import subprocess

from e2e import TUICR_BIN, Scenario, grep, repos, scenario


@scenario
def main(s: Scenario) -> None:
    d = repos.fresh(s.dir)
    (d / "real/.cargo/bin").mkdir(parents=True)
    (d / "fake").mkdir()
    os.symlink(d / "real", d / "linkhome")
    shutil.copy(TUICR_BIN, d / "real/.cargo/bin/tuicr")
    cargo = d / "fake/cargo"
    cargo.write_text(f'#!/bin/sh\necho "cargo $*" >> {d}/cargo.log\nexit 0\n')
    cargo.chmod(0o755)
    log = repos.write(d / "cargo.log", "")
    p = subprocess.run(
        [str(d / "linkhome/.cargo/bin/tuicr"), "update"],
        stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
        env={
            "HOME": str(d / "linkhome"),
            "PATH": f"{d}/fake:/usr/bin:/bin",
            "https_proxy": "http://127.0.0.1:9",
            "HTTPS_PROXY": "http://127.0.0.1:9",
        },
    )
    print(f"rc={p.returncode}")
    print(p.stdout.rstrip("\n"))
    print("cargo log:")
    print(log.read_text(), end="")
    assert grep("cargo install tuicr", log.read_text()), (
        "Cargo install behind a symlinked home was not detected (cargo never ran)"
    )
