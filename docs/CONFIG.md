# Configuration

tuicr reads a TOML config file at startup.

| Platform | Path |
| --- | --- |
| Linux / macOS | `$XDG_CONFIG_HOME/tuicr/config.toml` (default: `~/.config/tuicr/config.toml`) |
| Windows | `%APPDATA%\tuicr\config.toml` |

Local themes live in the sibling `themes/` directory:

| Platform | Theme directory |
| --- | --- |
| Linux / macOS | `$XDG_CONFIG_HOME/tuicr/themes/` (default: `~/.config/tuicr/themes/`) |
| Windows | `%APPDATA%\tuicr\themes\` |

Unknown keys are ignored with a startup warning.

## Full example

```toml
theme = "catppuccin-mocha"
appearance = "system"
theme_dark = "gruvbox-dark"
theme_light = "gruvbox-light"

diff_view = "side-by-side"
show_file_list = true
mouse = true
leader = ","
wrap = false
cursor_line = true
transparent_background = true
scroll_offset = 5
no_update_check = false
review_watch_interval_ms = 1000

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
```

## Options

| Key | Default | Description |
|-----|---------|-------------|
| `theme` | (none) | Explicit theme name. See [Themes](#themes) for bundled names and local theme lookup. |
| `appearance` | `system` | `dark`, `light`, or `system`. Used when no explicit theme is set. |
| `theme_dark` | (none) | Theme name for dark appearance (paired with `theme_light`). |
| `theme_light` | (none) | Theme name for light appearance (paired with `theme_dark`). |
| `diff_view` | `unified` | `unified` or `side-by-side`. Toggle in-app with `:diff`. |
| `show_file_list` | `true` | Whether the file list panel is visible on startup. Toggle with `<leader>e`. |
| `mouse` | `true` | Wheel scrolling, clicks, and drag-to-select. |
| `leader` | `;` | Single-character prefix for panel focus, sidebar toggles, and review-comment shortcuts. Invalid multi-character values are ignored with a startup warning. |
| `wrap` | `false` | Line wrap in the diff view. Toggle with `:set wrap!`. |
| `cursor_line` | `true` | Highlight the current cursor line and visual selection. |
| `transparent_background` | `true` | Let the terminal background show through panels. `false` paints the theme's `panel_bg`. |
| `scroll_offset` | `0` | Minimum lines visible above and below the cursor when scrolling (like Vim's `scrolloff`). |
| `no_update_check` | `false` | Skip startup update check when `true`. |
| `review_watch_interval_ms` | `1000` | Poll interval for persisted review-session changes. Set to `0` to disable automatic local-session reloads. |
| `backend` | `libgit2` | Git backend: `libgit2` or `cli`. Sparse-checkout repos auto-route to `cli`. |
| `comment_types` | (built-in) | Comment categories. See [Comment types](#comment-types). |

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

| Field | Required | Description |
|-------|----------|-------------|
| `id` | yes | Stable internal value. Saved in sessions and used for matching. |
| `label` | no | Visible tag in UI and export (`[QUESTION]`, `[NITPICK]`). Defaults to `id` uppercased. |
| `definition` | no | Guidance text for LLMs, included in the exported `Comment types:` legend. |
| `color` | no | Comment badge / border color. Terminal name (`yellow`, `light_red`) or hex (`#RRGGBB`). |

### Defaults

If `comment_types` is missing, tuicr uses: `note`, `suggestion`, `issue`, `praise`.

### Replacement semantics

`comment_types` is a full replacement. If you define 2 types, only those 2 are available. Invalid entries are ignored with startup warnings; if every entry is invalid, tuicr falls back to defaults.

### Minimal example

```toml
comment_types = [
  { id = "question", definition = "ask for clarification" },
  { id = "blocker", color = "red", definition = "must be fixed before merge" },
]
```

## Forge

Settings under the `[forge]` section control how tuicr submits reviews to GitHub.

```toml
[forge]
comment_type_prefix = false
```

| Key | Default | Description |
|-----|---------|-------------|
| `comment_type_prefix` | `true` | Prepend `[TYPE] ` to comment bodies on submit (e.g. `[ISSUE] Magic number should be a constant`). Set to `false` to send the raw comment body without a classification tag. |

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

This applies to inline line comments, file-level comments, and review-level comments pushed via `:submit`.

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
