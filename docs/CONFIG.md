# Configuration

tuicr reads a TOML config file at startup.

| Platform      | Path                                                                          |
| ------------- | ----------------------------------------------------------------------------- |
| Linux / macOS | `$XDG_CONFIG_HOME/tuicr/config.toml` (default: `~/.config/tuicr/config.toml`) |
| Windows       | `%APPDATA%\tuicr\config.toml`                                                 |

Local themes live in the sibling `themes/` directory:

| Platform      | Theme directory                                                       |
| ------------- | --------------------------------------------------------------------- |
| Linux / macOS | `$XDG_CONFIG_HOME/tuicr/themes/` (default: `~/.config/tuicr/themes/`) |
| Windows       | `%APPDATA%\tuicr\themes\`                                             |

Unknown keys are ignored with a startup warning.

## Full example

```toml
theme = "catppuccin-mocha"
appearance = "system"
theme_dark = "gruvbox-dark"
theme_light = "gruvbox-light"

diff_view = "side-by-side"
ignore_whitespace = false
show_file_list = true
show_pr_checks = true
show_pr_comments = true
mouse = true
leader = ","
comment_vim = false
comment_tab_width = 4
wrap = false
relative_line_numbers = false
cursor_line = true
search_highlight = true
transparent_background = true
scroll_offset = 5
no_update_check = false
review_watch_interval_ms = 1000
single_file_view = false
username = "user"

backend = "libgit2"

comment_types = [
  { id = "note", label = "question", definition = "ask for clarification", color = "yellow" },
  { id = "suggestion", definition = "possible improvements" },
  { id = "issue", definition = "problems to fix" },
  { id = "praise", definition = "positive feedback" },
  { id = "nit", label = "nitpick", definition = "small optional tweaks", color = "#d19a66" },
]

[forge]
comment_type_prefix = true

[export]
intro = "I reviewed your code and have the following comments. Please address them."
scope_line = true
pr_metadata = true
comments_header = "## Local tuicr Comments"
remote_comments_header = "## Existing GitHub Comments"
legend = true
```

## Options

| Key                        | Default      | Description                                                                                                                                                |
| -------------------------- | ------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `theme`                    | (none)       | Explicit theme name. See [Themes](#themes) for bundled names and local theme lookup.                                                                       |
| `appearance`               | `system`     | `dark`, `light`, or `system`. Used when no explicit theme is set.                                                                                          |
| `theme_dark`               | (none)       | Theme name for dark appearance (paired with `theme_light`).                                                                                                |
| `theme_light`              | (none)       | Theme name for light appearance (paired with `theme_dark`).                                                                                                |
| `diff_view`                | `unified`    | `unified` or `side-by-side`. Toggle in-app with `:diff`.                                                                                                   |
| `commit_order`             | `descending` | Inline commit selector order: `descending` (newest on top, the default) or `ascending` (oldest on top).                                                    |
| `initial_commit_selection` | `all`        | Which commits are selected when a multi-commit review first opens: `all`, or `oldest` to start on just the oldest commit and walk forward with `(` / `)`.  |
| `ignore_whitespace`        | `false`      | Ignore all whitespace in local Git, jj, and hg diffs. PR diffs are unchanged.                                                                              |
| `show_file_list`           | `true`       | Whether the file list panel is visible on startup. Toggle with `<leader>e`.                                                                                |
| `show_pr_checks`           | `true`       | Whether PR CI checks are fetched and shown. Set to `false` to skip GitHub check rollups.                                                             |
| `show_pr_comments`         | `true`       | Whether PR conversation comments are fetched and shown. Set to `false` to skip PR comments.                                                         |
| `show_commits`             | `true`       | Whether the inline commit selector pane is visible on startup for multi-commit reviews. Toggle with `<leader>s` or `:set commits!`.                        |
| `mouse`                    | `true`       | Wheel scrolling, clicks, and drag-to-select.                                                                                                               |
| `leader`                   | `;`          | Single-character prefix for panel focus, sidebar toggles, and review-comment shortcuts. Invalid multi-character values are ignored with a startup warning. |
| `comment_vim`              | `false`      | Vim modal editing in the comment box; toggle at runtime with `:vim`. When off, default emacs/readline bindings.                                            |
| `comment_tab_width`        | `4`          | Spaces inserted by Tab while typing in the vim comment box (Insert mode).                                                                                  |
| `wrap`                     | `false`      | Line wrap in the diff view. Toggle with `:set wrap!`.                                                                                                      |
| `relative_line_numbers`    | `false`      | Show gutter numbers as rendered-row distances from the cursor. Toggle with `:set relativenumber!`.                                                         |
| `cursor_line`              | `true`       | Highlight the current cursor line and visual selection.                                                                                                    |
| `search_highlight`         | `true`       | Highlight `/` search matches in the diff view. Clear at runtime with `Esc`; `n` / `N` re-enable.                                                           |
| `transparent_background`   | `true`       | Let the terminal background show through panels. `false` paints the theme's `panel_bg`.                                                                    |
| `scroll_offset`            | `0`          | Minimum lines visible above and below the cursor when scrolling (like Vim's `scrolloff`).                                                                  |
| `no_update_check`          | `false`      | Skip startup update check when `true`.                                                                                                                     |
| `review_watch_interval_ms` | `1000`       | Poll interval for persisted review-session changes. Set to `0` to disable automatic local-session reloads.                                                 |
| `single_file_view`         | `false`      | Start in single-file view for supported review targets. Pristine `--all-files` mode always starts in single-file view.                                     |
| `username`                 | `"user"`     | Display name stamped on local comments and used as the viewer identity for local comment coloring.                                                         |
| `backend`                  | `libgit2`    | Git backend: `libgit2` or `cli`. Sparse-checkout repos auto-route to `cli`.                                                                                |
| `comment_types`            | (none)       | Comment categories. Untyped by default. See [Comment types](#comment-types).                                                                               |
| `export_legend`            | `true`       | Include the `Comment types:` legend in the exported review. Superseded by `legend` under [Export](#export).                                                |

## Themes

Bundled themes:

`dark`, `light`, `ayu-light`, `ayu-mirage`, `onedark`, `github-light`, `github-dark`, `catppuccin-latte`, `catppuccin-frappe`, `catppuccin-macchiato`, `catppuccin-mocha`, `everforest-dark`, `everforest-light`, `gruvbox-dark`, `gruvbox-light`, `nord-dark`, `nord-light`, `nord-dark-high-contrast`, `nord-light-high-contrast`, `solarized-light`, `solarized-dark`, `tokyo-night-storm`, `tokyo-night-day`.

Local themes:

- `--theme <name>` and config `theme = "<name>"` first check bundled theme names, then try `<themes dir>/<name>.toml`.
- `theme_dark` and `theme_light` follow the same bundled-then-local lookup.
- Bundled names win if a local file uses the same name.
- TOML comments are supported, so local theme files can document where palette values came from.

### Local theme file format

Local theme files are flat TOML files with required palette keys matching tuicr's UI colors.
Use the checked-in example for a complete file, then adjust the palette values to taste.

```toml
# ~/.config/tuicr/themes/my-theme.toml
# Local theme file names are selected by theme name.
# `theme = "my-theme"` loads `my-theme.toml` from the local themes directory.

