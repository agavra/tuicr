#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'USAGE' >&2
Usage: scripts/demo/setup-fixture.sh [OUTPUT_DIR]

Creates a fresh git repository containing a tiny Rust crate with one base
commit and two review commits for the README demo recording. The repository
path is printed on stdout.

Set TUICR_DEMO_SKIP_TEST=1 to skip the fixture cargo test validation.
USAGE
}

if [[ "${1:-}" == "-h" || "${1:-}" == "--help" ]]; then
  usage
  exit 0
fi

if [[ $# -gt 1 ]]; then
  usage
  exit 2
fi

if [[ $# -eq 1 ]]; then
  repo_dir="$1"
  if [[ -e "$repo_dir" ]]; then
    echo "error: output directory already exists: $repo_dir" >&2
    exit 1
  fi
  mkdir -p "$repo_dir"
else
  repo_dir="$(mktemp -d "${TMPDIR:-/tmp}/tuicr-demo-fixture.XXXXXX")"
fi

require() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "error: missing required command: $1" >&2
    exit 1
  fi
}

commit_with_date() {
  local date="$1"
  local message="$2"
  GIT_AUTHOR_DATE="$date" GIT_COMMITTER_DATE="$date" git commit -q -m "$message"
}

require git
require cargo
require python3

cd "$repo_dir"
git init -q
git symbolic-ref HEAD refs/heads/main
git config user.name "tuicr demo"
git config user.email "demo@tuicr.dev"

mkdir -p src

cat > Cargo.toml <<'EOF'
[package]
name = "tuicr-demo-fixture"
version = "0.1.0"
edition = "2024"

[lib]
path = "src/lib.rs"
EOF

cat > .gitignore <<'EOF'
/target/
EOF

cat > src/lib.rs <<'EOF'
pub mod auth;
EOF

cat > src/auth.rs <<'EOF'
//! Authentication helpers used by the tuicr-demo crate.
//!
//! This module models a tiny session store so review demos have a realistic
//! diff to look at. None of the cryptography is real.

use std::time::Duration;

/// Configuration for token issuance and refresh behavior.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionConfig {
    /// Identifier embedded in every token this issuer produces.
    pub issuer: String,
    /// How long a freshly minted token remains valid.
    pub token_ttl: Duration,
    /// Window before expiry where proactive refresh kicks in.
    pub refresh_skew: Duration,
    /// Cap on consecutive refresh failures before forcing reauth.
    pub max_refresh_attempts: u32,
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            issuer: "tuicr-demo".to_string(),
            token_ttl: Duration::from_secs(3_600),
            refresh_skew: Duration::from_secs(30),
            max_refresh_attempts: 5,
        }
    }
}

/// Effective lifetime of a token before refresh is required.
pub fn token_expires_after(config: &SessionConfig) -> Duration {
    config.token_ttl.saturating_sub(config.refresh_skew)
}

/// Tracks a single client's session state in memory.
#[derive(Debug, Clone)]
pub struct Session {
    pub config: SessionConfig,
    pub failed_attempts: u32,
}

impl Session {
    pub fn new(config: SessionConfig) -> Self {
        Self {
            config,
            failed_attempts: 0,
        }
    }

    pub fn record_failure(&mut self) {
        self.failed_attempts = self.failed_attempts.saturating_add(1);
    }

    pub fn reset_failures(&mut self) {
        self.failed_attempts = 0;
    }

