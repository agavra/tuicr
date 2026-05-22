---
name: tuicr
description: Review local git changes with tuicr TUI via a tmux or zellij split pane
---

# tuicr - TUI Change Reviewer

Launch the `tuicr` TUI tool in a tmux or zellij split pane to interactively review local git changes.

## Usage

```
/tuicr [directory]
```

Or simply mention wanting to review changes with tuicr.

## How It Works

Since coding agents cannot run interactive TUI applications directly, this skill uses a terminal-multiplexer workaround:

1. Detects which multiplexer the agent session is running inside (tmux or zellij)
2. If detected: Invokes the matching wrapper script, which creates a split pane with tuicr running in it
3. If neither is detected: Provides instructions to restart the agent inside tmux or zellij

## Detecting the Multiplexer

Detect the active multiplexer **before** choosing a wrapper script:

| Environment | Multiplexer | Wrapper |
|-------------|-------------|---------|
| `$TMUX` is set (non-empty) | tmux | `tuicr-wrapper.sh` |
| `$ZELLIJ` is set (non-empty) | zellij | `tuicr-wrapper-zellij.sh` |
| Neither set | none | Tell user to restart the agent inside tmux or zellij |

If both are somehow set, prefer the innermost one (whichever the agent is directly running inside). When in doubt, ask the user.

## Determining the Directory

**Important:** You must determine the correct git repository directory based on context.

Consider:
- The user's current working directory
- Any repository they've been working in during the session
- Explicit directory mentioned in their request
- The git status output if available

Common patterns:
- "review my changes" -> use current working directory
- "review changes in myproject" -> find that repo path
- After editing files -> use the directory of those files

## Workflow

1. **Determine target directory**:
   - Check current working directory
   - Consider recent file operations
   - Ask user if ambiguous

2. **Detect multiplexer** and select the wrapper script:
   - tmux -> `<skill-directory>/tuicr-wrapper.sh`
   - zellij -> `<skill-directory>/tuicr-wrapper-zellij.sh`

3. **Run the wrapper script** with a 10-minute timeout:
   ```bash
   <skill-directory>/<wrapper-script> [directory]
   ```

   **IMPORTANT:** Always set `timeout: 600000` (10 minutes) on the Bash tool call.
   The script waits for tuicr to exit, and without the extended timeout the agent
   may background the command after 2 minutes.

4. **Handle the result**:
   - If successful: tuicr opens in a split pane and blocks until the user exits
   - If not in a supported multiplexer: relay the instructions to the user
   - If not a git repo: inform the user and ask for the correct path

5. **Process instructions from tuicr output**:

   The script output may contain instructions between markers:
   ```
   === TUICR INSTRUCTIONS ===
   <instructions here>
   === END TUICR INSTRUCTIONS ===
   ```

   **If instructions are present:** Parse and execute them. These are typically
   code changes or actions the user approved during the review.

   **If no instructions in output:** The script will say "No instructions exported"
   or mention clipboard. Tell the user:
   > "No instructions were exported from tuicr. If you exported to clipboard,
   > paste the instructions here and I'll execute them."

## Configuration

### tmux wrapper (`tuicr-wrapper.sh`)

| Variable | Default | Description |
|----------|---------|-------------|
| `TUICR_PANE_POSITION` | `top` | Pane position: `top` or `bottom` |
| `TUICR_PANE_SIZE` | `80` | Pane size as percentage of screen |

Example:
```bash
TUICR_PANE_SIZE=70 TUICR_PANE_POSITION=bottom <skill-directory>/tuicr-wrapper.sh /path/to/repo
```

### zellij wrapper (`tuicr-wrapper-zellij.sh`)

| Variable | Default | Description |
|----------|---------|-------------|
| `TUICR_PANE_DIRECTION` | `stacked` | Split direction: `down`, `right`, or `stacked` |
| `ZELLIJ_BIN` | `zellij` | Path to the zellij executable |

For backward compatibility the zellij wrapper also accepts the tmux-style
`TUICR_PANE_POSITION`: `top`/`bottom` -> `down`, `left`/`right` -> `right`,
`stacked` -> `stacked`.

Example:
```bash
TUICR_PANE_DIRECTION=right <skill-directory>/tuicr-wrapper-zellij.sh /path/to/repo
```

## Example Invocations

User says: "review my changes"
-> Detect multiplexer, run matching wrapper with the current working directory

User says: "let me review the changes in myproject"
-> Find myproject path based on context
-> Detect multiplexer, run matching wrapper with that path

User says: "/tuicr ~/projects/myapp"
-> Detect multiplexer, run matching wrapper with `~/projects/myapp`

## Tmux Tips (relay to user if needed)

- Switch between panes: `Ctrl-b` then arrow keys
- Close tuicr: Press `q` in tuicr (pane closes automatically)
- Resize panes: `Ctrl-b` then `Ctrl-arrow`
- Zoom current pane: `Ctrl-b` then `z` (toggle)

## Zellij Tips (relay to user if needed)

- Switch between panes: `Alt` + arrow keys
- Close tuicr: Press `q` in tuicr (pane closes automatically via `--close-on-exit`)
- Resize panes: `Ctrl-n` to enter resize mode, then arrow keys
- Toggle fullscreen for current pane: `Alt-f`
- Cycle through stacked panes: `Alt` + `[` / `]`

## Error Handling

| Error | Action |
|-------|--------|
| Neither `$TMUX` nor `$ZELLIJ` set | Tell the user to restart the agent inside tmux or zellij |
| `$TMUX` set but `tmux` binary missing | Tell the user to install tmux (or switch to zellij) |
| `$ZELLIJ` set but `zellij` binary missing | Tell the user to install zellij (or switch to tmux) |
| Not a git repo | Ask the user for the correct directory |
| `tuicr` not installed | Tell the user to install tuicr |
| tuicr already running (zellij) | Tell the user to switch to the existing pane via `Alt` + arrow keys |

## When NOT to use

- When the user just wants `git diff` output (use git directly)
- When reviewing remote/PR changes (use gh CLI or web)
- When the user explicitly asks for non-interactive review
