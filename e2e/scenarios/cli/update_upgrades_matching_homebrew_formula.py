#!/usr/bin/env python3
# Covers the Homebrew half of the update install-detection fix: the README installs from homebrew-core
# (`brew install tuicr`), so `tuicr update` must upgrade the formula named by the keg's INSTALL_RECEIPT.json
# (`tuicr` for homebrew/core, `agavra/tap/tuicr` for the tap). Baseline always ran `brew upgrade agavra/tap/tuicr`,
# which fails for a core install. An unreadable receipt still upgrades `tuicr`, with a one-line warning that the
# tap could not be determined.
import shutil
import subprocess
from pathlib import Path

from e2e import TUICR_BIN, Scenario, repos, scenario


def run_update(d: Path, tap: str | None) -> tuple[str, str]:
    keg = d / (tap or "unreadable").replace("/", "-") / "Cellar/tuicr/1.0.0"
    (keg / "bin").mkdir(parents=True)
    shutil.copy(TUICR_BIN, keg / "bin/tuicr")
    if tap is None:
        (keg / "INSTALL_RECEIPT.json").mkdir()
    else:
        repos.write(keg / "INSTALL_RECEIPT.json", f'{{"source": {{"tap": "{tap}"}}}}\n')
    log = repos.write(d / "brew.log", "")
    p = subprocess.run(
        [str(keg / "bin/tuicr"), "update"],
        stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
        env={
            "HOME": str(d),
            "PATH": f"{d}/fake:/usr/bin:/bin",
            "https_proxy": "http://127.0.0.1:9",
            "HTTPS_PROXY": "http://127.0.0.1:9",
        },
    )
    print(f"{tap}: rc={p.returncode}")
    print(p.stdout.rstrip("\n"))
    return log.read_text(), p.stdout


@scenario
def main(s: Scenario) -> None:
    d = repos.fresh(s.dir)
    (d / "fake").mkdir()
    brew = d / "fake/brew"
    brew.write_text(f'#!/bin/sh\necho "brew $*" >> {d}/brew.log\nexit 0\n')
    brew.chmod(0o755)

    core, out = run_update(d, "homebrew/core")
    assert "warning" not in out, out
    print(f"brew log: {core}", end="")
    assert core == "brew upgrade tuicr\n", f"homebrew-core install upgraded the wrong formula: {core!r}"

    tap, _ = run_update(d, "agavra/tap")
    print(f"brew log: {tap}", end="")
    assert tap == "brew upgrade agavra/tap/tuicr\n", f"tap install upgraded the wrong formula: {tap!r}"

    unreadable, out = run_update(d, None)
    print(f"brew log: {unreadable}", end="")
    assert unreadable == "brew upgrade tuicr\n", f"unreadable receipt upgraded the wrong formula: {unreadable!r}"
    assert "could not determine the Homebrew tap" in out, f"missing tap warning: {out!r}"
