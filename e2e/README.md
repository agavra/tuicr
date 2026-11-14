# tuicr end-to-end tests

These scenarios drive the **real tuicr binary** inside a private tmux server and assert on what is on the
screen, on files tuicr writes, and on the calls it makes to **fake** forge CLIs (`gh`, `glab`, `tea`, `bkt`)
and a fake Gerrit REST server. No real forge, network or credentials are involved. They cover what unit tests
cannot see: long-lived UI state, mouse/paste/resize handling, terminal rendering, CLI wiring, persisted sessions
and the submit → refetch lifecycle.

```sh
cargo build --release
TUICR_BIN=target/release/tuicr e2e/run.sh                 # everything (~5 min)
TUICR_BIN=target/release/tuicr e2e/run.sh pr/submit tree/ # filters: substrings of <area>/<behaviour>
e2e/run.sh --list                                         # names only
e2e/selftest/run.sh                                       # tests of the harness itself (no tuicr needed)
```

Exit status is 0 only if at least one scenario ran and none failed. Results per scenario are `PASS`, `FAIL`
(assertion, crash or timeout), `SKIP` (scenario exit 77, e.g. no Mercurial) and `QPASS`/`QFAIL` for
quarantined scenarios (see below). Logs, final screenshots (`*.screen.txt`), fake-forge call logs
(`*.forge.log`) and `results.tsv` (name, result, seconds) land in `$E2E_OUT` (default `$E2E_ROOT/out`).

## Requirements

| Tool | Version | Notes |
|---|---|---|
| tmux | 3.4+ (developed on 3.8) | provides the pty; no display or attached terminal needed |
| bash | 3.2+ (macOS) / 5 (Linux) | scenarios and library |
| python3 | 3.9+ | fakes, a few helpers, the pty test of the installer |
| git | any recent | fixtures; sparse/CLI backend scenarios use the git CLI |
| Mercurial | real `hg` (not Sapling) | `local/hg_hunks_under_own_files` only; SKIPs without it, FAILs with `E2E_REQUIRE_HG=1` |
| Jujutsu | real `jj` (tested with 0.47) | `local/jj_*`; SKIP without it, FAIL with `E2E_REQUIRE_JJ=1`; `E2E_JJ_DIR` names the directory holding it |

Locale: tuicr and tmux run in UTF-8 (`en_US.UTF-8` if installed, otherwise `C.UTF-8`; override with
`E2E_LOCALE`). The terminal is 160x45 (`E2E_COLS`/`E2E_ROWS`). Linux and macOS are supported.

Environment knobs: `TUICR_BIN` (required), `E2E_ROOT` (scratch, default fresh `mktemp -d`), `E2E_OUT`,
`E2E_SOCKET` (tmux socket, default unique per run), `E2E_SCENARIO_TIMEOUT` (seconds per scenario, default 120),
`E2E_TIMEOUT` (default wait, 10 s), `E2E_HG_DIR` (directory holding a real `hg`), `E2E_REQUIRE_HG=1`,
`E2E_REQUIRE_JJ=1`, `E2E_JJ_DIR`, `E2E_REQUIRE_SKIPS_ZERO=1` (CI: a skip is a failure), `E2E_KEEP_ROOT=1`, `E2E_INSTALL_SH` / `E2E_INSTALL_REV`
(the installer script for `cli/install_script_without_tty`; default `upstream/site`, SKIP when unavailable).

## Layout

```
e2e/
  run.sh              runner: select, timeout, classify, report
  quarantine.tsv      tracked known failures: <area>/<behaviour> <TAB> reason (run, reported, not gating)
  lib/                the shell library, one file per responsibility
    init.sh             what a scenario sources
    core.sh             environment, git/hg/locale isolation, composed cleanup (e2e_on_exit), e2e_skip
    tmux.sh             terminal driver: e2e_start/keys/screen, wait_for/wait_gone/wait_exit/settle
    assert.sh           e2e_expect/reject, file and fake-forge-log waits and assertions
    repos.sh            fixture repositories (fx_git_repo, fx_pr_repo, fx_wide_repo, fx_long_file_repo, fx_jj_repo, e2e_use_hg/jj, ...)
    flows.sh            shared UI flows (sc_open, sc_diff, sc_cmd, sc_cursor_on, sc_save, sc_capture, sc_session_has,
                        sc_fake_clipboard/sc_clipboard: recording pbcopy/xclip/wl-copy so no scenario touches the real clipboard)
    forge.sh            forge fixtures (sc_pr_open, sc_forge_open, sc_gerrit_open, sc_posts, sc_forge_posts <tool>, ...)
    run_scenario.py     runs one scenario in its own process group, kills the tree on timeout
  fakeforge/          ONE fake-forge core (core.py) + thin adapters gh.py glab.py tea.py bkt.py, gerrit.py (REST server)
  bin/                gh glab tea bkt: wrappers that put fakeforge first on the child's PATH
  data/               fixture data per forge (github/, gitlab/, gitea/, bitbucket/, gerrit/), variant overlays
  scenarios/<area>/<behaviour>.sh
  selftest/           tests of the runner (false-green paths, timeout kill, skip, zero selection)
```