panel_bg = "#011627"
bg_highlight = "#1d3b53"
fg_primary = "#c3ccdc"
fg_secondary = "#a1aab8"
# `syntax_theme` points to a local `.tmTheme` file, relative to this file.
syntax_theme = "my-theme.tmTheme"

# Remaining keys are required. See `examples/tuicr-teal.toml` for the full list.
diff_add = "#21c7a8"
diff_del = "#ff5874"
status_bar_bg = "#252c3f"
mode_bg = "#82aaff"
```

Notes:

- Every listed color key is required.
- Color values accept named terminal colors or `#RRGGBB`.
- `syntax_theme` is optional. When present it must point to a local `.tmTheme` file.
- Relative `syntax_theme` paths resolve relative to the local theme TOML file.
- If `syntax_theme` is omitted, tuicr falls back to a bundled dark or light syntax theme based on the local theme background.
- `theme`, `theme_dark`, and `theme_light` may name either a bundled theme or a local theme file without the `.toml` suffix.
- A ready-to-copy example lives at [`examples/tuicr-teal.toml`](../examples/tuicr-teal.toml) with its matching [`examples/tuicr-teal-syntax.tmTheme`](../examples/tuicr-teal-syntax.tmTheme) syntax theme.

To try the checked-in example locally:

```sh
mkdir -p ~/.config/tuicr/themes
cp examples/tuicr-teal.toml examples/tuicr-teal-syntax.tmTheme ~/.config/tuicr/themes/
tuicr --theme tuicr-teal
```

### Resolution precedence

When multiple sources are set, tuicr resolves the theme in this order:

1. `--theme <THEME>` flag
2. `theme` in the config file
3. `theme_dark` + `theme_light` in config (chosen by appearance)
4. `theme_dark` alone or `theme_light` alone in config (appearance ignored)
5. `--appearance <MODE>` flag (only when no explicit theme or variants are set)
6. `appearance` in config (only when no explicit theme or variants are set)
7. Bundled default (`system`)

Invalid `--theme` values cause an immediate non-zero exit. The same is true when a selected
local theme file exists but is invalid. Invalid config-selected local themes emit startup warnings
and fall back through normal precedence.

## Comment types

Comment categories control:

