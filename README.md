# tuicr

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

```bash
# Clone the repository
git clone https://github.com/yourusername/tuicr.git
cd tuicr

# Build and install
cargo install --path .
```

## Usage

Run `tuicr` in any git repository with uncommitted changes:

```bash
cd /path/to/your/repo
tuicr
```

### Keybindings

#### Navigation
| Key | Action |
|-----|--------|
| `j/k` | Scroll down/up |
| `Ctrl-d/u` | Half page down/up |
| `Ctrl-f/b` | Full page down/up |
| `g/G` | Go to first/last file |
| `{/}` | Jump to previous/next file |
| `[/]` | Jump to previous/next hunk |
| `Tab` | Toggle focus between file list and diff |
| `h/l` | Focus file list / diff panel |

#### Review Actions
| Key | Action |
|-----|--------|
| `r` | Toggle file reviewed |
| `c` | Add line comment |
| `C` | Add file comment |

#### Comment Mode
| Key | Action |
|-----|--------|
| `Tab` | Cycle comment type Note/Suggestion/Issue/Praise |
| `Ctrl-S` | Save comment |
| `Ctrl-C/Esc` | Cancel |

#### Commands
| Key | Action |
|-----|--------|
| `:w` | Save session |
| `:e` | Copy review to clipboard |
| `:q` | Quit |
| `:x` / `:wq` | Save and quit (prompts to copy if comments exist) |
| `?` | Show help |

## Review Output

When you export your review (`:e` or confirm on `:wq`), `tuicr` copies structured Markdown to your clipboard. The format is optimized for pasting into AI agent conversations:

```markdown
I reviewed your code and have the following comments. Please address them.

Comment types: ISSUE (problems to fix), SUGGESTION (improvements), NOTE (observations), PRAISE (positive feedback)

1. **[SUGGESTION]** `src/auth.rs` - Consider adding unit tests
2. **[ISSUE]** `src/auth.rs:42` - Magic number should be a named constant
```

Each comment is numbered and self-contained with its file path and line number (if applicable).

## Session Persistence

Sessions are automatically saved to `~/.local/share/tuicr/reviews/` (XDG compliant). When you reopen `tuicr` in the same repository, your previous review progress (comments, reviewed status) is restored.

