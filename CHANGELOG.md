# Changelog

All notable changes to this project will be documented in this file.

## [0.1.2] - 2026-01-10

### Documentation

- Add Homebrew installation and tap update instructions

### Features

- Support modifying comments (#37)
- Add commit selection when no unstaged changes (#38)
- Add syntax highlighting (#41)
  
## [0.1.1] - 2026-01-09

### Bug Fixes

- Use native macOS runners for each architecture
- Drop Intel macOS build (macos-13 runners retired)
- Use vendored OpenSSL (via git2) for cross-compilation
- Use native runners instead of cross for binary builds

### Features

- Reload command refreshes diffs w/ scroll preservation and adds :clip export (#23)
- Add cross-compiled binary builds to release workflow (#33)

