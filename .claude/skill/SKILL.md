---
name: tuicr
description: Review local git changes with tuicr TUI via tmux split pane
---

# tuicr - TUI Change Reviewer

Launch the `tuicr` TUI tool in a tmux split pane to interactively review local git changes.

## Usage

```
/tuicr [directory]
```

Or simply mention wanting to review changes with tuicr.

## How It Works

Since Claude Code cannot run interactive TUI applications directly, this skill uses a tmux workaround:

1. Detects if Claude is running inside tmux
2. If yes: Creates a split pane with tuicr running in it
3. If no: Provides instructions to restart Claude inside tmux

## Determining the Directory

**Important:** You must determine the correct git repository directory based on context.

Consider:
- The user's current working directory
- Any repository they've been working in during the session
- Explicit directory mentioned in their request
- The git status output if available

Common patterns:
- "review my changes" → use current working directory
- "review changes in myproject" → find that repo path
- After editing files → use the directory of those files

## Workflow

1. **Determine target directory**:
   - Check current working directory
   - Consider recent file operations
   - Ask user if ambiguous

2. **Run the wrapper script** with a 10-minute timeout:
   ```bash
   <skill-directory>/tuicr-wrapper.sh [directory]
   ```

   **IMPORTANT:** Always set `timeout: 600000` (10 minutes) on the Bash tool call.
   The script waits for tuicr to exit, and without the extended timeout Claude will
   background the command after 2 minutes.

3. **Handle the result**:
   - If successful: tuicr opens in a split pane and blocks until user exits
   - If not in tmux: relay the instructions to the user
   - If not a git repo: inform user and ask for correct path

4. **Process instructions from tuicr output**:

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

The wrapper script supports environment variables:

| Variable | Default | Description |
|----------|---------|-------------|
| `TUICR_PANE_POSITION` | `top` | Pane position: `top` or `bottom` |
| `TUICR_PANE_SIZE` | `80` | Pane size as percentage of screen |

Example with custom settings:
```bash
TUICR_PANE_SIZE=70 TUICR_PANE_POSITION=bottom <skill-directory>/tuicr-wrapper.sh /path/to/repo
```

## Example Invocations

User says: "review my changes"
→ Run tuicr-wrapper.sh with current working directory

User says: "let me review the changes in myproject"
→ Find myproject path based on context
→ Run tuicr-wrapper.sh with that path

User says: "/tuicr ~/projects/myapp"
→ Run tuicr-wrapper.sh with `~/projects/myapp`

## Tmux Tips (relay to user if needed)

- Switch between panes: `Ctrl-b` then arrow keys
- Close tuicr: Press `q` in tuicr (pane closes automatically)
- Resize panes: `Ctrl-b` then `Ctrl-arrow`
- Zoom current pane: `Ctrl-b` then `z` (toggle)

## Error Handling

| Error | Action |
|-------|--------|
| Not in tmux | Show restart instructions with `claude -c` |
| Not a git repo | Ask user for correct directory |
| tuicr not installed | Tell user to install tuicr |

## When NOT to use

- When user just wants `git diff` output (use git directly)
- When reviewing remote/PR changes (use gh CLI or web)
- When user explicitly asks for non-interactive review
