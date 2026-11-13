# tuicr

**Review AI-generated diffs like a GitHub pull request, right from your terminal.**

Comment on lines, ranges and files with vim keys, then hand the review to your coding agent or
post it to GitHub, GitLab, Gitea, Bitbucket, Azure DevOps or Gerrit.

[![Crates.io](https://img.shields.io/crates/v/tuicr)](https://crates.io/crates/tuicr)
[![License](https://img.shields.io/crates/l/tuicr)](./LICENSE)
[![Website](https://img.shields.io/badge/website-tuicr.dev-green)](https://tuicr.dev)

![demo](./public/tuicr-demo.gif)

> [!TIP]
> Pronounced "tweaker".

## Why tuicr

- **One continuous diff.** Every changed file scrolls past in one stream, as on a GitHub PR page.
- **Generated files collapse by default.** Press `Space` to expand them in the diff.
- **PR-style comments.** Comment on a line, a range, a file or the whole review, and suggest code changes.
- **Review tracking.** Mark files or hunks reviewed. The marks persist across sessions.
- **git, jj and Mercurial.** uncommitted changes, commit ranges, or a forge PR/MR.
- **Two ways out.** Copy numbered, agent-ready markdown, or push a real inline review to six forges.

## Install

```bash
curl -fsSL tuicr.dev/install.sh | sh
# or
brew install tuicr
# or
sudo pacman -S tuicr
```

<details>
<summary>Other install methods (cargo, mise, nix, Home Manager, binaries, source)</summary>

```bash
cargo install tuicr                 # Cargo
mise use github:agavra/tuicr        # Mise
nix run github:agavra/tuicr         # Nix (temporary; `nix profile install` to keep it)
programs.tuicr.enable = true;       # Home Manager, see docs/CONFIG.md
```

Pre-built binaries: [GitHub Releases](https://github.com/agavra/tuicr/releases). Linux releases
include dynamically linked GNU and static musl variants.

From source: `git clone https://github.com/agavra/tuicr.git && cd tuicr && cargo install --path .`

</details>

`tuicr update` upgrades the active installation. See [docs/CLI.md](docs/CLI.md#updating) for
which install methods it supports.

## 60-second walkthrough

```bash
tuicr -w                    # review uncommitted changes (plain `tuicr` opens a target picker)
tuicr -r main..HEAD         # or a commit range
tuicr pr 125                # or a PR/MR/change on any supported forge
```

1. Scroll with `j` / `k`; jump between files with `{` / `}`.
2. Press `c` on a line, type your comment, and press `Enter` to save it. `v` selects a range first.
3. Press `r` to mark the file reviewed. The sidebar tracks progress.
4. Press `y` to copy the review, then paste it into your coding agent.
   In a PR, run `:submit` instead to post the review on the forge.
5. `:wq` saves and quits (with comments, it first offers to copy the review). Reopen the same target later and your comments and marks are still there.

Press `?` at any time for the full key reference.

## Using it with a coding agent

`y` (or `:clip`) copies a numbered markdown review. After `tuicr -r main..HEAD`, with the
default settings, it looks like this:

```markdown
## Session: acme/app@feature-login/commits/1a2b3c4..9f8e7d6

I reviewed your code and have the following comments. Please address them.

Reviewing commits: 9f8e7d6, 1a2b3c4

## Local tuicr Comments

1. `src/auth.rs` - Consider adding unit tests
2. `src/auth.rs:42` - Magic number should be a named constant
3. `src/auth.rs:50-55` - This block could be refactored
```

Paste it into any coding agent (Claude, Codex, Cursor, etc.). `tuicr --stdout` prints the same
markdown to stdout. The `[export]` config section rewords or drops every line except the numbered
comments, and `comment_types` adds `**[ISSUE]**`-style tags ([docs/CONFIG.md](docs/CONFIG.md)).

- **Agent-launched panes.** The [tuicr skill](skills/tuicr/SKILL.md) lets your agent open tuicr in
  a cmux, Rex, tmux, Zellij or Herdr split pane, then read your comments when you are done.
- **Scripting saved reviews.** `tuicr review` lists, reads, adds and deletes comments without
  opening the TUI, for example
  `tuicr review add --session <slug> --target-file src/main.rs --line 42 "Handle the empty case"`.
  See [docs/REVIEW_CLI.md](docs/REVIEW_CLI.md). Rust tools can use the
  [library API](docs/LIBRARY.md).

## Reviewing pull requests

`tuicr pr` reviews the PR for the current branch, and `tuicr pr <number|owner/repo#N|URL>` any
other one (`tuicr mr` is an alias). `:submit` opens a picker with Comment, Approve, Request changes
and Draft. Inline comments land on their lines; review-level comments become the review summary.

| Forge | Tool / auth | Submit notes | Guide |
|---|---|---|---|
| GitHub | [`gh`](https://cli.github.com/) (`gh auth login`) | Draft leaves a pending review | [docs/GITHUB.md](docs/GITHUB.md) |
| GitLab | [`glab`](https://gitlab.com/gitlab-org/cli) | Draft posts draft notes; Request changes needs you as an assigned reviewer | [docs/GITLAB.md](docs/GITLAB.md) |
| Gitea | [`tea`](https://gitea.com/gitea/tea) (`tea logins add`) | Draft leaves a pending review; ranges collapse to their last line | [docs/GITEA.md](docs/GITEA.md) |
| Bitbucket Cloud | `bkt` | Draft posts pending comments; Data Center unsupported | [docs/BITBUCKET.md](docs/BITBUCKET.md) |
| Azure DevOps | `AZURE_DEVOPS_EXT_PAT`, or `az login` | Approve / Request changes cast a vote; no drafts, so Draft posts like Comment; needs a local clone | [docs/AZURE.md](docs/AZURE.md) |
| Gerrit | `GERRIT_USERNAME` / `GERRIT_PASSWORD` | Approve votes Code-Review +2, Request changes -1; Draft keeps Gerrit drafts; needs a local clone | [docs/GERRIT.md](docs/GERRIT.md) |

## Essential keys

| Key | Action |
|---|---|
| `j` / `k`, `Ctrl-d` / `Ctrl-u` | Scroll a line / half a page |
| `{` / `}`, `[` / `]` | Previous / next file, hunk |
| `<leader>p` | Jump to a file by path (leader is `;` by default) |
| `c` / `C` | Comment on the line / file |
| `v` / `V` | Visual mode for a range comment (`s` suggests a code change) |
| `r` / `R` | Toggle file / hunk reviewed |
| `/`, `n` / `N` | Search the diff, next / previous match |
| `y` | Copy the review to the clipboard |
| `:submit` | Post the review to the PR's forge |
| `:w`, `:wq`, `:q` | Save, save and quit, quit |
| `?` | Full help |

Every key and `:` command: [docs/KEYBINDINGS.md](docs/KEYBINDINGS.md).

## Configuration

Path: `~/.config/tuicr/config.toml` on Linux/macOS, `%APPDATA%\tuicr\config.toml` on Windows.

```toml
theme = "catppuccin-mocha"   # 24 bundled themes, or your own
diff_view = "side-by-side"   # or "unified"
editor = "nvim"              # for `e` / `:edit`; overrides $EDITOR
leader = ";"                 # prefix for leader shortcuts

[[comment_types]]            # optional; reviews are untyped by default
id = "issue"
color = "red"
definition = "must fix before merge"
```

All options, the theme list, local themes, Home Manager settings and `.tuicrignore`:
[docs/CONFIG.md](docs/CONFIG.md).

## How it compares

| | tuicr | [hunk](https://github.com/modem-dev/hunk) | [lumen](https://github.com/jnsahaj/lumen) | `gh pr review` | `git diff` |
|---|:---:|:---:|:---:|:---:|:---:|
| TUI diff viewer | ✅ | ✅ | ✅ | ❌ | ❌ |
| Write comments in the TUI | ✅ | ✅ | ✅ | ❌ | ❌ |
| Vim keybindings | ✅ | ❌ | partial¹ | ❌ | ❌ |
| Push inline review to GitHub | ✅ | ❌ | ❌ | partial² | ❌ |
| Push inline review to GitLab, Gitea, Bitbucket, Azure DevOps, Gerrit | ✅ | ❌ | ❌ | ❌ | ❌ |
| Agent-ready markdown export | ✅ | via CLI skill | ❌ | ❌ | ❌ |
| git | ✅ | ✅ | ✅ | ❌ | ✅ |
| jj | ✅ | ✅ | ✅ | ❌ | ❌ |
| Mercurial (hg) | ✅ | ❌ | ❌ | ❌ | ❌ |
| Standalone binary, no runtime needed | ✅ | ✅ | ✅ | ✅ | ✅ |

¹ Lumen has `j`/`k` navigation but no broader vim model (visual mode, `{N}G`, `Ctrl-d`/`Ctrl-u`,
etc.).

² `gh pr review` posts approve/comment/request-changes at the review level only. No inline line
comments.

## Documentation

| Page | Covers |
|---|---|
| [docs/CLI.md](docs/CLI.md) | Commands, flags, environment variables, exit codes, `tuicr update`, VCS backends |
| [docs/KEYBINDINGS.md](docs/KEYBINDINGS.md) | Every key and `:` command, the target selector and the Sessions tab |
| [docs/CONFIG.md](docs/CONFIG.md) | `config.toml` options, themes, comment types, export format, `.tuicrignore` |
| [docs/REVIEW_CLI.md](docs/REVIEW_CLI.md) | `tuicr review` for scripts and agents |
| [docs/LIBRARY.md](docs/LIBRARY.md) | The Rust library API (`ReviewStore`) |
| [docs/GITHUB.md](docs/GITHUB.md), [docs/GITLAB.md](docs/GITLAB.md), [docs/GITEA.md](docs/GITEA.md) | Forge setup, submit behaviour, limitations |
| [docs/BITBUCKET.md](docs/BITBUCKET.md), [docs/AZURE.md](docs/AZURE.md), [docs/GERRIT.md](docs/GERRIT.md) | Forge setup, submit behaviour, limitations |
| [skills/tuicr/SKILL.md](skills/tuicr/SKILL.md) | The agent skill and its split-pane wrappers |

## Sponsors

Thanks to the folks below for keeping tuicr development going, it means a lot to have the
work I'm doing here appreciated!

<p>
  <a href="https://www.coderabbit.ai/">
    <picture>
      <source media="(prefers-color-scheme: dark)" srcset="./public/sponsors/coderabbit-dark.svg">
      <img src="./public/sponsors/coderabbit-light.svg" alt="CodeRabbit" height="40">
    </picture>
  </a>
</p>

## License

MIT licensed. Contribution notes in [CONTRIBUTING.md](CONTRIBUTING.md).
