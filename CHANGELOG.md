# Changelog

All notable changes to this project will be documented in this file.

## [0.9.0] - 2026-03-24

### Bug Fixes

- Append newline to lines passed to syntect parser for correct scope matching (#202)
- **diff-parser:** Handle empty files and mode-only changes in git-style diffs (#215)
- **input:** Support Shift+Tab reverse cycling (#213)

### Documentation

- Add {N}G jump-to-line shortcut to README and AGENTS.md (#216)

### Features

- **skill:** Improve skill integration with other agents (#201)
- **config:** Customizable comment types with labels, colors and definitions (#211)
- Add --version flag (#212)
- **config:** Add show_file_list, diff_view, and wrap config options (#218)
- Add Nord theme (#219)
- Add staged and unstaged review options (#183)
## [0.8.0] - 2026-03-11

### Bug Fixes

- **ui:** Make diff row backgrounds consistent to eol (#180)
- **diff:** Normalize tabs across parsers and add coverage (#179)
- **ui:** Shift focus to diff when file list is collapsed (#185)
- Remove nix result symlink that breaks cargo publish

### Features

- **theme:** Add gruvbox-dark, gruvbox-light themes (#181)
- Show commit message as reviewable entry for single-commit reviews (#182)
- Add {N}G shortcut to jump to source line in diff view (#193)
- **theme:** Add ayu-light and onedark themes (#195)
- **theme:** Add appearance mode and split dark/light config variants (#196)
- **comments:** Add review-level comments across review scope (#197)
## [0.7.2] - 2026-02-12

### Bug Fixes

- Skip large untracked files to prevent startup hang (#177)
- Prefer OSC 52 clipboard in Zellij sessions (#176)
## [0.7.0] - 2026-02-10

### Bug Fixes

- **diff:** Expand collapsed lines in side-by-side mode (#156)
- **config:** Ignore unknown keys while preserving known settings (#166)

### Features

- **syntax:** Add syntax highlighting for diffs (#154)
- **syntax:** Replace syntect defaults with two-face for expanded syntax highlighting (#155)
- Add inline commit selector for multi-commit reviews (#160)
- Allow selecting both worktree and commits in the selector (#161)
- Add configuration file support and catppuccin themes (#162)
## [0.6.0] - 2026-01-30

### Bug Fixes

- **ui:** Render comment input inline instead of as overlay (#137)
- **jj:** Show closest bookmark instead of 'detached' in UI (#144)
- **input:** Handle multi-byte UTF-8 characters in comment input (#132) (#147)

### Documentation

- **ui:** Update help and docs for search, commands, and stdout export (#148)

### Features

- **cli:** Add --stdout flag to output export to stdout (#142)
- **skill:** Add Claude Code skill for interactive review (#143)
- **app:** Support expandable commit list and adjust default commit loading (#138)
- **update:** Check crates.io for new releases and surface update status in UI (#150)
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

