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
e2e/selftest/run.py                                       # tests of the harness itself (no tuicr needed)
```

Exit status is 0 only if at least one scenario ran and none failed. Results per scenario are `PASS`, `FAIL`
(assertion, crash or timeout), `SKIP` (scenario exit 77, e.g. no Mercurial) and `QPASS`/`QFAIL` for
quarantined scenarios (see below). Logs, final screenshots (`*.screen.txt`), fake-forge call logs
(`*.forge.log`) and `results.tsv` (name, result, seconds) land in `$E2E_OUT` (default `$E2E_ROOT/out`).

## Requirements

| Tool | Version | Notes |
|---|---|---|
| tmux | 3.4+ (developed on 3.8) | provides the pty; no display or attached terminal needed |
| python3 | 3.9+, standard library only | the Python scenario library (`lib/e2e/`), fakes, selftest, the pty test of the installer |
| bash | 3.2+ (macOS) / 5 (Linux) | runner, and the scenarios and library not yet ported to Python |
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
`PYTHONPATH` is set by `run.sh` (`e2e/lib`); to run a Python scenario by hand set it yourself (see Debugging).

## Layout

```
e2e/
  run.sh              runner: select, timeout, classify, report
  quarantine.tsv      tracked known failures: <area>/<behaviour> <TAB> reason (run, reported, not gating)
  lib/e2e/            the Python scenario library (stdlib only, Python 3.9+), one module per responsibility
    core.py             environment, git/hg/locale isolation, Scenario (start, cli, sessions), @scenario, Failure/NoPane
    tui.py              terminal driver: Tui keys/type/click/paste/resize, screen, wait_for/wait_gone/reject/settle/wait_exit,
                        cursor_on, comment/cmd/save/quit, fake-forge call log readers
    waits.py            wait_file, wait_grep, wait_cmd (state outside the screen)
    repos.py            fixture repositories (git_repo, git_merge_base, pr_repo, wide_repo, long_file_repo, hg_repo, jj_repo, use_hg/use_jj)
    flows.py            open_review/diff, FakeClipboard (recording pbcopy/xclip/wl-copy), minimal_path
    forge.py            pr_open/forge_open, pr_comment, gerrit_open (REST server), pr_editor_snapshot
  lib/*.sh            the bash library of the scenarios not yet ported (init, core, tmux, assert, repos, flows, forge)
  lib/run_scenario.py runs one scenario (either language) in its own process group, kills the tree on timeout
  fakeforge/          ONE fake-forge core (core.py) + thin adapters gh.py glab.py tea.py bkt.py, gerrit.py (REST server)
  bin/                gh glab tea bkt: wrappers that put fakeforge first on the child's PATH
  data/               fixture data per forge (github/, gitlab/, gitea/, bitbucket/, gerrit/), variant overlays
  scenarios/<area>/<behaviour>.py   (or .sh while unported; never both for one behaviour: run.sh exits 2)
  selftest/           run.py: tests of the runner and the library (false-green paths, timeout kill, skip, zero selection, .sh+.py clash);
                      scenarios/ and scenarios-hung/ are the scenarios it runs
```

Areas: `local` (git/hg/commit selection), `tree`, `diff`, `comments`, `review` (marks, persistence), `theme`,
`input` (prompts, mouse, paste, selection), `help`, `ui`, `export`, `syntax`, `cli`, `pr` (GitHub PR flows), and one
per other forge: `gitlab`, `gitea`, `bitbucket`, `gerrit`, `azure`.

## Writing a scenario

A scenario is a short script named after the **behaviour it checks** (`pr/review_threads_reply_resolves.py`, not after a
PR or a unit). Put the PR/issue it covers in the header comment (`# Covers #901: ...`). New scenarios are Python;
the shell scenarios still present are ported one by one (see "Bash scenarios" below).

```python
#!/usr/bin/env python3
# Covers #123: filtering by `^src/` narrows the tree and the diff.
from e2e import scenario


@scenario
def main(s):                                    # s: the Scenario (name, scratch dir, environment, cleanups)
    tui = s.diff()                              # fixture repo, range of 2 commits, focus on the diff
    tui.keys(";h").settle()                     # focus the file tree
    tui.keys("i", "^src/", "Enter")
    tui.wait_for("Files · 0/2").wait_gone(r"README\.md")
    tui.quit()
```

`@scenario` runs `main(s)` and owns the exit status (0 pass, 1 fail, 77 skip). A failure prints the message, the
scenario's own `file:line: source` and the evidence (the screen, a log); no traceback from the library. Plain `assert`
works and reports the same way: give it a message (`assert x == y, f"... {x}"`).

### The API

Patterns are Python regexes matched **line by line** against the screen (like `grep -E`). Waits and key methods return
the `Tui`, so steps chain: `tui.cmd("theme").wait_for("Switch Theme").keys("Enter")`.

`Scenario` (`s`): `s.name` (`<area>_<behaviour>`), `s.dir` (scratch dir, `s.path_of(name)` for another TUI's),
`s.start(*tuicr_args, cwd=, env=, keep=, name=, cols=, rows=)` (tuicr with `--no-update-check` in a tmux pane; `cwd`
defaults to `<scratch>/work`; `env` is a **dict** of extra child variables; `keep=True` keeps home/data so a restart sees
saved sessions; `name=` starts a second TUI), `s.start_cmd(argv, ...)` (any command, same environment),
`s.cli(*args, input=, env=, path=, check=True)` (non-interactive tuicr, returns stdout, fails on a non-zero exit),
`s.cli_result(*args, cwd=)` (`.rc`, `.out` = stdout+stderr, never fails), `s.session()` (the one saved session, parsed JSON),
`s.session_json()`, `s.session_has(regex)`, `s.skip(reason)`, `s.on_exit(fn, *args)` (cleanups run in reverse order after
the TUIs stop; never install signal or atexit handlers), `s.allow_unexpected_forge_calls(why)`, `s.env` / `s.pre_path`
(variables and PATH directories applied to every later start; `use_hg`, `use_jj` and `fake_clipboard` add to them),
`s.cols` / `s.rows` (size of the next pane).

Fixtures: `s.diff()` / `s.open(*args)` (git fixture, tuicr started), `s.pr_open(variant)` / `s.forge_open(url, variant)`
(PR #1 through the fake `gh`/`glab`/`tea`/`bkt`, picked by the origin URL; `env=` for the fake), `e2e.pr_comment(tui, text)`,
`s.gerrit_open("root"|"threads", host=, project=, target=)` / `s.gerrit_stop()`, `e2e.forge.pr_editor_snapshot(s)`,
`s.fake_clipboard()` (create it before starting; `.text()` waits for what was copied, `.copied` says whether anything was),
`s.minimal_path(dir, "git")` (a PATH holding only those tools; pass as `path=` to `s.cli`), `s.use_hg()` / `s.use_jj()`
(SKIP without the real tool, FAIL with `E2E_REQUIRE_HG=1`/`E2E_REQUIRE_JJ=1`), and in `e2e.repos`: `git_repo`,
`git_merge_base` (`.repo .base .main .topic`), `pr_repo`, `wide_repo`, `long_file_repo`, `hg_repo`, `jj_repo`, `git(repo, *args)`,
`run(cwd, *argv)`, `write(path, text_or_bytes)`.

`Tui`: input `keys(*tmux_keys)`, `type(text)` (literal, `send-keys -l`), `click(col, row)`, `paste(text)`,
`resize(cols, rows)`, `kill()`; screen `screen(colors=, join=)`, `cursor_row()`, `colors_hash(*rows)` (rows are `1` or
`(3, 30)`; no argument hashes everything), `alive()`, `pane_exists()`, `exit_status()`; waits and checks
`wait_for` (alias `expect`), `wait_gone`, `reject(pattern, window=0.4)`, `check(pattern, message)` (no polling),
`settle(quiet=0.25)`, `wait_screen_change(before)`, `wait_exit(timeout, status=0|"any")`; flows `quit()`, `cmd("w")`,
`save()`, `comment(text, key="c")`, `cursor_on(regex)`; fake-forge log `posts()` (review POSTs of the fake `gh`),
`forge_posts(tool)`, `forge_calls()`, `forge_count(regex)`, `forge_expect` / `forge_reject`, `wait_forge_log(regex, timeout,
count=1)`. Timeouts default to `E2E_TIMEOUT` (10 s). Module functions: `wait_file(path)`, `wait_grep(path, regex)`,
`wait_cmd(timeout, argv_or_callable)`, `grep(regex, text)`, `must_fail(fn, message, exc)` (harness self-tests).

Rules that keep scenarios reliable:

* **Wait for the effect, never `sleep`.** After keys that start something asynchronous wait for the text it
  produces (`wait_for`/`wait_gone`), a file (`wait_file`, `wait_grep`), a command (`wait_cmd`), a fake-forge call
  (`wait_forge_log`) or process exit (`wait_exit(timeout, status)`). For "the UI finished redrawing" with nothing
  specific to look for use `settle()` (screen unchanged for 0.25 s). `save()` runs `:w` and waits for `Saved to`.
* **Wait first, then assert absence.** `reject` only proves something is absent *now*; put a positive barrier
  (`wait_for`) before it, otherwise it passes before the app has reacted.
* **A missing pane is a failure.** Every screen query raises `NoPane` when the tmux pane is gone; never turn that into
  "text disappeared", "forbidden text absent" or "process exited". `alive()` raises too, it does not return False.
* **Fakes fail loudly.** A call a fake has no fixture for is logged with an `UNEXPECTED` note and fails the scenario at
  exit (tuicr treats some forge errors as best effort, so the exit code alone would hide it). A scenario that provokes
  one on purpose calls `s.allow_unexpected_forge_calls(why)`.
* **Exit status is checked.** `wait_exit` requires status 0 unless you pass the expected one (or `"any"`); `s.cli`
  fails on a non-zero exit, `s.cli_result` returns `.rc` and `.out`.
* **Assert on structure, not text.** Parse JSON (`s.session()`, `json.loads(s.cli(...))`, `tui.posts()`) and assert on the
  fields instead of grepping a dump.
* **No machine assumptions.** Use `s.dir`-relative paths and the library's fixtures; tools the child needs are on its PATH.
  To make a tool's *absence* deterministic use `s.minimal_path`. Environment goes in dicts (`env={"EDITOR": path}`), not
  strings. No CPU or IO load generators in a scenario.
* Skip with `s.skip("reason")`. If a scenario is known to fail for a product/docs reason, add it to `quarantine.tsv`
  with the reason instead of ignoring the failure.
* Text assertions use short, distinctive fragments, not full-screen snapshots; coordinates and colours are used only
  where the behaviour *is* the coordinate or colour (mouse hit areas, themes).

### Bash scenarios (being ported)

The remaining `.sh` scenarios use the shell library in `lib/*.sh` (`. "$(dirname "$0")/../../lib/init.sh"`; `e2e_start`,
`e2e_keys`, `e2e_wait_for`, `sc_diff`, `sc_pr_open`, ...). Do not add new ones. The same rules apply, with
`e2e_allow_unexpected_forge_calls`, `e2e_on_exit '<command>'` and `e2e_skip` as the equivalents. Porting is
mechanical and must be faithful: same steps, same assertions, never weaker; delete the `.sh` in the same commit, because
`run.sh` exits 2 when `<behaviour>.sh` and `<behaviour>.py` both exist.

### Fake forges

`fakeforge/core.py` owns the call log (`$E2E_FORGE_LOG`, one JSON per call), fixture lookup with variant overlays
(`E2E_VARIANT`, or `E2E_VARIANT_FILE` to change what the forge serves mid-run), per-run state files, and the
exit conventions (97 = unexpected call). An adapter (`gh.py`, ...) only maps that forge's REST/GraphQL routes to
fixture files under `data/<forge>/`. To teach a fake a new route add the fixture file and one `if` in the adapter.
Which fake answers is decided by the repository's origin URL (`s.forge_open(url)`).

## Debugging a failure

1. Read `$E2E_OUT/<area>/<behaviour>.log`: it names the failed step (`at <scenario>:<line>`) and prints the evidence, e.g. the screen it saw.
2. `$E2E_OUT/<area>_<behaviour>.screen.txt` is the final screen; `*.forge.log` the fake-forge calls.
3. Re-run only that scenario with the scratch dir kept: `E2E_ROOT=/tmp/e2e E2E_KEEP_ROOT=1 e2e/run.sh area/behaviour`.
4. Watch it live: run the scenario by hand (`PYTHONPATH=e2e/lib TUICR_BIN=... python3 e2e/scenarios/<area>/<behaviour>.py`; `bash ...sh` for an unported one), then
   `tmux -L <socket> attach -t e2e-<area>_<behaviour>` (the socket name is `$E2E_SOCKET`, printed by `ps`).
5. A timeout kills the scenario's whole process tree; look for `exceeded ...s` in the log.

## CI

`.github/workflows/e2e.yml` runs the suite on ubuntu-24.04 and macos-latest with real Mercurial (`E2E_REQUIRE_HG=1`), and uploads `$E2E_OUT` as an artifact on failure. See [COVERAGE.md](COVERAGE.md) for what is covered.
