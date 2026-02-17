# tuicr - Terminal UI Code Review Tool

## Overview

A Rust terminal UI for reviewing code changes made by coding agents. Features vim keybindings, side-by-side diffs, file/line comments, and markdown output for feeding instructions back to agents.

## Requirements Summary

- **Input**: Git working tree changes (`git diff HEAD`)
- **Comments**: Both file-level and line-level
- **File Status**: Binary (reviewed / not reviewed)
- **Output**: Structured Markdown optimized for LLM consumption
- **Navigation**: Vim keybindings
- **Diff View**: Side-by-side (primary)
- **Persistence**: JSON files on disk

---

## Technology Stack

```toml
[dependencies]
ratatui = "0.29"           # TUI framework
crossterm = "0.29"         # Terminal backend
git2 = "0.19"              # Git operations
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
chrono = { version = "0.4", features = ["serde"] }
thiserror = "2.0"
anyhow = "1.0"
directories = "5.0"        # XDG paths
unicode-width = "0.2"
uuid = { version = "1.0", features = ["v4"] }
```

---

## Module Structure

```
src/
├── main.rs                 # Entry point, CLI args, main loop
├── app.rs                  # Application state
├── error.rs                # Error types
│
├── git/
│   ├── mod.rs
│   ├── diff.rs             # Diff parsing via git2
│   └── repository.rs       # Repo discovery
│
├── model/
│   ├── mod.rs
│   ├── review.rs           # ReviewSession, FileReview
│   ├── comment.rs          # Comment, CommentType
│   └── diff_types.rs       # DiffFile, DiffHunk, DiffLine
│
├── ui/
│   ├── mod.rs
│   ├── app_layout.rs       # Main layout
│   ├── file_list.rs        # File list panel
│   ├── diff_view.rs        # Side-by-side diff
│   ├── comment_panel.rs    # Comments display/input
│   ├── status_bar.rs       # Header and status
│   ├── help_popup.rs       # Keybinding help
│   └── styles.rs           # Colors and styling
│
├── input/
│   ├── mod.rs
│   ├── handler.rs          # Event processing
│   ├── keybindings.rs      # Key -> Action mapping
│   └── mode.rs             # Normal, Comment, Search modes
│
├── persistence/
│   ├── mod.rs
│   └── storage.rs          # Save/load sessions
│
└── output/
    ├── mod.rs
    └── markdown.rs         # Export to markdown
```

---

## Core Data Model

### ReviewSession
```rust
pub struct ReviewSession {
    pub id: String,
    pub repo_path: PathBuf,
    pub base_commit: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub files: HashMap<PathBuf, FileReview>,
    pub session_notes: Option<String>,
}
```

### FileReview
```rust
pub struct FileReview {
    pub path: PathBuf,
    pub reviewed: bool,
    pub file_comments: Vec<Comment>,
    pub line_comments: HashMap<u32, Vec<Comment>>,
    pub status: FileStatus,  // Added, Modified, Deleted, Renamed
}
```

### Comment
```rust
pub struct Comment {
    pub id: String,
    pub content: String,
    pub comment_type: CommentType,  // Note, Suggestion, Issue, Praise
    pub created_at: DateTime<Utc>,
    pub line_context: Option<LineContext>,
}
```

---

## UI Layout

```
+------------------------------------------------------------------+
|  tuicr - Code Review                            [branch: main]   |
+------------------+-----------------------------------------------+
|  FILES           |  ═══ src/main.rs [M] ══════════════════════   |
|  [x] src/main.rs |  OLD (HEAD)          │  NEW (working)         |
|  [ ] src/lib.rs  |   1 fn main() {      │   1 fn main() {        |
| ▶[ ] src/app.rs  |   2     let x = 1;   │   2     let x = 42;    |
|                  |   3 }                │   3 }                  |
|                  |  ─────────────────────────────────────────────|
|                  |  💬 Line 2: [ISSUE] Magic number...           |
|                  |  ═══ src/lib.rs [M] ═══════════════════════   |
|                  |  OLD (HEAD)          │  NEW (working)         |
|                  |   1 pub fn foo() {}  │   1 pub fn foo() {     |
|                  |                      │   2     bar();         |
|                  |                      │   3 }                  |
+------------------+-----------------------------------------------+
|  [N]ormal  j/k:scroll  Enter:jump  r:reviewed  c:comment  ?:help |
+------------------------------------------------------------------+
```

### Infinite Scroll Design (GitHub-style)

- **Diff panel**: Shows ALL changed files in one continuous scroll
  - Each file has a header separator (`═══ filename [status] ═══`)
  - Side-by-side diff for each file
  - Inline comments displayed below relevant lines
  - Scroll through entire review with `j/k` or `Ctrl-d/u`

- **File list panel**: Quick navigation aid (not selection)
  - Shows review status `[x]/[ ]` for each file
  - `▶` indicator shows which file is currently visible in diff
  - `Enter` on file list jumps to that file in the scroll
  - File list auto-highlights based on scroll position

