# tuicr

**A code review TUI with vim keybindings. Export to GitHub or clipboard.**

[![Crates.io](https://img.shields.io/crates/v/tuicr)](https://crates.io/crates/tuicr)
[![License](https://img.shields.io/crates/l/tuicr)](./LICENSE)
[![Website](https://img.shields.io/badge/website-tuicr.dev-green)](https://tuicr.dev)

![demo](./public/tuicr-demo.gif)

> [!TIP]
> Pronounced "tweaker".

## What it does

- GitHub-style continuous diff in the terminal. Scroll through every changed file in one stream.
- PR-style comments at the line, range, file, and review level, with classifications like
  `issue`, `suggestion`, `note`, and `praise`.
- Three export targets: push a real PR review to GitHub, copy structured markdown to your
  clipboard, or pipe to stdout.
- Works with git, jj, and mercurial. Reviews uncommitted changes, commit ranges, or any GitHub PR.

## Install

```bash
curl -fsSL tuicr.dev/install.sh | sh
# or
brew install agavra/tap/tuicr
```

<details>
<summary>Other install methods (cargo, mise, nix, binaries, source)</summary>

```bash
# Cargo
cargo install tuicr

# Mise
mise use github:agavra/tuicr

# Nix
nix run github:agavra/tuicr
```

Pre-built binaries: [GitHub Releases](https://github.com/agavra/tuicr/releases)

From source:

```bash
git clone https://github.com/agavra/tuicr.git
cd tuicr
cargo install --path .
```

</details>

## Quick start

```bash
tuicr                       # Pick from a commit selector
tuicr -w                    # Uncommitted changes (skip selector)
tuicr -r main..HEAD         # Commit range
tuicr pr 125                # GitHub PR
tuicr --stdout              # Pipe the review to stdout
```

Inside tuicr, navigate with `j`/`k`, press `c` to comment, then `y` to copy the review or
`:submit` to push it to GitHub. Auto-detects git, jj, or mercurial.

## How it compares

| | tuicr | [hunk](https://github.com/modem-dev/hunk) | [lumen](https://github.com/jnsahaj/lumen) | `gh pr review` | `git diff` |
|---|:---:|:---:|:---:|:---:|:---:|
| TUI diff viewer | ✅ | ✅ | ✅ | | |
| Write comments in the TUI | ✅ | agent-only¹ | ✅ | | |
| Vim keybindings | ✅ | | partial² | | |
| Push inline review to GitHub | ✅ | | | partial³ | |
| Agent-ready markdown export | ✅ | via CLI skill | | | |
| git / jj / hg | ✅ / ✅ / ✅ | ✅ / ✅ / | ✅ / ✅ / | | ✅ / / |
| Single static binary | ✅ | (needs Node) | ✅ | ✅ | ✅ |

¹ Hunk has a `hunk session comment add` CLI for agents to inject notes into a live TUI session.
No in-TUI commenting keybinding.

² Lumen has `j`/`k` navigation but no broader vim model (visual mode, `{N}G`, `Ctrl-d`/`Ctrl-u`,
etc.).

³ `gh pr review` posts approve/comment/request-changes at the review level only. No inline line
comments.

## Export your review

When you're done reviewing, send your comments wherever the work continues.

### To GitHub

`:submit` opens a picker for Comment, Approve, Request changes, or Draft. Inline comments land
on the right lines as a real PR review; review-level comments become the review summary.
Requires `gh` authenticated to the repo.

### To your coding agent

`y` or `:clip` copies a structured markdown block to your clipboard. Each comment has a number,
a classification, and a file/line anchor:

```markdown
I reviewed your code and have the following comments. Please address them.

Comment types: ISSUE (problems to fix), SUGGESTION (improvements), NOTE (observations), PRAISE (positive feedback)

1. [SUGGESTION] `src/auth.rs` - Consider adding unit tests
2. [ISSUE] `src/auth.rs:42` - Magic number should be a named constant
3. [NOTE] `src/auth.rs:50-55` - This block could be refactored
```

Paste it back to any coding agent (Claude, Codex, Cursor, etc).

For an agent-driven workflow where your agent opens tuicr in a tmux split pane, see
[skills/tuicr/SKILL.md](skills/tuicr/SKILL.md).

### To stdout

Run with `--stdout` to pipe the markdown to another process:

```bash
tuicr --stdout > review.md
tuicr --stdout | pbcopy
```

## Configuration

Path: `~/.config/tuicr/config.toml` on Linux/macOS, `%APPDATA%\tuicr\config.toml` on Windows.

```toml
theme = "catppuccin-mocha"
diff_view = "side-by-side"   # or "unified"
appearance = "system"        # or "dark" / "light"
mouse = true

[[comment_types]]
id = "issue"
color = "red"
definition = "must fix before merge"
```

Themes: `dark`, `light`, `ayu-light`, `ayu-mirage`, `onedark`, `github-light`, `github-dark`,
`catppuccin-latte`, `catppuccin-frappe`, `catppuccin-macchiato`, `catppuccin-mocha`,
`gruvbox-dark`, `gruvbox-light`, `nord-dark`, `nord-light`, `nord-dark-high-contrast`,
`nord-light-high-contrast`, `solarized-light`, `solarized-dark`, `tokyo-night-storm`.

Full options, theme resolution precedence, `comment_types` semantics, and `.tuicrignore` rules in
[docs/CONFIG.md](docs/CONFIG.md).

## Keybindings

A first-session cheatsheet. Press `?` inside tuicr for the full reference.

| Key | Action |
|---|---|
| `j` / `k` | Down / up |
| `Ctrl-d` / `Ctrl-u` | Half-page down / up |
| `g` / `G` | Top / bottom |
| `{` / `}` | Previous / next file |
| `[` / `]` | Previous / next hunk |
| `/` | Search |
| `c` / `C` | Add line / file comment |
| `v` / `V` | Visual mode (range comment) |
| `r` | Toggle file reviewed |
| `y` | Copy review to clipboard |
| `:submit` | Push review to GitHub |
| `?` | Toggle full help |

Full reference in [docs/KEYBINDINGS.md](docs/KEYBINDINGS.md).

## License

MIT licensed. Contribution notes in [CONTRIBUTING.md](CONTRIBUTING.md).
