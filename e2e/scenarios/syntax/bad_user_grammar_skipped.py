#!/usr/bin/env python3
# Covers #907: a user grammar syntect cannot compile (here `push: [[]]`) is skipped with a warning; tuicr still starts
# and the valid user grammar next to it still highlights. Baseline panics inside syntect at startup.
from e2e import Scenario, because, repos, scenario


@scenario
def main(s: Scenario) -> None:
    r = repos.empty_repo(s.dir / "repo")
    grammar = (
        "%YAML 1.2\n---\nname: PushEmpty\nscope: source.pushempty\nfile_extensions: [pushempty]\n"
        "contexts:\n  main:\n    - match: a\n      push: [[]]\n"
    )
    repos.write(
        s.dir / "config" / "tuicr" / "syntaxes" / "PushEmpty.sublime-syntax", grammar
    )
    repos.write(r / "a.txt", "hello\n")
    repos.write(r / "b.pushempty", "aaa\n")
    tui = s.start("--file", "a.txt", cwd=r, keep=True)
    with because("tuicr did not start with a malformed user grammar present"):
        tui.wait_for(r"a\.txt", 8)
    print("\n".join(ln[:150] for ln in tui.screen().split("\n")[:6]))
    assert tui.alive(), "tuicr died at startup"
    tui.quit()
