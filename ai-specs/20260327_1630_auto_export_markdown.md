# Auto-Export Markdown with Session Files

**Date:** 20260327
**Type:** Feature Implementation
**Status:** Completed

## Summary

Extended the session save functionality to automatically export a markdown version alongside the JSON session file. This provides human-readable review output that persists with the session data.

## Changes Made

### 1. Storage Layer (`src/persistence/storage.rs`)

- Extended `save_session()` signature to accept:
  - `diff_source: &DiffSource` - the source of the diff being reviewed
  - `comment_types: &[CommentTypeDefinition]` - configured comment types for markdown generation
- When saving a session, now also generates a `.md` file with the same base filename
- Added test helper functions `test_diff_source()` and `test_comment_types()` for test compatibility
- Updated all 19 existing test call sites to pass the new required parameters

### 2. Markdown Export (`src/output/markdown.rs`)

- Made `generate_markdown()` function public (was private)
- Reordered imports to fix clippy lint (base64 crate)
- Function now callable from storage layer for auto-export

### 3. Handler Layer (`src/handler.rs`)

- Updated `:w` (write) command to pass `diff_source` and `comment_types` to `save_session()`
- Updated `:x` / `:wq` (save and quit) command similarly

### 4. Main Entry Point (`src/main.rs`)

- Updated `ZZ` keybinding (export and quit) to pass required parameters to `save_session()`

## Behavior

When saving a session (via `:w`, `:wq`, `:x`, or `ZZ`):
1. JSON session file is saved as before (e.g., `20260327_144530_branch_main.json`)
2. Markdown export is now also saved (e.g., `20260327_144530_branch_main.md`)
3. Both files are stored in the same directory:
   - Global storage: `~/.local/share/tuicr/reviews/`
   - Local storage: `.tuicr/reviews/` (if `local_storage` is enabled)

## Storage Locations

Markdown files are saved alongside JSON files:
- **Global storage:** `~/.local/share/tuicr/reviews/*.md`
- **Local storage:** `.tuicr/reviews/*.md`

## Benefits

- Human-readable review output persists with session data
- Easy to view review comments without running tuicr
- Markdown files can be committed or shared independently
- Complements the clipboard export functionality (`:clip`, `y`)

## Testing

- All 300+ existing tests pass
- Test helpers provide default `DiffSource::WorkingTree` and standard comment types
- No new tests needed - functionality is exercised by existing save/load test suite
