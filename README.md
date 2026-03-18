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
feedback the agent can act on in one pass.

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
- **Expandable context** - Press Enter on "... expand (N lines) ..." to reveal hidden context between hunks
- **Comments** - Add review-level, file-level, or line-level comments with types
- **Visual mode** - Select line ranges with `v` / `V` and comment on multiple lines at once
- **Review tracking** - Mark files as reviewed, persist progress to disk
- **`.tuicrignore` support** - Exclude matching files from review diffs
- **Clipboard export** - Copy structured Markdown optimized for LLM consumption
- **Session persistence** - Reviews auto-save and reload on restart
- **Jujutsu support** - Built-in jj support (tried first since jj repos are Git-backed)
- **Mercurial support** - Built-in hg support

## Installation

### Homebrew (macOS/Linux)

```bash
brew install agavra/tap/tuicr
```

### Pre-built binaries

Download the latest release for your platform from [GitHub Releases](https://github.com/agavra/tuicr/releases).

### Mise (macOS/Linux/Windows)

```
mise use github:agavra/tuicr
```

### From crates.io

```bash
cargo install tuicr
```

### Using Nix

```bash
# build tuirc (links binary to ./result/bin/tuirc)
nix build github:agavra/tuicr

# or just run with
nix run github:agavra/tuicr
```

### From source

```bash
git clone https://github.com/agavra/tuicr.git
cd tuicr
cargo install --path .
```

## Usage

Run `tuicr` in any git, jujutsu, or mercurial repository:

```bash
cd /path/to/your/repo
tuicr
```

Detection order: Jujutsu → Git → Mercurial. Jujutsu is tried first because jj repos are Git-backed.

### Options

| Flag | Description |
|------|-------------|
| `-r` / `--revisions <REVSET>` | Commit range/Revision set to review. Exact syntax depends on VCS backend (Git, JJ, Hg) |
| `--theme <THEME>` | Color theme override (`dark`, `light`, `ayu-light`, `onedark`, `catppuccin-latte`, `catppuccin-frappe`, `catppuccin-macchiato`, `catppuccin-mocha`, `gruvbox-dark`, `gruvbox-light`) |
| `--appearance <MODE>` | Appearance mode for default theme (`dark`, `light`, `system`) |
| `--stdout` | Output to stdout instead of clipboard when exporting |
| `--no-update-check` | Skip checking for updates on startup |

By default, `tuicr` starts in commit selection mode.  
If uncommitted changes exist, the first selectable entry is `Uncommitted changes`.  
When `-r` / `--revisions` is provided, `tuicr` opens that revision range directly.
On narrow terminals (less than 100 columns), `tuicr` starts with the file list hidden; toggle it with `;e`.

### Configuration

Set a default theme in:
- Linux/macOS: `$XDG_CONFIG_HOME/tuicr/config.toml` (default: `~/.config/tuicr/config.toml`)
- Windows: `%APPDATA%\tuicr\config.toml`

Examples:

```toml
theme = "catppuccin-mocha"

appearance = "system"
theme_dark = "gruvbox-dark"
theme_light = "gruvbox-light"

show_file_list = false
diff_view = "side-by-side"
wrap = true

comment_types = [
  { id = "note", label = "question", definition = "ask for clarification", color = "yellow" },
  { id = "suggestion", definition = "possible improvements" },
  { id = "issue", definition = "problems to fix" },
  { id = "praise", definition = "positive feedback" },
  { id = "nit", label = "nitpick", definition = "small optional tweaks", color = "#d19a66" }
]
```

`show_file_list` controls whether the file list panel is visible on startup (default: `true`). Toggle at runtime with `;e`.

`diff_view` sets the default diff layout: `"unified"` (default) or `"side-by-side"`. Toggle at runtime with `:diff`.

`wrap` enables line wrapping in the diff view (default: `false`). Toggle at runtime with `:set wrap!`.

`comment_types` replaces the default list and defines Tab cycle order.
Each entry requires `id` and can optionally set `label`, `definition`, and `color`.
Color accepts terminal names (for example `yellow`, `light_red`) or hex (`#RRGGBB`).

#### How `comment_types` works

- `id` is the stable internal value that is saved in sessions and used for matching.
- `label` is the visible tag shown in UI and export (`[QUESTION]`, `[NITPICK]`, etc.).
- `definition` is guidance text for LLMs, included in the exported `Comment types:` legend.
- `color` controls the comment badge/border color in the TUI.
- Declaring `comment_types` is a full replacement, if you define 2 types, only those 2 are available.
- If `comment_types` is missing, tuicr uses defaults: `note`, `suggestion`, `issue`, `praise`.
- Invalid entries are ignored with startup warnings, if all entries are invalid, tuicr falls back to defaults.

Minimal replacement example:

```toml
comment_types = [
  { id = "question", definition = "ask for clarification" },
  { id = "blocker", color = "red", definition = "must be fixed before merge" }
]
```

Theme resolution precedence:
1. `--theme <THEME>`
2. `theme` in config file path above (OS-specific)
3. `theme_dark` + `theme_light` in config (selected by appearance)
4. `theme_dark` only or `theme_light` only in config (appearance ignored)
5. `--appearance <MODE>` (only when no explicit theme or variants are set)
6. `appearance` in config (only when no explicit theme or variants are set)
7. built-in default (`system`)

Notes:
- Invalid `--theme` values cause an immediate non-zero exit.
- Unknown keys in `config.toml` are ignored with a startup warning.

### Ignoring Files With `.tuicrignore`

`tuicr` reads `.tuicrignore` from the repository root and excludes matching files from all review diffs.

Rules follow gitignore-style pattern matching, including `!` negation.

Example:

```gitignore
target/
dist/
*.lock
!Cargo.lock
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
| `{N}G` | Go to source line N in current file |
| `{` / `}` | Jump to previous/next file |
| `[` / `]` | Jump to previous/next hunk |
| `/` | Search within diff |
| `n` / `N` | Next/previous search match |
| `Enter` | Expand/collapse hidden context between hunks |
| `zz` | Center cursor on screen |

#### File Tree

| Key | Action |
|-----|--------|
| `Space` | Toggle expand directory |
| `Enter` | Expand directory / Jump to file in diff |
| `o` | Expand all directories |
| `O` | Collapse all directories |

#### Panel Focus

| Key | Action |
|-----|--------|
| `Tab` / `Shift-Tab` | Toggle focus forward/backward between file list, diff, and commit selector |
| `;h` | Focus file list (left panel) |
| `;l` | Focus diff view (right panel) |
| `;k` | Focus commit selector (top panel) |
| `;j` | Focus diff view |
| `;e` | Toggle file list visibility |
| `Enter` | Select file (when file list is focused) |

#### Review Actions

| Key | Action |
|-----|--------|
| `r` | Toggle file reviewed |
| `c` | Add line comment (or file comment if not on a diff line) |
| `C` | Add file comment |
| `;c` | Add review comment |
| `v` / `V` | Enter visual mode for range comments |
| `dd` | Delete comment at cursor |
| `i` | Edit comment at cursor |
| `y` | Copy review to clipboard |

#### Visual Mode

| Key | Action |
|-----|--------|
| `j` / `k` | Extend selection down/up |
| `c` / `Enter` | Create comment for selected range |
| `Esc` / `v` / `V` | Cancel selection |

#### Comment Mode

| Key | Action |
|-----|--------|
| `Tab` / `Shift-Tab` | Cycle comment type forward/backward (from `comment_types` order) |
| `Enter` / `Ctrl-Enter` / `Ctrl-s` | Save comment |
| `Shift-Enter` / `Ctrl-j` | Insert newline |
| `←` / `→` | Move cursor |
| `Ctrl-w` | Delete word |
| `Ctrl-u` | Clear line |
| `Esc` / `Ctrl-c` | Cancel |

#### Commands

| Command | Action |
|---------|--------|
| `:w` | Save session |
| `:e` (`:reload`) | Reload diff files |
| `:clip` (`:export`) | Copy review to clipboard |
| `:diff` | Toggle diff view (unified / side-by-side) |
| `:commits` | Select commits to review |
| `:set wrap` | Enable line wrap in diff view |
| `:set wrap!` | Toggle line wrap in diff view |
| `:set commits` | Show inline commit selector |
| `:set nocommits` | Hide inline commit selector |
| `:set commits!` | Toggle inline commit selector |
| `:clear` | Clear all comments |
| `:version` | Show tuicr version |
| `:update` | Check for updates |
| `:q` | Quit (warns if unsaved) |
| `:q!` | Force quit |
| `:x` / `:wq` | Save and quit (prompts to copy if comments exist) |
| `?` | Toggle help |
| `q` | Quick quit |

#### Commit Selection (startup)

| Key | Action |
|-----|--------|
| `j` / `k` | Move selection |
| `Space` | Toggle commit selection |
| `Enter` | Confirm and load diff |
| `q` / `Esc` | Quit |

#### Inline Commit Selector (multi-commit reviews)

When reviewing multiple commits, an inline commit selector panel appears at the top of the diff view. Focus it with `;k` or `Tab`.

| Key | Action |
|-----|--------|
| `j` / `k` | Navigate commits |
| `Space` / `Enter` | Toggle commit selection (updates diff) |
| `(` / `)` | Cycle through individual commits |
| `Esc` | Return focus to diff |

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
3. **[NOTE]** `src/auth.rs:50-55` - This block could be refactored
```

Each comment is numbered and self-contained with its file path and line number or range (if applicable).
If `comment_types` is configured, this legend and the `[TYPE]` tags reflect your configured labels and definitions.

## Session Persistence

Sessions are automatically saved to `~/.local/share/tuicr/reviews/` (XDG compliant). When you reopen `tuicr` in the same repository, your previous review progress (comments, reviewed status) is restored.

## Agent Integrations

tuicr ships a repo-managed skill bundle at `skills/tuicr/`.

It opens tuicr in a tmux split pane so you can review changes interactively and feed comments back to your coding agent.

**Usage:** `/tuicr` or ask your coding agent to "review my changes with tuicr".

### Claude Code

**Prerequisites:** Claude Code running inside tmux, tuicr installed.

**Installation** (choose one):

```bash
# Copy the shared skill into Claude's local skills directory
mkdir -p ~/.claude/skills
cp -r /path/to/tuicr/skills/tuicr ~/.claude/skills/tuicr
```

### Codex

**Prerequisites:** Codex running inside tmux, tuicr installed.

**Installation**:

```bash
# Copy the shared skill into the local agents skills directory
mkdir -p ~/.agents/skills
cp -r /path/to/tuicr/skills/tuicr ~/.agents/skills/tuicr
```
