# Keybindings

Full reference. Press `?` inside tuicr for an in-app version of this list.

`<leader>` defaults to `;`. Override it with `leader = ","` in `~/.config/tuicr/config.toml`.

## Navigation

| Key | Action |
|-----|--------|
| `j` / `↓` | Scroll down |
| `k` / `↑` | Scroll up |
| `h` / `←` | Scroll left |
| `l` / `→` | Scroll right |
| `Ctrl-d` / `Ctrl-u` | Half page down / up |
| `Ctrl-f` / `Ctrl-b` | Full page down / up |
| `g` / `G` | Go to first / last file |
| `{N}G` | Go to source line N in current file |
| `{N}{motion}` | Vim-style count prefix — repeats `j` / `k` / `h` / `l` / `{` / `}` / `[` / `]` `N` times |
| `{` / `}` | Jump to previous / next file |
| `[` / `]` | Jump to previous / next hunk |
| `/` | Search within diff |
| `n` / `N` | Next / previous search match |
| `Enter` | Expand or collapse hidden context between hunks |
| `zt` | Scroll cursor to top of screen |
| `zz` | Center cursor on screen |
| `zb` | Scroll cursor to bottom of screen |

## File tree

| Key | Action |
|-----|--------|
| `Space` | Toggle expand directory |
| `Enter` | Expand directory / jump to file in diff |
| `o` | Expand all directories |
| `O` | Collapse all directories |

## Panel focus

| Key | Action |
|-----|--------|
| `Tab` / `Shift-Tab` | Cycle focus forward / backward between file list, comment navigator, diff, and commit selector |
| `<leader>h` | Focus file list (left panel) |
| `<leader>l` | Focus diff view (right panel) |
| `<leader>k` | Move focus up (comments to files, or diff/files to commit selector when visible) |
| `<leader>j` | Move focus down (files to comments when visible, otherwise diff) |
| `<leader>e` | Toggle file list visibility |
| `Enter` | Select file (when file list is focused) |

## Comment navigator

Shown below the file tree when local comments or visible remote PR threads exist.

| Key | Action |
|-----|--------|
| `j` / `k` | Move selection |
| `h` / `l` | Scroll rows left / right |
| `Enter` | Jump to selected comment |

## Review actions

| Key | Action |
|-----|--------|
| `r` | Toggle file reviewed |
| `c` | Add line comment (or file comment if not on a diff line) |
| `C` | Add file comment |
| `<leader>c` | Add review comment |
| `v` / `V` | Enter visual mode for range comments |
| `dd` | Delete comment at cursor |
| `i` | Edit comment at cursor |
| `y` | Copy review to clipboard |

## Visual mode

| Key | Action |
|-----|--------|
| `j` / `k` | Extend selection down / up |
| `c` / `Enter` | Create comment for selected range |
| `Esc` / `v` / `V` | Cancel selection |

## Comment mode

| Key | Action |
|-----|--------|
| `Tab` / `Shift-Tab` | Cycle comment type forward / backward (per `comment_types` order) |
| `Enter` / `Ctrl-Enter` / `Ctrl-s` | Save comment |
| `Shift-Enter` / `Ctrl-j` | Insert newline |
| `←` / `→` | Move cursor |
| `Ctrl-w` / `Alt-Backspace` / `Cmd-Backspace` | Delete word |
| `Ctrl-u` | Clear line |
| `Esc` / `Ctrl-c` | Cancel |

## Commands

In command mode,
`Tab` and `Shift-Tab` complete or cycle command names.

| Command | Action |
|---------|--------|
| `:{N}` | Jump to new-side line N in current file |
| `:o{N}` | Jump to old-side line N in current file (matches deletions) |
| `:w` | Save session |
| `:e` (`:reload`) | Reload diff files |
| `:edit` | Open focused file in `$EDITOR` |
| `:clip` (`:export`) | Copy review to clipboard |
| `:diff` | Toggle diff view (unified / side-by-side) |
| `:commits` | Select commits to review |
| `:submit` | Open submit picker (Comment / Approve / Request changes / Draft) |
| `:submit comment` | Submit a Comment review |
| `:submit approve` | Submit an Approve review |
| `:submit request-changes` | Submit a Request-changes review |
| `:submit draft` | Submit a Draft review (pending on GitHub) |
| `:set wrap` | Enable line wrap in diff view |
| `:set wrap!` | Toggle line wrap in diff view |
| `:set commits` | Show inline commit selector |
| `:set nocommits` | Hide inline commit selector |
| `:set commits!` | Toggle inline commit selector |
| `:clear` | Clear all comments |
| `:clearc` | Clear comments without clearing reviewed marks |
| `:version` | Show tuicr version |
| `:update` | Check for updates |
| `:q` | Quit (warns if unsaved) |
| `:q!` | Force quit |
| `:x` / `:wq` | Save and quit (prompts to copy if comments exist) |
| `ZZ` | Save and quit |
| `ZQ` | Quit without saving |
| `?` | Toggle help |
| `q` | Quick quit |

## Commit selection (startup)

| Key | Action |
|-----|--------|
| `j` / `k` | Move selection |
| `Space` | Toggle commit selection |
| `Enter` | Confirm and load diff |
| `q` / `Esc` | Quit |

## Inline commit selector

Shown at the top of the diff when reviewing multiple commits. Focus it with `<leader>k` or `Tab`.

| Key | Action |
|-----|--------|
| `j` / `k` | Navigate commits |
| `Space` / `Enter` | Toggle commit selection (updates diff) |
| `(` / `)` | Cycle through individual commits |
| `Esc` | Return focus to diff |

## Confirm dialogs

| Key | Action |
|-----|--------|
| `y` / `Enter` | Yes |
| `n` / `Esc` | No |

## Mouse

Mouse support is on by default. Disable with `mouse = false` in config.

| Action | Effect |
|--------|--------|
| Wheel up / down | Scroll the panel under the cursor (file list, comment navigator, diff, commit list, or help popup) without moving the cursor line |
| Click on a file | Jump to that file (lazygit-style) |
| Click on a directory | Expand or collapse it |
| Click on a diff line | Position the cursor on that line |
| Click on a commit | Toggle selection (or expand the row to load more) |
| Drag in diff | Highlight a range; press `y` to copy the selected source lines |

For full native terminal selection across the UI, hold your terminal's bypass modifier while dragging (usually **Shift** or **Option/Alt**, depending on the terminal).
