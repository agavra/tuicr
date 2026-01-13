# Changelog

All notable changes to this project will be documented in this file.

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

