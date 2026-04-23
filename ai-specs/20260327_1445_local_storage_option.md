# Local Storage Option for Review Sessions

**Date:** 20260327
**Type:** Feature Implementation
**Status:** Completed

## Summary

Added the ability to store review sessions within the repository (`.tuicr/reviews/`) instead of the global data directory (`~/.local/share/tuicr/reviews/`). This is useful for teams that want to share review sessions or keep review data versioned with the code.

## Changes Made

### 1. Config Support (`src/config/mod.rs`)

- Added `local_storage: Option<bool>` field to `AppConfig`
- Added "local_storage" to `KNOWN_KEYS` list
- Added parsing with `read_bool()` in `load_config_from_path()`
- Added tests for parsing `local_storage = true/false` and handling invalid types

### 2. CLI Flag Support (`src/theme/mod.rs`)

- Added `local_storage: bool` field to `CliArgs` struct
- Added parsing for `--local-storage` flag in CLI argument parser
- Updated help text to include the new flag description
- Added tests for parsing `--local-storage` flag

### 3. Storage Layer (`src/persistence/storage.rs`)

- Added `get_local_reviews_dir(repo_path: &Path) -> Result<PathBuf>` - returns `.tuicr/reviews` in the repository
- Added `get_storage_dir(repo_path: &Path, local_storage: bool) -> Result<PathBuf>` - dispatches to appropriate directory
- Modified `save_session()` to accept `local_storage: bool` parameter and use appropriate storage directory
- Modified `load_latest_session_for_context()` to accept `local_storage: bool` parameter
- Added tests for local storage functionality:
  - `should_save_and_load_with_local_storage`
  - `should_not_find_local_storage_session_in_global_dir`
- Updated all existing tests to pass `local_storage: false`

### 4. Application Layer (`src/app.rs`)

- Added `local_storage: bool` field to `App` struct
- Modified `App::new()` to accept `local_storage: bool` parameter
- Modified `App::build()` to accept and initialize `local_storage` field
- Updated all session loading functions to pass `local_storage`:
  - `load_or_create_commit_range_session()`
  - `load_or_create_staged_unstaged_and_commits_session()`
  - `load_or_create_session()`
- Updated all call sites to pass the parameter appropriately

### 5. Handler Layer (`src/handler.rs`)

- Updated save_session calls in command handlers to use `app.local_storage`

### 6. Main Entry Point (`src/main.rs`)

- Added logic to determine `local_storage` value (CLI flag takes precedence over config)
- Pass `local_storage` to `App::new()`
- Updated `save_session` call in ZZ keybinding handler

### 7. Documentation

- Updated `AGENTS.md` Data Flow section to document the new option

## Configuration

### Config File
```toml
# ~/.config/tuicr/config.toml
local_storage = true
```

### CLI Flag
```bash
tuicr --local-storage
```

### Precedence
CLI flag (`--local-storage`) takes precedence over config file setting.

## Storage Locations

- **Default (local_storage = false):** `~/.local/share/tuicr/reviews/` (XDG compliant)
- **Local Storage (local_storage = true):** `.tuicr/reviews/` within the repository

## Behavior

When `local_storage` is enabled:
1. Review sessions are saved to `.tuicr/reviews/` in the repository root
2. Session loading looks for existing sessions in the local directory only
3. The `.tuicr/` directory can be added to `.gitignore` if sessions should not be committed

## Testing

- All 300 existing tests pass
- New tests verify:
  - Sessions can be saved and loaded with local storage
  - Local storage sessions are not found when searching global storage
  - Config file parsing for the option
  - CLI flag parsing
