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
| `m` / `M` | Jump to next / previous comment |
| `/` | Search within diff (case-insensitive); matches on diff content are highlighted and the status bar shows the `[current/total]` position (headers, comments, and PR info are searchable but not highlighted) |
| `n` / `N` | Next / previous search match (wraps around) |
| `Esc` | Clear search-match highlighting; the pattern is kept so `n` / `N` still work |
| `Enter` | Expand or collapse hidden context between hunks |
| `zt` | Scroll cursor to top of screen |
| `zz` | Center cursor on screen |
| `zb` | Scroll cursor to bottom of screen |

## Help

Press `?` to open help.

| Key | Action |
|-----|--------|
| `/` | Search within help (case-insensitive) |
| `n` / `N` | Next / previous help search match |
| `j` / `k` | Scroll down / up |
| `q` / `?` / `Esc` | Close help |

## File tree

| Key | Action |
|-----|--------|
| `Space` | Toggle expand directory |
| `Enter` | Expand directory / jump to file in diff |
| `o` | Expand all directories |
| `O` | Collapse all directories |
| `i` | Filter to files matching a regex (include) |
| `e` | Filter out files matching a regex (exclude) |
| `I` | Clear the include filter |
| `E` | Clear the exclude filter |
| `/` | Search file paths (substring) |
| `n` / `N` | Next / previous file-path match |

These keys are only active while the file tree is focused — in the diff, `i` still
edits the comment at the cursor and `/` still searches the diff.

### Filtering

`i` and `e` take case-insensitive regexes matched against each file's **full
relative path** (`^src/`, `\.rs$`, `test|spec`). Both can be active at once:
include runs first, then exclude removes from what's left.

A filter is not just a tree view — hidden files also disappear from the diff
pane, from `{`/`}` file navigation, `[`/`]` hunk navigation, and from the file
and `+/-` counts in the header. The tree title reports how much is hidden
(`Files · 2/12 · 12 of 58`), and the active patterns show in its bottom border.

Enter applies, `Esc` cancels, `Ctrl-u` clears the line. Reopening a prompt
pre-fills the pattern already applied, so submitting an emptied buffer is the
same as `I` / `E`. An invalid regex reports the error and leaves the prompt open
so it can be fixed. Comments on hidden files are not deleted and still export —
filters are a view, not an edit.

Filters are session-local: they are not written to the review session and reset
when tuicr restarts, but they survive a `:e` reload.

### Hiding reviewed files

`:set noreviewed` / `:set reviewed` / `:set reviewed!` (or the bare `:reviewed`)
hide or show files already marked reviewed with `r`. There is deliberately no
single-key binding: `H` is a vim motion, and this is a command-only feature. Like
`i` / `e`, hidden files leave the tree, the diff pane, `{`/`}` and `[`/`]`
navigation, `/` search, and the `+/-` counts in the header — so the header reports
the diff still left to review.

Two things stay deliberately unaffected. The tree title keeps counting reviewed
files in its `reviewed/total` fraction, since scoping it to the visible rows would
collapse it to `0/n` exactly when progress matters most; the bottom border carries
a `reviewed hidden` cue instead. And a file whose hunks are individually marked
with `R` is not hidden — only the file-level `r` flag counts.

While hiding, `r` becomes a burn-down loop: marking the file you are reading moves
you to the next unreviewed file, wrapping at the end. A hidden file cannot be
un-reviewed, because `r` can no longer reach it — `:set reviewed` brings it back.
Start a session with them hidden via `show_reviewed = false` in `config.toml`.

### Search

`/` moves only the tree selection to the next matching path, expanding collapsed
parents as needed, and leaves the diff viewport where it was — press `Enter` to
jump the diff there. `n` / `N` step matches and wrap around. Search only ever
considers files that pass the active filters.

## Panel focus

| Key | Action |
|-----|--------|
| `Tab` / `Shift-Tab` | Cycle focus forward / backward between file list, comment navigator, diff, and commit selector |
| `<leader>h` | Move focus one panel left (side-by-side: new side → old side → file list) |
| `<leader>l` | Move focus one panel right (side-by-side: file list → diff → old side → new side) |
| `<leader>k` | Move focus up (comments to files, or diff/files to commit selector when visible) |
| `<leader>j` | Move focus down (files to comments when visible, otherwise diff) |
| `<leader>e` | Toggle file list visibility |
| `<leader>s` | Toggle commit selector visibility (also `:set commits!`) |
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
| `R` | Toggle hunk reviewed |
| `c` | Add line comment (or file comment if not on a diff line) |
| `C` | Add file comment |
| `<leader>c` | Add review comment |
| `v` / `V` | Enter visual mode for range comments |
| `dd` | Delete comment at cursor |
| `i` | Edit comment at cursor (vim: text cursor at start) |
| `A` | Edit comment at cursor with text cursor at end (vim mode only) |
| `e` | Open focused file in `$EDITOR` |
| `y` | Copy review to clipboard |
| `Y` | Copy the comment at cursor to clipboard |