- **Layout proportions**:
  - File list: 20% width (collapsible with `h`)
  - Diff view: 80% width (full width when file list collapsed)

---

## Vim Keybindings

### Navigation (Infinite Scroll)
| Key | Action |
|-----|--------|
| `j/k` | Scroll diff down/up one line |
| `Ctrl-d/u` | Half page down/up |
| `Ctrl-f/b` | Full page down/up |
| `gg/G` | Go to first/last file |
| `{/}` | Jump to previous/next file header |
| `[c/]c` | Jump to previous/next comment |
| `[[/]]` | Jump to previous/next hunk |

### File List Navigation
| Key | Action |
|-----|--------|
| `Tab` | Toggle focus between file list and diff |
| `Enter` | Jump to selected file in diff (when in file list) |
| `h` | Collapse file list panel |
| `l` | Expand file list panel / focus diff |

### Actions
| Key | Action |
|-----|--------|
| `r` | Toggle current file reviewed |
| `c` | Add line comment at cursor |
| `C` | Add file comment for current file |
| `e` | Edit comment under cursor |
| `dd` | Delete comment under cursor |
| `1-4` | Set comment type (1=Note, 2=Suggestion, 3=Issue, 4=Praise) |

### Session
| Key | Action |
|-----|--------|
| `:w` | Save session |
| `:e` | Reload diff files |
| `:clip` (`:export`) | Export markdown |
| `:q` | Quit |
| `:wq` / `ZZ` | Save and quit |
| `?` | Show help |

---

## Persistence

**Location**: `~/.local/share/tuicr/reviews/` (XDG compliant)

**Filename**: `{repo}_{commit}_{timestamp}.json`

Auto-saves every 60 seconds and on file navigation.

---

## Markdown Output Format

```markdown
# Code Review: {repo_name}

**Reviewed:** {timestamp}
**Base Commit:** `{commit_sha}`
**Files Reviewed:** {reviewed}/{total}

## Summary
{session_notes}

## Files

### M `src/auth.rs` [REVIEWED]

#### File Comments
> **[SUGGESTION]** Consider adding unit tests

#### Line Comments
```rust
// Line 42
    let expiry = now + 3600;
```
> **[ISSUE]** Magic number should be a named constant

---

## Action Items

1. **`src/auth.rs`:42** - Magic number should be a named constant
2. **`src/auth.rs`:15** - Use `?` operator instead of unwrap
```

---

## Implementation Order

### Phase 0: Overview
1. Setup README.md
2. Setup AGENTS.md

### Phase 1: Foundation
1. Set up Cargo.toml with dependencies
2. Create module structure (empty mod.rs files)
3. Implement `model/` structs with serde derives
4. Implement `error.rs` with thiserror

### Phase 2: Git Integration
5. Implement `git/repository.rs` - repo discovery
6. Implement `git/diff.rs` - parse `git diff HEAD` into DiffFile structs

### Phase 3: Persistence
7. Implement `persistence/storage.rs` - save/load JSON sessions

### Phase 4: TUI Core
8. Implement `app.rs` - application state container
9. Implement `input/mode.rs` and `input/keybindings.rs`
10. Implement `ui/styles.rs` - color definitions
11. Implement `ui/status_bar.rs` - header and footer

### Phase 5: TUI Widgets
12. Implement `ui/file_list.rs` - file list panel
13. Implement `ui/diff_view.rs` - side-by-side diff (most complex)
14. Implement `ui/comment_panel.rs` - comment display/input
15. Implement `ui/help_popup.rs` - keybinding overlay
16. Implement `ui/app_layout.rs` - compose all widgets

### Phase 6: Input Handling
17. Implement `input/handler.rs` - event loop and action dispatch

### Phase 7: Output
18. Implement `output/markdown.rs` - export functionality

### Phase 8: Main
19. Implement `main.rs` - CLI args, startup, main loop, shutdown

---

## Verification Plan

1. **Build**: `cargo build` should compile without errors
2. **Format**: `cargo fmt` after each file modification
3. **Manual test flow**:
   - Create a test git repo with multiple staged/unstaged changed files
   - Run `tuicr` in that directory
   - Scroll through all diffs with `j/k` and `Ctrl-d/u`
   - Verify file list `▶` indicator updates as you scroll
   - Use `{/}` to jump between file headers
   - Press `Tab` to focus file list, use `Enter` to jump to a file
   - Add line comments with `c`, file comments with `C`
   - Mark files reviewed with `r`
   - Save with `:w`, verify JSON file created
   - Export with `:clip`, verify markdown output
   - Quit with `:q`, restart, verify session loads with scroll position

---

## Key Files to Create

| File | Purpose |
|------|---------|
| `src/model/review.rs` | Core domain types |
| `src/git/diff.rs` | Git diff parsing |
| `src/ui/diff_view.rs` | Infinite-scroll side-by-side diff (all files in one view) |
| `src/input/handler.rs` | Vim keybinding implementation |
| `src/output/markdown.rs` | LLM-optimized output |
