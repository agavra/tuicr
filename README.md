# tuicr: TUI for Code Review

<a href="https://crates.io/crates/tuicr" target="_blank"><img src="https://img.shields.io/crates/v/tuicr" alt="Crates.io"></a>
<a href="https://github.com/agavra/tuicr/blob/main/LICENSE" target="_blank"><img src="https://img.shields.io/crates/l/tuicr" alt="License"></a>
<a href="https://tuicr.dev" target="_blank"><img src="https://img.shields.io/badge/website-tuicr.dev-green" alt="Website"></a>

Review AI-generated diffs like a GitHub pull request, right from your terminal.

![demo](./public/tuicr-demo.gif)

## Why I built this

I use Claude a lot but there's no middle ground between "review every change"
and "accept all edits". Reviewing every change slows things down to human speed,
but accepting all edits makes the final review painful since I end up leaving
comments one at a time and wait after each fix.

`tuicr` is the middle ground. Let the agent loose, review the changes like a
normal PR, drop comments where needed, and export everything as structured
feedback Claude can act on in one pass.

It makes my AI-assisted development go brrrrrr.

> [!TIP]
> I pronounce it "tweaker"

## Overview

A GitHub-style diff viewer in your terminal with vim keybindings. Scroll through
changed files, leave comments, mark files as reviewed, and copy your full review
to clipboard in a format ready to paste back to the agent.

## Features

- **Infinite scroll diff view** - All changed files in one continuous scroll (GitHub-style)
- **Vim keybindings** - Navigate with `j/k`, `Ctrl-d/u`, `g/G`, `{/}`, `[/]`
- **Comments** - Add file-level or line-level comments with types
- **Review tracking** - Mark files as reviewed, persist progress to disk
- **Clipboard export** - Copy structured Markdown optimized for LLM consumption
- **Session persistence** - Reviews auto-save and reload on restart

## Installation

### Homebrew (macOS/Linux)

```bash
brew install agavra/tap/tuicr
```

### Pre-built binaries

Download the latest release for your platform from [GitHub Releases](https://github.com/agavra/tuicr/releases).

### From crates.io

```bash
cargo install tuicr
```

### From source

```bash
git clone https://github.com/agavra/tuicr.git
cd tuicr
cargo install --path .
```

## Usage

Run `tuicr` in any git repository:

```bash
cd /path/to/your/repo
tuicr
```

### Keybindings

#### Navigation

| Key | Action |
|-----|--------|
| `j` / `↓` | Scroll down |
| `k` / `↑` | Scroll up |
| `h` / `←` | Scroll left |
| `l` / `→` | Scroll right |
| `Ctrl-d` / `Ctrl-u` | Half page down/up |
| `Ctrl-f` / `Ctrl-b` | Full page down/up |
| `g` / `G` | Go to first/last file |
| `{` / `}` | Jump to previous/next file |
| `[` / `]` | Jump to previous/next hunk |
| `zz` | Center cursor on screen |

#### Panel Focus

| Key | Action |
|-----|--------|
| `Tab` | Toggle focus between file list and diff |
| `;h` | Focus file list (left panel) |
| `;l` | Focus diff view (right panel) |
| `;e` | Toggle file list visibility |
| `Enter` | Select file (when file list is focused) |

#### Review Actions

| Key | Action |
|-----|--------|
| `r` | Toggle file reviewed |
| `c` | Add line comment (or file comment if not on a diff line) |
| `C` | Add file comment |
| `dd` | Delete comment at cursor |
| `i` | Edit comment at cursor |
| `v` | Toggle diff view (unified / side-by-side) |
| `y` | Copy review to clipboard |

#### Comment Mode

| Key | Action |
|-----|--------|
| `Tab` | Cycle comment type (Note → Suggestion → Issue → Praise) |
| `Enter` / `Ctrl-Enter` / `Ctrl-s` | Save comment |
| `Shift-Enter` / `Ctrl-j` | Insert newline |
| `←` / `→` | Move cursor |
| `Ctrl-w` | Delete word |
| `Ctrl-u` | Clear line |
| `Esc` / `Ctrl-c` | Cancel |

#### Commands

| Key | Action |
|-----|--------|
| `:w` | Save session |
| `:e` (`:reload`) | Reload diff files |
| `:clip` (`:export`) | Copy review to clipboard |
| `:q` | Quit |
| `:x` / `:wq` | Save and quit (prompts to copy if comments exist) |
| `?` | Toggle help |
| `q` | Quick quit |

#### Commit Selection (when no unstaged changes)

| Key | Action |
|-----|--------|
| `j` / `k` | Move selection |
| `Space` | Toggle commit selection |
| `Enter` | Confirm and load diff |
| `q` / `Esc` | Quit |

#### Confirm Dialogs

| Key | Action |
|-----|--------|
| `y` / `Enter` | Yes |
| `n` / `Esc` | No |

## Review Output

When you export your review (`:clip` or confirm on `:wq`), `tuicr` copies structured Markdown to your clipboard. The format is optimized for pasting into AI agent conversations:

```markdown
I reviewed your code and have the following comments. Please address them.

Comment types: ISSUE (problems to fix), SUGGESTION (improvements), NOTE (observations), PRAISE (positive feedback)

1. **[SUGGESTION]** `src/auth.rs` - Consider adding unit tests
2. **[ISSUE]** `src/auth.rs:42` - Magic number should be a named constant
```

Each comment is numbered and self-contained with its file path and line number (if applicable).

## Session Persistence

Sessions are automatically saved to `~/.local/share/tuicr/reviews/` (XDG compliant). When you reopen `tuicr` in the same repository, your previous review progress (comments, reviewed status) is restored.
