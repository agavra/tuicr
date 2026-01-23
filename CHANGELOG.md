# Changelog

All notable changes to this project will be documented in this file.

## [0.5.0] - 2026-01-23

### Bug Fixes

- Use absolute path for git repository discovery in worktrees (#123)
- Parse paths from rename/copy metadata and binary file lines (#124)
- Correct scroll behavior when line wrapping is enabled (#130)
- **ui:** Status bar not appearing on commit panel (#121)
- **clipboard:** Prefer OSC 52 in tmux/SSH sessions (#135)

### Features

- Add line range comment support with visual selection mode (#115)
- Add manual commit selection mode (#91)
- Add vim-style warning on exit with unsaved changes (#122)

### Ci

- Use cargo-binstall for faster jj installation (#126)
## [0.4.0] - 2026-01-17

### Bug Fixes

- Replace tabs with space (#106)

### Documentation

- Update demo for v0.3.0 (#99)

### Features

- Add optional Mercurial (hg) support (#93)
- Add OSC 52 clipboard fallback for remote sessions (#94)
- Add optional Jujutsu (jj) support (#96)
- Add Ctrl+C twice to exit (#100)
- Add commit selection support for hg and jj backends (#103)
- Display VCS type in status bar header (#102)
- Add PageUp/PageDown key support for scrolling (#112)

### Refactor

- Introduce VCS abstraction layer (#92)

### Ui

- Add theme support with dark and light modes (#105)
## [0.3.0] - 2026-01-15

### Bug Fixes

- Enforce scroll bounds to prevent scrolling past content (#75)
- `r` when focused on file viewer should mark file reviewed (#85)
- Lines at the bottom of diff were clipped (#89)

### Features

- Use `/` to enter search mode (#79)
- Support command `:clear` to clear comments (#80)
- Improve commenting experience navigation (#83)
- Improve color theme contrast (#84)
- Support cmd+delete to delete last word in comment (#87)
- Add line wrapping for unified view (#88)
## [0.2.0] - 2026-01-13

### Bug Fixes

- Support wayland clipboard. update arboard dependency to include wayland-data-control feature (#54)

### Features

- Add horizontal scroll to file list and ;h/;l panel navigation (#56)
- Add hierarchical file tree with expand/collapse (#50)
- Add support for expanding/collapsing files (#69)
- Enforce contiguous commit range selection (#70)

### Refactor

- Improve signal handling (#65)
## [0.1.3] - 2026-01-11

### Features

- Add scrolling support for file list panel (#47)
## [0.1.2] - 2026-01-10

### Documentation

- Add Homebrew installation and tap update instructions

### Features

- Add commit selection when no unstaged changes (#38)

### Release

- V0.1.2 (#46)
## [0.1.1] - 2026-01-09

### Bug Fixes

- Use native macOS runners for each architecture
- Drop Intel macOS build (macos-13 runners retired)
- Use vendored OpenSSL (via git2) for cross-compilation
- Use native runners instead of cross for binary builds

### Features

- Reload command refreshes diffs w/ scroll preservation and adds :clip export (#23)
- Add cross-compiled binary builds to release workflow (#33)