`e` opens the file at the cursor's line. Terminal editors (`vim`, `nvim`, `nano`, …)
take over the screen and tuicr reloads the diff once they exit. Windowed editors
(`code`, `cursor`, `zed`, `subl`, …) open in their own window while tuicr stays on
screen; reload with `:e` after editing. Adding `--wait` to `$EDITOR` opts a windowed
editor back into the blocking behaviour.

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

With `comment_vim = true` the box uses [`edtui`](https://github.com/preiter93/edtui)
modal editing (Normal/Insert/Visual: `hjkl`, `w`/`b`/`e`, `dd`/`D`/`ciw`/`x`,
`u`/`Ctrl-r`, visual `v`+`y`/`d`/`p`). From Normal mode `:w` (or `Enter` twice)
saves and `:q` (or `Esc`/`q` twice) cancels — the first press arms the action
and the header shows a confirm hint. `Alt-Enter` (Option+Enter) accepts and
`Alt-Esc` discards directly (no double-press) — Alt is the one modified
`Enter`/`Esc` that reaches the app across terminals, including browser/web
terminals like zellij web. `Tab` cycles the comment type in Normal
mode and inserts `comment_tab_width` spaces (default 4) in Insert mode; `Ctrl-s`
also saves.

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
| `:copy-url` | Copy the open PR URL to clipboard (PR mode) |
| `:summary` | Show all pending local-draft comments; `j`/`k` select and `Enter` jumps |
| `:diff` | Toggle diff view (unified / side-by-side) |
| `:vim` / `:novim` (`:set vim` / `:set novim`) | Enable/toggle/disable vim modal editing in the comment box (overrides `comment_vim`) |
| `:commits` | Select commits to review |
| `:submit` | Open submit picker (Comment / Approve / Request changes / Draft) |
| `:submit comment` | Submit a Comment review |
| `:submit approve` | Submit an Approve review |
| `:submit request-changes` | Submit a Request-changes review |
| `:submit draft` | Submit a Draft review (pending on GitHub) |
| `:set wrap` | Enable line wrap in diff view |
| `:set wrap!` | Toggle line wrap in diff view |
| `:set relativenumber` / `:set norelativenumber` | Enable / disable relative rendered-row numbers |
| `:set relativenumber!` | Toggle relative rendered-row numbers |
| `:set commits` | Show inline commit selector |
| `:set nocommits` | Hide inline commit selector |
| `:set commits!` | Toggle inline commit selector |
| `:set reviewed` | Show files already marked reviewed |
| `:set noreviewed` | Hide files already marked reviewed |
| `:set reviewed!` / `:reviewed` | Toggle files already marked reviewed |
| `:clear` | Clear all comments |
| `:clearc` | Clear comments without clearing reviewed marks |
| `:version` | Show tuicr version |
| `:update` | Check for updates |
| `:q` | Quit (warns on unsaved comments; discards review-only state) |
| `:q!` | Force quit |
| `:x` / `:wq` | Save and quit (prompts to copy if comments exist) |
| `ZZ` | Save and quit |
| `ZQ` | Quit without saving |
| `?` | Toggle help |

Pressing bare `q` no longer quits; it prints a reminder to use `:q` instead — a transitional
affordance that will be removed in a future release.

The summary replaces the diff while leaving the file sidebar visible when it is open. The first
pending comment is selected when the summary opens. Use `j`/`k` to select the next
or previous comment; the view scrolls automatically to keep the selection visible. `Enter` returns
to the continuous diff and jumps to the selected comment, leaving single-file view if necessary,
while `Esc` returns without jumping. Reviewed files and hunks are revealed for the jump without
losing their reviewed state.

`draft` applies to GitHub only. `comment` and `approve` work on GitHub, GitLab, and Bitbucket.
`request-changes` works on GitHub and GitLab, but not Bitbucket yet.

## Commit selection / review target selector

| Key | Action |
|-----|--------|
| `Tab` / `Shift-Tab` | Switch between Local and Pull Requests tabs |
| `j` / `k` | Move selection |
| `Space` | Toggle local commit selection |
| `Enter` | Confirm local commit range, open PR, or load more PRs |
| `/` | Filter currently loaded PR rows locally |
| `r` | In Pull Requests tab, toggle all open PRs / PRs requesting your review |
| `Esc` | Return to the diff |
| `:q` | Quit |

## Inline commit selector

Shown at the top of the diff when reviewing multiple commits. Focus it with `<leader>k` or `Tab`.
When opening a GitHub PR or GitLab MR you have reviewed before, tuicr may preselect only commits
newer than your latest submitted review; commits already covered by that review are marked with
`✓`. Use `Space` / `Enter` here to expand or adjust the range. Bitbucket does not record which
commit an approval covered, so no commits are preselected there.

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
