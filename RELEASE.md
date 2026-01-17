# Release Process

This project uses an automated release workflow via GitHub Actions.

## How to Release

1. **Go to Actions** → **Release** → **Run workflow**
2. **Select action:**
   - `bump-patch` (0.1.0 → 0.1.1) - bug fixes
   - `bump-minor` (0.1.0 → 0.2.0) - new features
   - `bump-major` (0.1.0 → 1.0.0) - breaking changes
3. **Click "Run workflow"**
4. **Review and merge** the auto-created PR
5. **Done!** Merging automatically:
   - Creates git tag `vX.Y.Z`
   - Publishes to crates.io
   - Creates GitHub Release with release notes
   - Builds and uploads binaries for:
     - `x86_64-unknown-linux-gnu` (Linux x64)
     - `aarch64-unknown-linux-gnu` (Linux ARM64)
     - `aarch64-apple-darwin` (macOS Apple Silicon)
     - `x86_64-pc-windows-msvc` (Windows x64)
6. **Update Homebrew tap** (manual, see below)

## Build Binaries for Existing Release

If you need to rebuild binaries for an existing release:

1. **Go to Actions** → **Release** → **Run workflow**
2. **Select action:** `build-binaries`
3. **Enter version:** e.g., `0.1.1` (without the `v` prefix)
4. **Click "Run workflow"**

## What Gets Updated

The release PR includes:
- `Cargo.toml` - version bump
- `Cargo.lock` - updated lockfile
- `CHANGELOG.md` - auto-generated from commits

## Commit Message Convention

For meaningful changelogs, use conventional commits:

| Prefix | Category | Example |
|--------|----------|---------|
| `feat:` | Features | `feat: add export to JSON` |
| `fix:` | Bug Fixes | `fix: resolve crash on empty diff` |
| `docs:` | Documentation | `docs: update keybindings table` |
| `perf:` | Performance | `perf: optimize large file rendering` |
| `refactor:` | Refactor | `refactor: simplify state machine` |
| `test:` | Testing | `test: add integration tests` |
| `chore:` | Miscellaneous | `chore: update dependencies` |

## Update Homebrew Tap

After binaries are uploaded, update the Homebrew formula:

```bash
# Get SHA256 checksums for the new version
VERSION=X.Y.Z
curl -sL "https://github.com/agavra/tuicr/releases/download/v${VERSION}/tuicr-${VERSION}-aarch64-apple-darwin.tar.gz" | shasum -a 256
curl -sL "https://github.com/agavra/tuicr/releases/download/v${VERSION}/tuicr-${VERSION}-x86_64-unknown-linux-gnu.tar.gz" | shasum -a 256
curl -sL "https://github.com/agavra/tuicr/releases/download/v${VERSION}/tuicr-${VERSION}-aarch64-unknown-linux-gnu.tar.gz" | shasum -a 256

# Update homebrew-tap/Formula/tuicr.rb with new version and checksums
# Then commit and push to homebrew-tap repo
```

## Required Secrets

The following secrets must be configured in GitHub repository settings:

- `CARGO_REGISTRY_TOKEN` - API token from https://crates.io/settings/tokens

## Manual Release (if needed)

```bash
# Update version in Cargo.toml manually, then:
cargo publish --dry-run  # verify
cargo publish            # publish to crates.io
git tag v0.2.0
git push origin v0.2.0
```