- The classification badge shown in the TUI (color + label)
- The `[TYPE]` tag in the exported markdown
- The Tab cycle order in comment mode

### Fields

| Field        | Required | Description                                                                             |
| ------------ | -------- | --------------------------------------------------------------------------------------- |
| `id`         | yes      | Stable internal value. Saved in sessions and used for matching.                         |
| `label`      | no       | Visible tag in UI and export (`[QUESTION]`, `[NITPICK]`). Defaults to `id` uppercased.  |
| `definition` | no       | Guidance text for LLMs, included in the exported `Comment types:` legend.               |
| `color`      | no       | Comment badge / border color. Terminal name (`yellow`, `light_red`) or hex (`#RRGGBB`). |

### Defaults

If `comment_types` is missing, comments are **untyped** (`None`): no `[TYPE]` tag is prepended on
submit or export, and no badge is shown in the TUI. Define `comment_types` to opt into
classifications.

### The `None` type

`None` is always available regardless of config — it is the default when no types are configured,
and it is appended to the end of the Tab cycle when they are, so you can always leave a comment
untyped. An untyped comment never renders a `[TYPE]` tag, a badge, or a legend entry (file-level
comments still keep their `File-level:` marker on submit).

### Replacement semantics

`comment_types` is a full replacement of the _configured_ types. If you define 2 types, those 2 —
plus `None` — are available, and the first configured type becomes the default. Invalid entries are
ignored with startup warnings; if every entry is invalid, tuicr falls back to `None` only.

### Minimal example

```toml
comment_types = [
  { id = "question", definition = "ask for clarification" },
  { id = "blocker", color = "red", definition = "must be fixed before merge" },
]
```

## Forge

Settings under the `[forge]` section control how tuicr submits reviews to GitHub, GitLab, and Bitbucket.

```toml
[forge]
comment_type_prefix = false
```

| Key                   | Default | Description                                                                                                                                                                 |
| --------------------- | ------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `comment_type_prefix` | `true`  | Prepend `[TYPE] ` to comment bodies on submit (e.g. `[ISSUE] Magic number should be a constant`). Set to `false` to send the raw comment body without a classification tag. |

When enabled (the default), submitted comments look like:

```
[SUGGESTION] Consider adding unit tests
[ISSUE] Magic number should be a named constant
[NOTE] File-level: This module could use a doc comment
```

When disabled, the same comments are submitted without the prefix:

```
Consider adding unit tests
Magic number should be a named constant
This module could use a doc comment
```

This applies to inline line comments, file-level comments, and review-level comments pushed via `:submit`. The prefix works the same way on GitLab MR and Bitbucket PR submissions.

## Export

Settings under the `[export]` section control the Markdown that `y` and `:clip` copy to the clipboard, and that `--stdout` prints. They do not affect reviews you push to a forge with `:submit`; see [Forge](#forge) for those.

Every key defaults to what tuicr has always emitted, so your exports stay byte-identical until you set one. Setting a string key to `""` omits that line along with the blank line after it:

```toml
[export]
intro = ""
scope_line = false
comments_header = "## Comments"
```

| Key                      | Default                                                                      | Description                                                                                                                                 |
| ------------------------ | ---------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------- |
| `intro`                  | `I reviewed your code and have the following comments. Please address them.` | Opening line above the comment list. Set to `""` to omit it.                                                                                |
| `scope_line`             | `true`                                                                       | Emit the `Reviewing <scope>` line naming the staged, unstaged, commit, or pull request scope.                                               |
| `pr_metadata`            | `true`                                                                       | Emit the `URL:` and `Head:` lines in pull request mode. Independent of `scope_line`, because an agent needs both to fetch the pull request. |
| `comments_header`        | `## Local tuicr Comments`                                                    | Heading above comments you wrote in the TUI. Set to `""` to omit it.                                                                        |
| `remote_comments_header` | `## Existing GitHub Comments`                                                | Heading above unresolved forge threads. Appears only in pull request mode. Set to `""` to omit it.                                          |
| `legend`                 | `true`                                                                       | Emit the `Comment types:` legend. Takes precedence over the top-level `export_legend` key when set.                                         |

The example above produces an export that opens directly on the comment list:

```markdown
## Comments

1. **[ISSUE]** `src/auth.rs:42` - Magic number should be a named constant
```

### Relationship to `export_legend`

The top-level `export_legend` key predates this section and still works. When both are set, `legend` wins. When `[export]` omits `legend`, `export_legend` stays in force, so adding an `[export]` block to trim the intro will not switch the legend back on.

## .tuicrignore

tuicr reads `.tuicrignore` from the repository root and excludes matching files from all review diffs. Rules follow gitignore-style pattern matching, including `!` negation.

`.gitignore` is also honored automatically.

Example:

```gitignore
target/
dist/
*.lock
!Cargo.lock
```