Areas: `local` (git/hg/commit selection), `tree`, `diff`, `comments`, `review` (marks, persistence), `theme`,
`input` (prompts, mouse, paste, selection), `help`, `ui`, `export`, `syntax`, `cli`, `pr` (GitHub PR flows), and one
per other forge: `gitlab`, `gitea`, `bitbucket`, `gerrit`, `azure`.

## Writing a scenario

A scenario is a short bash script named after the **behaviour it checks** (`pr/review_threads_reply_resolves.sh`,
not after a PR or a unit). Put the PR/issue it covers in the header comment (`# Covers #901: ...`).

```bash
#!/usr/bin/env bash
set -euo pipefail
. "$(dirname "$0")/../../lib/init.sh"
# Covers #123: filtering by `^src/` narrows the tree and the diff.
n=$(e2e_name); sc_diff $n                          # fixture repo, range of 2 commits, focus on the diff
e2e_keys $n ';h'; e2e_settle $n                    # focus the file tree
e2e_keys $n i '^src/' Enter
e2e_wait_for $n 'Files · 0/2'; e2e_wait_gone $n 'README\.md'
sc_quit $n
```

Rules that keep scenarios reliable:

* **Wait for the effect, never `sleep`.** After keys that start something asynchronous wait for the text it
  produces (`e2e_wait_for`/`e2e_wait_gone`), a file (`e2e_wait_file`, `e2e_wait_grep`), a fake-forge call
  (`e2e_wait_forge_log`, `e2e_wait_forge_count`) or process exit (`e2e_wait_exit [timeout] [status]`). For "the UI
  finished redrawing" with nothing specific to look for use `e2e_settle` (screen unchanged for 0.25 s).
  `sc_save` runs `:w` and waits for `Saved to`.
* **Wait first, then assert absence.** `e2e_reject` only proves something is absent *now*; put a positive
  barrier before it, otherwise it passes before the app has reacted.
* **A missing pane is a failure.** All screen helpers fail when the tmux pane is gone; capture into a variable
  (`scr=$(e2e_screen $n)`) before piping into `grep` when you assert absence, so a capture error is not an empty match.
* **Fakes fail loudly.** A call a fake has no fixture for is logged with an `UNEXPECTED` note and fails the scenario at
  exit (tuicr treats some forge errors as best effort, so the exit code alone would hide it). A scenario that provokes
  one on purpose calls `e2e_allow_unexpected_forge_calls`.
* **Exit status is checked.** `e2e_wait_exit` requires status 0 unless you pass the expected one (or `any`);
  `sc_capture` returns `SC_RC`/`SC_OUT` for non-interactive invocations.
* **Cleanup composes.** Register teardown with `e2e_on_exit '<command>'`; never `trap ... EXIT` in a scenario.
* **No machine assumptions.** Use `$E2E_ROOT`-relative paths (`e2e_dir $n`), `$E2E_DIR`, `$TUICR_BIN`; tools the child needs
  are on its PATH via `E2E_TOOL_PATH`. To make a tool's *absence* deterministic use `e2e_minimal_path`.
* Skip with `e2e_skip "reason"`. If a scenario is known to fail for a product/docs reason, add it to `quarantine.tsv`
  with the reason instead of ignoring the failure.
* Text assertions use short, distinctive fragments, not full-screen snapshots; coordinates and colours are used only
  where the behaviour *is* the coordinate or colour (mouse hit areas, themes).

### Fake forges

`fakeforge/core.py` owns the call log (`$E2E_FORGE_LOG`, one JSON per call), fixture lookup with variant overlays
(`E2E_VARIANT`, or `E2E_VARIANT_FILE` to change what the forge serves mid-run), per-run state files, and the
exit conventions (97 = unexpected call). An adapter (`gh.py`, ...) only maps that forge's REST/GraphQL routes to
fixture files under `data/<forge>/`. To teach a fake a new route add the fixture file and one `if` in the adapter.
Which fake answers is decided by the repository's origin URL (`sc_forge_open <name> <url>`).

## Debugging a failure

1. Read `$E2E_OUT/<area>/<behaviour>.log`: the failed wait prints the screen it saw.
2. `$E2E_OUT/<area>_<behaviour>.screen.txt` is the final screen; `*.forge.log` the fake-forge calls.
3. Re-run only that scenario with the scratch dir kept: `E2E_ROOT=/tmp/e2e E2E_KEEP_ROOT=1 e2e/run.sh area/behaviour`.
4. Watch it live: run the scenario by hand (`bash e2e/scenarios/<area>/<behaviour>.sh`), then
   `tmux -L <socket> attach -t e2e-<area>_<behaviour>` (the socket name is `$E2E_SOCKET`, printed by `ps`).
5. A timeout kills the scenario's whole process tree; look for `exceeded ...s` in the log.

## CI

`.github/workflows/e2e.yml` runs the suite on ubuntu-24.04 and macos-latest with real Mercurial (`E2E_REQUIRE_HG=1`), and uploads `$E2E_OUT` as an artifact on failure. See [COVERAGE.md](COVERAGE.md) for what is covered.