    pub fn is_locked_out(&self) -> bool {
        self.failed_attempts >= self.config.max_refresh_attempts
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_expiry_leaves_refresh_skew() {
        let config = SessionConfig::default();
        assert_eq!(token_expires_after(&config), Duration::from_secs(3_570));
    }

    #[test]
    fn lockout_after_max_attempts() {
        let mut session = Session::new(SessionConfig::default());
        for _ in 0..5 {
            session.record_failure();
        }
        assert!(session.is_locked_out());
    }
}
EOF

cargo generate-lockfile -q

git add .
commit_with_date "2026-05-01T09:00:00Z" "Create review fixture crate"

python3 - <<'PY'
from pathlib import Path

path = Path("src/auth.rs")
old = path.read_text()
new = """//! Authentication helpers used by the tuicr-demo crate.
//!
//! This module models a tiny session store so review demos have a realistic
//! diff to look at. None of the cryptography is real.

use std::time::Duration;

/// Configuration for token issuance and refresh behavior.
///
/// Defaults favor aggressive expiry so flaky upstreams roll over fast.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionConfig {
    /// Identifier embedded in every token this issuer produces.
    pub issuer: String,
    /// How long a freshly minted token remains valid.
    pub token_ttl: Duration,
    /// Window before expiry where proactive refresh kicks in.
    pub refresh_skew: Duration,
    /// Cap on consecutive refresh failures before forcing reauth.
    pub max_refresh_attempts: u32,
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            issuer: "tuicr-demo".to_string(),
            token_ttl: Duration::from_secs(15),
            refresh_skew: Duration::from_secs(5),
            max_refresh_attempts: 3,
        }
    }
}

/// Effective lifetime of a token before refresh is required.
pub fn token_expires_after(config: &SessionConfig) -> Duration {
    config.token_ttl.saturating_sub(config.refresh_skew)
}

/// Tracks a single client's session state in memory.
#[derive(Debug, Clone)]
pub struct Session {
    pub config: SessionConfig,
    pub failed_attempts: u32,
}

impl Session {
    pub fn new(config: SessionConfig) -> Self {
        Self {
            config,
            failed_attempts: 0,
        }
    }

    pub fn record_failure(&mut self) {
        self.failed_attempts = self.failed_attempts.saturating_add(1);
    }

    pub fn reset_failures(&mut self) {
        self.failed_attempts = 0;
    }

    pub fn is_locked_out(&self) -> bool {
        self.failed_attempts >= self.config.max_refresh_attempts
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_expiry_leaves_refresh_skew() {
        let config = SessionConfig::default();
        assert_eq!(token_expires_after(&config), Duration::from_secs(10));
    }

    #[test]
    fn lockout_after_max_attempts() {
        let mut session = Session::new(SessionConfig::default());
        for _ in 0..3 {
            session.record_failure();
        }
        assert!(session.is_locked_out());
    }
}
"""
path.write_text(new)
PY

git add src/auth.rs
commit_with_date "2026-05-01T09:05:00Z" "Shorten session token timeout"

python3 - <<'PY'
from pathlib import Path

path = Path("src/auth.rs")
text = path.read_text()
insert = """
/// Returns true when the HTTP status code warrants another attempt.
///
/// Errors that surface as 408 (request timeout) and 429 (rate limit) are
/// transient, as are anything in the 5xx range. Everything else should fail
/// fast so we surface real bugs upstream.
pub fn should_retry_status(status: u16) -> bool {
    matches!(status, 408 | 429 | 500..=599)
}

/// Suggests a backoff duration for the given attempt number.
pub fn retry_delay_for_attempt(attempt: u32) -> Duration {
    let base = Duration::from_millis(250);
    base.saturating_mul(1u32 << attempt.min(5))
}
"""
text = text.replace("\n#[cfg(test)]\n", f"{insert}\n#[cfg(test)]\n")

extra_tests = """
    #[test]
    fn retry_status_includes_429_and_5xx() {
        assert!(should_retry_status(408));
        assert!(should_retry_status(429));
        assert!(should_retry_status(503));
        assert!(!should_retry_status(404));
    }

    #[test]
    fn retry_delay_grows_with_attempt() {
        let first = retry_delay_for_attempt(0);
        let third = retry_delay_for_attempt(3);
        assert!(third > first);
    }
"""
text = text.rstrip()
if text.endswith("}"):
    text = text[:-1].rstrip() + extra_tests + "}\n"
path.write_text(text)
PY

git add src/auth.rs
commit_with_date "2026-05-01T09:10:00Z" "Handle rate-limit retry responses"

if [[ "${TUICR_DEMO_SKIP_TEST:-0}" != "1" ]]; then
  cargo test -q >&2
fi

if [[ -n "$(git status --porcelain)" ]]; then
  echo "error: generated fixture is dirty after validation" >&2
  git status --short >&2
  exit 1
fi

printf '%s\n' "$repo_dir"
