# Command Line Reference

Every flag and subcommand `tuicr` accepts, plus the environment variables it
reads and the output it produces for scripts and agents. `tuicr --help` is the
same list in condensed form.

Related references: [CONFIG.md](CONFIG.md) for the config file and themes,
[KEYBINDINGS.md](KEYBINDINGS.md) for keys and `:` commands inside the TUI, and
[REVIEW_CLI.md](REVIEW_CLI.md) for the `tuicr review` subcommands in full.

## Commands

| Form | What it does |
|---|---|
| `tuicr` | Open the TUI on the review target selector |
| `tuicr tui` | Same TUI, explicit subcommand |
| `tuicr pr <target>` (`tuicr mr`) | Review a pull request / merge request |
| `tuicr tui pr <target>` (`tuicr tui mr`) | Same, under the explicit TUI subcommand |
| `tuicr review list` | List persisted review sessions |
| `tuicr review add` | Add a local draft comment to a session |
| `tuicr review comments` (`tuicr review get`) | Print a session's comments |
| `tuicr update [VERSION]` | Update the installed binary |

`mr` is an alias of `pr`, and `get` an alias of `comments`; they behave
identically. `<target>` accepts a bare `<number>`, `<owner/repo#N>`, or a PR
URL.

`tuicr update` takes an optional SemVer version to install a specific release,
including an older known-good one. It only accepts a version number — there is
no `latest` keyword; omit the argument to get the newest release.

There is no `tuicr help` subcommand. Use `-h` / `--help`, which also works per
command (`tuicr review add --help`).

## Flags

These apply to the TUI and can be given on the bare command, on `tui`, or on
`pr` / `mr`. When the same option appears at more than one level, string values
take the last one given and boolean flags stay on once set, so
`tuicr --stdout tui pr 125 --theme nord-dark` is valid.

| Flag | Short | Value | Description |
|---|---|---|---|
| `--revisions` | `-r` | `REVSET` | Commit range / revset to review. Syntax depends on the VCS backend (`main..HEAD` for git, a jj revset for jj) |
| `--theme` | | `THEME` | Color theme. Bundled names resolve first, then `<config>/themes/<name>.toml` |
| `--appearance` | | `light`, `dark`, `system` | Appearance mode, used when no explicit theme is set |
| `--path` | `-p` | `PATH` | Filter the diff to a file or directory |
| `--working-tree` | `-w` | | Include uncommitted changes and skip the target selector |
| `--file` | | `PATH` | Open a file or directory for annotation with no VCS required |
| `--all-files` | `-A` | | Review every tracked file in the current repo |
| `--stdout` | | | Print the export to stdout instead of copying to the clipboard |
| `--no-update-check` | | | Skip the startup update check (same as `no_update_check` in the config) |
| `--repo-url` | | `URL` | Override the forge repo for PR operations |
| `--version` | `-V` | | Print the version and exit |
| `--help` | `-h` | | Print help and exit |

`--repo-url` accepts GitHub, GitLab, Gitea, Bitbucket, Azure DevOps, and Gerrit repos in HTTPS,
SCP-style SSH, or `ssh://` form — for example
`https://github.com/owner/repo`, `git@gitlab.com:owner/repo`, or
`https://dev.azure.com/org/project/_git/repo`. It is what to reach for when the
checkout's `origin` remote does not point at the forge repo you want to review
against.

### Scope selection

`--path`, `--working-tree`, `--file`, and `--all-files` pick what gets reviewed,
so they cannot be combined — passing two is a usage error. `--file` also
conflicts with `--revisions`, since annotating an arbitrary file has no commit
range.

`--path` on its own implies `--working-tree`. Pair it with `-r` to filter a
commit range instead:

```bash
tuicr -p src/cli.rs                 # uncommitted changes to one file
tuicr -r main..HEAD -p src/cli.rs   # that file's changes across a range
```

`--revisions` accepts leading hyphens, so revsets like `-r -3` reach the VCS
rather than being read as another flag.

TUI flags cannot be mixed with `review` or `update`, which do not open a TUI:

```bash
tuicr --stdout review list
# error: TUI options cannot be used with `tuicr review`; run `tuicr review --help` for command options
```

## Environment variables

No flag reads an environment variable as a fallback — anything below either
configures something with no flag equivalent, or is read by a tool tuicr shells
out to.

| Variable | Effect |
|---|---|
| `EDITOR` | Editor used by `e` and `:edit`. The config `editor` setting wins; `vi` is the last resort |
| `XDG_CONFIG_HOME`, `HOME`, `APPDATA` | Locate the config directory. See [CONFIG.md](CONFIG.md) |
| `TUICR_PROFILE` | Write a startup/render profile log. See below |
| `TUICR_PROFILE_FILE` | Path for that log, overriding the default location |
| `AZURE_DEVOPS_EXT_PAT`, `AZURE_DEVOPS_PAT` | Azure DevOps PAT. Checked in that order; without one tuicr falls back to `az rest`. See [AZURE.md](AZURE.md) |
| `GERRIT_URL` | Gerrit server root; also identifies Gerrit remotes on otherwise neutral hostnames. See [GERRIT.md](GERRIT.md) |
| `GERRIT_USERNAME`, `GERRIT_PASSWORD` | Gerrit REST API credentials. `GERRIT_PASSWORD` must be an HTTP password, not the account password |
| `TUICR_GLAB_DEBUG` | Log `glab` invocations for debugging. See [GITLAB.md](GITLAB.md) |
| `TMUX`, `ZELLIJ`, `SSH_TTY` | Detected to choose a clipboard path — OSC 52 over SSH, `tmux load-buffer` inside tmux |
| `XDG_SESSION_TYPE` | Picks `wl-copy` on Wayland or `xclip` on X11 |

Set `TUICR_PROFILE` to `1`, `true`, `on`, or `yes` to log to
`tuicr-profile.<timestamp>.log` in the system temp directory. Any other
non-empty value is treated as the log path itself; `0`, `false`, `off`, `no`,
and an empty value disable profiling. `TUICR_PROFILE_FILE` overrides the path
either way, but only when `TUICR_PROFILE` is enabled.

## Output for scripts and agents

Two markers go to **stderr**, so they survive on the user's scrollback and stay
out of piped output:

```
tuicr-session: agavra/tuicr@main/worktree
tuicr-summary: reviewed 3/3 files, 2 comments added
```

`tuicr-session:` is printed once at startup, before the alternate screen is
entered, and names the session slug. Pass it straight to
`tuicr review comments --session <slug>` to read comments while the TUI is
still open, or after it exits. `tuicr-summary:` is printed on exit, always —
including with zero comments, so a watcher can tell "reviewed everything, found
nothing" apart from "quit without looking".

`--stdout` prints the export markdown to stdout when the TUI exits, with the
TUI itself rendered to `/dev/tty`. Combined with the two stderr markers, that
keeps stdout clean for a pipe.

All three `tuicr review` subcommands print pretty JSON to stdout and nothing
else. There is no `--json` flag because JSON is the only format. See
[REVIEW_CLI.md](REVIEW_CLI.md) for the schemas.

`tuicr update` prints a single line describing what it did — updated, already
up to date, or completed through an external package manager.

## Exit codes

| Code | Meaning |
|---|---|
| `0` | Success, including `--help` and `--version` |
| `1` | Startup failure (not a repository, forge auth, missing PR, bad `--file` path) or a failed `review` / `update` command |
| `2` | Usage error (unknown flag, conflicting flags, invalid `--appearance` value) or a theme that could not be resolved |

## Not provided

tuicr ships no man page and no shell completions. `tuicr --help` and this page
are the reference.
