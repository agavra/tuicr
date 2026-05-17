# Configuration

tuicr reads a TOML config file at startup.

| Platform | Path |
|----------|------|
| Linux / macOS | `$XDG_CONFIG_HOME/tuicr/config.toml` (default: `~/.config/tuicr/config.toml`) |
| Windows | `%APPDATA%\tuicr\config.toml` |

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

backend = "libgit2"

comment_types = [
  { id = "note", label = "question", definition = "ask for clarification", color = "yellow" },
  { id = "suggestion", definition = "possible improvements" },
  { id = "issue", definition = "problems to fix" },
  { id = "praise", definition = "positive feedback" },
  { id = "nit", label = "nitpick", definition = "small optional tweaks", color = "#d19a66" },
]
```

## Options

| Key | Default | Description |
|-----|---------|-------------|
| `theme` | (none) | Explicit theme. See [Themes](#themes) for valid values. |
| `appearance` | `system` | `dark`, `light`, or `system`. Used when no explicit theme is set. |
| `theme_dark` | (none) | Theme for dark appearance (paired with `theme_light`). |
| `theme_light` | (none) | Theme for light appearance (paired with `theme_dark`). |
| `diff_view` | `unified` | `unified` or `side-by-side`. Toggle in-app with `:diff`. |
| `show_file_list` | `true` | Whether the file list panel is visible on startup. Toggle with `<leader>e`. |
| `mouse` | `true` | Wheel scrolling, clicks, and drag-to-select. |
| `leader` | `;` | Single-character prefix for panel focus, file-list toggle, and review-comment shortcuts. Invalid multi-character values are ignored with a startup warning. |
| `wrap` | `false` | Line wrap in the diff view. Toggle with `:set wrap!`. |
| `cursor_line` | `true` | Highlight the current cursor line and visual selection. |
| `transparent_background` | `true` | Let the terminal background show through panels. `false` paints the theme's `panel_bg`. |
| `scroll_offset` | `0` | Minimum lines visible above and below the cursor when scrolling (like Vim's `scrolloff`). |
| `no_update_check` | `false` | Skip startup update check when `true`. |
| `backend` | `libgit2` | Git backend: `libgit2` or `cli`. Sparse-checkout repos auto-route to `cli`. |
| `comment_types` | (built-in) | Comment categories. See [Comment types](#comment-types). |

## Themes

Built-in themes:

`dark`, `light`, `ayu-light`, `ayu-mirage`, `onedark`, `github-light`, `github-dark`, `catppuccin-latte`, `catppuccin-frappe`, `catppuccin-macchiato`, `catppuccin-mocha`, `everforest-dark`, `everforest-light`, `gruvbox-dark`, `gruvbox-light`, `nord-dark`, `nord-light`, `nord-dark-high-contrast`, `nord-light-high-contrast`, `solarized-light`, `solarized-dark`, `tokyo-night-storm`.

### Resolution precedence

When multiple sources are set, tuicr resolves the theme in this order:

1. `--theme <THEME>` flag
2. `theme` in the config file
3. `theme_dark` + `theme_light` in config (chosen by appearance)
4. `theme_dark` alone or `theme_light` alone in config (appearance ignored)
5. `--appearance <MODE>` flag (only when no explicit theme or variants are set)
6. `appearance` in config (only when no explicit theme or variants are set)
7. Built-in default (`system`)

Invalid `--theme` values cause an immediate non-zero exit.

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
