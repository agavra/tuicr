//! Deterministic identifiers for review changesets.
//!
//! A `Slug` is the agent-facing identity for a review session. It is intended
//! to be human-readable and computable from public information about the
//! repository and the kind of review being performed.
//!
//! Grammar:
//!
//! - Local: `[<owner>/]<repo>@<anchor>/<source>`
//! - PR:    `gh:<owner>/<repo>/pr/<number>`
//!
//! Where `<anchor>` is either a sanitized branch/bookmark name (no `/`) or
//! `~<short-sha>` for detached / anonymous heads, and `<source>` is one of the
//! diff-source variants (`worktree/<head>`, `staged/<head>`,
//! `unstaged/<head>`, `staged-and-unstaged/<head>`, `pristine`,
//! `commits/<base>..<head>`, etc.).
//!
//! The "live" working-tree sources (`worktree`, `staged`, `unstaged`,
//! `staged-and-unstaged`) embed the short SHA of the current HEAD so that a
//! new commit on the same branch produces a fresh session instead of
//! resurrecting stale comments tied to the previous HEAD.
#![allow(dead_code)]

use std::fmt;
use std::path::Path;
use std::str::FromStr;

use git2::Repository;

use crate::forge::traits::{ForgeKind, PrSessionKey};
use crate::model::review::SessionDiffSource;

const SHORT_SHA_LEN: usize = 7;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Slug {
    Local(LocalSlug),
    Pr(PrSlug),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LocalSlug {
    pub owner: Option<String>,
    pub repo: String,
    pub anchor: SlugAnchor,
    pub source: SlugSource,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PrSlug {
    pub forge: ForgeKind,
    pub owner: String,
    pub repo: String,
    pub number: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum SlugAnchor {
    /// Named branch/bookmark. Slashes are sanitized to `-` at construction
    /// time so the anchor segment never contains `/`.
    Branch(String),
    /// Detached / anonymous head. Short SHA or change-id prefix without the
    /// leading `~`.
    Anonymous(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum SlugSource {
    /// Live working-tree diff. Carries the short SHA of HEAD so committing
    /// produces a new slug (and therefore a new persisted session).
    Worktree(String),
    Staged(String),
    Unstaged(String),
    StagedAndUnstaged(String),
    Pristine,
    Commits(CommitRange),
    WorktreeAndCommits(CommitRange),
    StagedUnstagedAndCommits(CommitRange),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CommitRange {
    pub base: String,
    pub head: String,
}

// ----- Display -----

impl fmt::Display for Slug {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Slug::Local(s) => s.fmt(f),
            Slug::Pr(s) => s.fmt(f),
        }
    }
}

impl fmt::Display for LocalSlug {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(owner) = &self.owner {
            write!(f, "{}/{}@{}/{}", owner, self.repo, self.anchor, self.source)
        } else {
            write!(f, "{}@{}/{}", self.repo, self.anchor, self.source)
        }
    }
}

impl fmt::Display for PrSlug {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let kind = match self.forge {
            ForgeKind::GitHub => "gh",
        };
        write!(
            f,
            "{}:{}/{}/pr/{}",
            kind, self.owner, self.repo, self.number
        )
    }
}

impl fmt::Display for SlugAnchor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SlugAnchor::Branch(name) => f.write_str(name),
            SlugAnchor::Anonymous(sha) => write!(f, "~{sha}"),
        }
    }
}

impl fmt::Display for SlugSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SlugSource::Worktree(head) => write!(f, "worktree/{head}"),
            SlugSource::Staged(head) => write!(f, "staged/{head}"),
            SlugSource::Unstaged(head) => write!(f, "unstaged/{head}"),
            SlugSource::StagedAndUnstaged(head) => write!(f, "staged-and-unstaged/{head}"),
            SlugSource::Pristine => f.write_str("pristine"),
            SlugSource::Commits(r) => write!(f, "commits/{}..{}", r.base, r.head),
            SlugSource::WorktreeAndCommits(r) => {
                write!(f, "worktree-and-commits/{}..{}", r.base, r.head)
            }
            SlugSource::StagedUnstagedAndCommits(r) => {
                write!(f, "staged-and-unstaged-and-commits/{}..{}", r.base, r.head)
            }
        }
    }
}

// ----- Parsing -----

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SlugParseError {
    #[error("empty slug")]
    Empty,
    #[error("invalid slug shape: {0}")]
    InvalidShape(String),
    #[error("unknown diff source: {0}")]
    UnknownSource(String),
    #[error("invalid PR number: {0}")]
    InvalidPrNumber(String),
    #[error("unknown forge kind: {0}")]
    UnknownForge(String),
    #[error("missing or malformed commit range")]
    MissingRange,
}

impl FromStr for Slug {
    type Err = SlugParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.is_empty() {
            return Err(SlugParseError::Empty);
        }
        // A PR slug starts with a forge kind followed by `:`. A local slug
        // never contains `:` in its leading segment because `[<owner>/]<repo>`
        // is alphanumeric/dash.
        if let Some((kind, rest)) = s.split_once(':') {
            let forge = match kind {
                "gh" => ForgeKind::GitHub,
                other => return Err(SlugParseError::UnknownForge(other.to_string())),
            };
            return parse_pr(forge, rest).map(Slug::Pr);
        }
        parse_local(s).map(Slug::Local)
    }
}

fn parse_pr(forge: ForgeKind, rest: &str) -> Result<PrSlug, SlugParseError> {
    let parts: Vec<&str> = rest.split('/').collect();
    if parts.len() != 4 || parts[2] != "pr" || parts[0].is_empty() || parts[1].is_empty() {
        return Err(SlugParseError::InvalidShape(rest.to_string()));
    }
    let number: u64 = parts[3]
        .parse()
        .map_err(|_| SlugParseError::InvalidPrNumber(parts[3].to_string()))?;
    Ok(PrSlug {
        forge,
        owner: parts[0].to_string(),
        repo: parts[1].to_string(),
        number,
    })
}

fn parse_local(s: &str) -> Result<LocalSlug, SlugParseError> {
    let (project, rest) = s
        .split_once('@')
        .ok_or_else(|| SlugParseError::InvalidShape(s.to_string()))?;

    let (owner, repo) = if let Some((o, r)) = project.split_once('/') {
        if r.contains('/') || o.is_empty() || r.is_empty() {
            return Err(SlugParseError::InvalidShape(s.to_string()));
        }
        (Some(o.to_string()), r.to_string())
    } else if project.is_empty() {
        return Err(SlugParseError::InvalidShape(s.to_string()));
    } else {
        (None, project.to_string())
    };

    let (anchor_str, source_str) = rest
        .split_once('/')
        .ok_or_else(|| SlugParseError::InvalidShape(s.to_string()))?;

    let anchor = if let Some(sha) = anchor_str.strip_prefix('~') {
        if sha.is_empty() {
            return Err(SlugParseError::InvalidShape(s.to_string()));
        }
        SlugAnchor::Anonymous(sha.to_string())
    } else {
        if anchor_str.is_empty() {
            return Err(SlugParseError::InvalidShape(s.to_string()));
        }
        SlugAnchor::Branch(anchor_str.to_string())
    };

    let source = parse_source(source_str)?;

    Ok(LocalSlug {
        owner,
        repo,
        anchor,
        source,
    })
}

fn parse_source(s: &str) -> Result<SlugSource, SlugParseError> {
    // The compound `*-and-commits/<range>` sources must be matched before the
    // simple `worktree/<head>` / `staged-and-unstaged/<head>` variants because
    // their prefixes overlap (e.g. `staged-and-unstaged-and-commits/` starts
    // with `staged-and-unstaged/`).
    if let Some(range) = s.strip_prefix("commits/") {
        return Ok(SlugSource::Commits(parse_range(range)?));
    }
    if let Some(range) = s.strip_prefix("worktree-and-commits/") {
        return Ok(SlugSource::WorktreeAndCommits(parse_range(range)?));
    }
    if let Some(range) = s.strip_prefix("staged-and-unstaged-and-commits/") {
        return Ok(SlugSource::StagedUnstagedAndCommits(parse_range(range)?));
    }
    if s == "pristine" {
        return Ok(SlugSource::Pristine);
    }
    if let Some(head) = s.strip_prefix("worktree/") {
        return live_source(head, s, SlugSource::Worktree);
    }
    if let Some(head) = s.strip_prefix("staged-and-unstaged/") {
        return live_source(head, s, SlugSource::StagedAndUnstaged);
    }
    if let Some(head) = s.strip_prefix("staged/") {
        return live_source(head, s, SlugSource::Staged);
    }
    if let Some(head) = s.strip_prefix("unstaged/") {
        return live_source(head, s, SlugSource::Unstaged);
    }
    Err(SlugParseError::UnknownSource(s.to_string()))
}

fn live_source(
    head: &str,
    full: &str,
    ctor: fn(String) -> SlugSource,
) -> Result<SlugSource, SlugParseError> {
    if head.is_empty() || head.contains('/') {
        return Err(SlugParseError::UnknownSource(full.to_string()));
    }
    Ok(ctor(head.to_string()))
}

fn parse_range(s: &str) -> Result<CommitRange, SlugParseError> {
    let (base, head) = s.split_once("..").ok_or(SlugParseError::MissingRange)?;
    if base.is_empty() || head.is_empty() {
        return Err(SlugParseError::MissingRange);
    }
    Ok(CommitRange {
        base: base.to_string(),
        head: head.to_string(),
    })
}

// ----- Helpers -----

/// Sanitize a branch/bookmark name for use as a slug anchor segment. Replaces
/// `/` with `-`. Lossy: `feature/login` and `feature-login` collapse to the
/// same anchor; in practice collisions are rare.
pub fn sanitize_ref(name: &str) -> String {
    name.replace('/', "-")
}

/// Take the first `SHORT_SHA_LEN` chars of a SHA. Shorter inputs pass through.
pub fn short_sha(sha: &str) -> String {
    sha.chars().take(SHORT_SHA_LEN).collect()
}

// ----- Derivation from a PR session key -----

impl From<&PrSessionKey> for Slug {
    fn from(key: &PrSessionKey) -> Self {
        Slug::Pr(PrSlug {
            forge: key.repository.kind,
            owner: key.repository.owner.clone(),
            repo: key.repository.name.clone(),
            number: key.number,
        })
    }
}

// ----- Derivation from local session inputs -----

#[derive(Debug, thiserror::Error)]
pub enum SlugDeriveError {
    #[error("cannot determine repo name from path {0}")]
    NoRepoName(String),
    #[error("commit range required for diff source {0:?} but missing")]
    MissingCommitRange(SessionDiffSource),
    #[error("PullRequest diff source has no local slug")]
    PullRequestNotLocal,
}

/// Resolve the `(owner, repo)` pair for a local repo from its remote `origin`
/// URL. Falls back to the directory name (no owner) when the origin URL is
/// missing or unparseable.
pub fn resolve_owner_repo(repo_path: &Path) -> Result<(Option<String>, String), SlugDeriveError> {
    if let Ok(repo) = Repository::discover(repo_path)
        && let Some((owner, name)) = origin_owner_repo(&repo)
    {
        return Ok((Some(owner), name));
    }
    let name = repo_path
        .file_name()
        .and_then(|n| n.to_str())
        .map(|s| s.to_string())
        .ok_or_else(|| SlugDeriveError::NoRepoName(repo_path.display().to_string()))?;
    Ok((None, name))
}

fn origin_owner_repo(repo: &Repository) -> Option<(String, String)> {
    let remote = repo.find_remote("origin").ok()?;
    let url = remote.url()?;
    parse_remote_owner_repo(url)
}

/// Forge-agnostic remote-URL parser. Handles HTTPS, SCP-style SSH
/// (`git@host:path`), and SSH scheme URLs. Always takes the last two path
/// segments as `owner/repo` so nested groupings (GitLab subgroups, etc.)
/// degrade gracefully. Strips a trailing `.git`.
fn parse_remote_owner_repo(remote_url: &str) -> Option<(String, String)> {
    let url = remote_url.trim();
    if let Some(rest) = url.strip_prefix("https://") {
        parse_path_segments(rest)
    } else if let Some(rest) = url.strip_prefix("http://") {
        parse_path_segments(rest)
    } else if let Some(rest) = url.strip_prefix("ssh://") {
        parse_path_segments(rest)
    } else if let Some((_, rest)) = url.split_once('@') {
        let normalized = rest.replacen(':', "/", 1);
        parse_path_segments(&normalized)
    } else {
        parse_path_segments(url)
    }
}

fn parse_path_segments(s: &str) -> Option<(String, String)> {
    let (_, path) = s.split_once('/')?;
    let mut segments: Vec<&str> = path
        .trim_matches('/')
        .split('/')
        .filter(|seg| !seg.is_empty())
        .collect();
    let repo_seg = segments.pop()?;
    let owner_seg = segments.pop()?;
    if owner_seg.is_empty() || repo_seg.is_empty() {
        return None;
    }
    let repo = repo_seg.strip_suffix(".git").unwrap_or(repo_seg);
    Some((owner_seg.to_string(), repo.to_string()))
}

/// Build a `LocalSlug` from session inputs. The `owner_repo` pair is taken
/// pre-resolved (see [`resolve_owner_repo`]) so that the build step itself is
/// pure and testable without git I/O.
pub fn build_local_slug(
    owner_repo: (Option<String>, String),
    branch_name: Option<&str>,
    head_commit: &str,
    diff_source: SessionDiffSource,
    commit_range: Option<&[String]>,
) -> Result<LocalSlug, SlugDeriveError> {
    let (owner, repo) = owner_repo;
    let anchor = match branch_name {
        Some(name) => SlugAnchor::Branch(sanitize_ref(name)),
        None => SlugAnchor::Anonymous(short_sha(head_commit)),
    };
    let source = build_source(diff_source, head_commit, commit_range)?;
    Ok(LocalSlug {
        owner,
        repo,
        anchor,
        source,
    })
}

fn build_source(
    diff_source: SessionDiffSource,
    head_commit: &str,
    commit_range: Option<&[String]>,
) -> Result<SlugSource, SlugDeriveError> {
    let live_head = || live_head_token(head_commit);
    match diff_source {
        SessionDiffSource::WorkingTree => Ok(SlugSource::Worktree(live_head())),
        SessionDiffSource::Staged => Ok(SlugSource::Staged(live_head())),
        SessionDiffSource::Unstaged => Ok(SlugSource::Unstaged(live_head())),
        SessionDiffSource::StagedAndUnstaged => Ok(SlugSource::StagedAndUnstaged(live_head())),
        SessionDiffSource::Pristine => Ok(SlugSource::Pristine),
        SessionDiffSource::CommitRange => {
            Ok(SlugSource::Commits(range_from(commit_range, diff_source)?))
        }
        SessionDiffSource::WorkingTreeAndCommits => Ok(SlugSource::WorktreeAndCommits(range_from(
            commit_range,
            diff_source,
        )?)),
        SessionDiffSource::StagedUnstagedAndCommits => Ok(SlugSource::StagedUnstagedAndCommits(
            range_from(commit_range, diff_source)?,
        )),
        SessionDiffSource::PullRequest => Err(SlugDeriveError::PullRequestNotLocal),
    }
}

/// Token used in the slug source segment to identify HEAD for live diff
/// sources. Short SHA when available; `none` for an unborn HEAD (empty
/// commit) so the slug remains well-formed.
fn live_head_token(head_commit: &str) -> String {
    let short = short_sha(head_commit);
    if short.is_empty() {
        "none".to_string()
    } else {
        short
    }
}

fn range_from(
    commit_range: Option<&[String]>,
    diff_source: SessionDiffSource,
) -> Result<CommitRange, SlugDeriveError> {
    let range = commit_range.ok_or(SlugDeriveError::MissingCommitRange(diff_source))?;
    if range.is_empty() {
        return Err(SlugDeriveError::MissingCommitRange(diff_source));
    }
    // `commit_range` is stored newest-first by the App layer.
    let head = short_sha(&range[0]);
    let base = short_sha(&range[range.len() - 1]);
    Ok(CommitRange { base, head })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::forge::traits::ForgeRepository;

    // ---------- Display ----------

    #[test]
    fn should_render_local_slug_with_owner() {
        let slug = LocalSlug {
            owner: Some("agavra".to_string()),
            repo: "tuicr".to_string(),
            anchor: SlugAnchor::Branch("main".to_string()),
            source: SlugSource::Worktree("abc1234".to_string()),
        };
        assert_eq!(slug.to_string(), "agavra/tuicr@main/worktree/abc1234");
    }

    #[test]
    fn should_render_local_slug_without_owner() {
        let slug = LocalSlug {
            owner: None,
            repo: "tuicr".to_string(),
            anchor: SlugAnchor::Branch("main".to_string()),
            source: SlugSource::Worktree("abc1234".to_string()),
        };
        assert_eq!(slug.to_string(), "tuicr@main/worktree/abc1234");
    }

    #[test]
    fn should_render_anonymous_anchor() {
        let slug = LocalSlug {
            owner: Some("agavra".to_string()),
            repo: "tuicr".to_string(),
            anchor: SlugAnchor::Anonymous("abc1234".to_string()),
            source: SlugSource::Worktree("abc1234".to_string()),
        };
        assert_eq!(slug.to_string(), "agavra/tuicr@~abc1234/worktree/abc1234");
    }

    #[test]
    fn should_render_commits_source() {
        let slug = LocalSlug {
            owner: Some("agavra".to_string()),
            repo: "tuicr".to_string(),
            anchor: SlugAnchor::Branch("main".to_string()),
            source: SlugSource::Commits(CommitRange {
                base: "abc1234".to_string(),
                head: "def5678".to_string(),
            }),
        };
        assert_eq!(
            slug.to_string(),
            "agavra/tuicr@main/commits/abc1234..def5678"
        );
    }

    #[test]
    fn should_render_pr_slug_github() {
        let slug = PrSlug {
            forge: ForgeKind::GitHub,
            owner: "agavra".to_string(),
            repo: "tuicr".to_string(),
            number: 125,
        };
        assert_eq!(slug.to_string(), "gh:agavra/tuicr/pr/125");
    }

    // ---------- Round-trip ----------

    fn assert_roundtrip(s: &str) {
        let parsed: Slug = s.parse().expect(s);
        assert_eq!(parsed.to_string(), s, "round-trip failed for {s}");
    }

    #[test]
    fn should_roundtrip_local_slug_variants() {
        assert_roundtrip("agavra/tuicr@main/worktree/abc1234");
        assert_roundtrip("agavra/tuicr@main/staged/abc1234");
        assert_roundtrip("agavra/tuicr@main/unstaged/abc1234");
        assert_roundtrip("agavra/tuicr@feature-login/staged-and-unstaged/abc1234");
        assert_roundtrip("agavra/tuicr@main/pristine");
        assert_roundtrip("agavra/tuicr@~abc1234/worktree/abc1234");
        assert_roundtrip("agavra/tuicr@main/commits/abc1234..def5678");
        assert_roundtrip("agavra/tuicr@main/worktree-and-commits/abc1234..def5678");
        assert_roundtrip("agavra/tuicr@main/staged-and-unstaged-and-commits/abc1234..def5678");
        assert_roundtrip("tuicr@main/worktree/abc1234");
    }

    #[test]
    fn should_roundtrip_pr_slug() {
        assert_roundtrip("gh:agavra/tuicr/pr/125");
        assert_roundtrip("gh:org/svc/pr/9999");
    }

    // ---------- Parse errors ----------

    #[test]
    fn should_reject_empty_slug() {
        assert_eq!("".parse::<Slug>().unwrap_err(), SlugParseError::Empty);
    }

    #[test]
    fn should_reject_pr_slug_with_non_numeric_number() {
        let err = "gh:agavra/tuicr/pr/notanumber".parse::<Slug>().unwrap_err();
        assert!(matches!(err, SlugParseError::InvalidPrNumber(_)));
    }

    #[test]
    fn should_reject_unknown_forge_prefix() {
        let err = "xy:agavra/tuicr/pr/1".parse::<Slug>().unwrap_err();
        assert!(matches!(err, SlugParseError::UnknownForge(_)));
    }

    #[test]
    fn should_reject_pr_slug_missing_pr_keyword() {
        assert!("gh:agavra/tuicr/notpr/1".parse::<Slug>().is_err());
    }

    #[test]
    fn should_reject_local_slug_without_at_separator() {
        assert!("agavra/tuicr/worktree/abc1234".parse::<Slug>().is_err());
    }

    #[test]
    fn should_reject_unknown_diff_source() {
        let err = "agavra/tuicr@main/blarghhh".parse::<Slug>().unwrap_err();
        assert!(matches!(err, SlugParseError::UnknownSource(_)));
    }

    #[test]
    fn should_reject_empty_anchor() {
        assert!("agavra/tuicr@/worktree/abc1234".parse::<Slug>().is_err());
    }

    #[test]
    fn should_reject_empty_anonymous_anchor() {
        assert!("agavra/tuicr@~/worktree/abc1234".parse::<Slug>().is_err());
    }

    #[test]
    fn should_reject_live_source_without_head() {
        // Bare `worktree` (no head segment) is no longer a valid slug — every
        // live diff source must encode the HEAD it was opened against.
        assert!("agavra/tuicr@main/worktree".parse::<Slug>().is_err());
        assert!("agavra/tuicr@main/staged".parse::<Slug>().is_err());
        assert!("agavra/tuicr@main/unstaged".parse::<Slug>().is_err());
        assert!(
            "agavra/tuicr@main/staged-and-unstaged"
                .parse::<Slug>()
                .is_err()
        );
        assert!("agavra/tuicr@main/worktree/".parse::<Slug>().is_err());
    }

    #[test]
    fn should_reject_commits_source_missing_range() {
        assert!("agavra/tuicr@main/commits/".parse::<Slug>().is_err());
        assert!("agavra/tuicr@main/commits/abc1234".parse::<Slug>().is_err());
        assert!(
            "agavra/tuicr@main/commits/..abc1234"
                .parse::<Slug>()
                .is_err()
        );
    }

    // ---------- Sanitization ----------

    #[test]
    fn should_replace_slashes_in_branch_with_dashes() {
        assert_eq!(sanitize_ref("feature/login"), "feature-login");
        assert_eq!(sanitize_ref("dependabot/foo/bar"), "dependabot-foo-bar");
        assert_eq!(sanitize_ref("main"), "main");
    }

    #[test]
    fn should_truncate_to_short_sha_length() {
        assert_eq!(short_sha("abcdef1234567890"), "abcdef1");
        assert_eq!(short_sha("abc"), "abc");
    }

    // ---------- From PrSessionKey ----------

    #[test]
    fn should_derive_slug_from_pr_session_key() {
        let key = PrSessionKey::new(
            ForgeRepository::github("github.com", "agavra", "tuicr"),
            125,
            "abcdef0123456789".to_string(),
        );
        let slug: Slug = (&key).into();
        assert_eq!(slug.to_string(), "gh:agavra/tuicr/pr/125");
    }

    // ---------- Remote URL parser ----------

    #[test]
    fn should_parse_https_remote_url() {
        assert_eq!(
            parse_remote_owner_repo("https://github.com/agavra/tuicr.git"),
            Some(("agavra".to_string(), "tuicr".to_string()))
        );
    }

    #[test]
    fn should_parse_https_remote_url_without_dot_git_suffix() {
        assert_eq!(
            parse_remote_owner_repo("https://github.com/agavra/tuicr"),
            Some(("agavra".to_string(), "tuicr".to_string()))
        );
    }

    #[test]
    fn should_parse_scp_like_ssh_remote_url() {
        assert_eq!(
            parse_remote_owner_repo("git@github.com:agavra/tuicr.git"),
            Some(("agavra".to_string(), "tuicr".to_string()))
        );
    }

    #[test]
    fn should_parse_ssh_scheme_remote_url() {
        assert_eq!(
            parse_remote_owner_repo("ssh://git@github.com/agavra/tuicr.git"),
            Some(("agavra".to_string(), "tuicr".to_string()))
        );
    }

    #[test]
    fn should_pick_last_two_segments_for_nested_subgroups() {
        // GitLab subgroups: take last two segments as owner/repo.
        assert_eq!(
            parse_remote_owner_repo("git@gitlab.com:org/team/svc.git"),
            Some(("team".to_string(), "svc".to_string()))
        );
    }

    #[test]
    fn should_return_none_for_unparseable_remote() {
        assert_eq!(parse_remote_owner_repo("not-a-url"), None);
        assert_eq!(parse_remote_owner_repo(""), None);
    }

    // ---------- build_local_slug ----------

    #[test]
    fn should_build_worktree_slug_from_branch() {
        let slug = build_local_slug(
            (Some("agavra".to_string()), "tuicr".to_string()),
            Some("main"),
            "abcdef0123",
            SessionDiffSource::WorkingTree,
            None,
        )
        .unwrap();
        assert_eq!(slug.to_string(), "agavra/tuicr@main/worktree/abcdef0");
    }

    #[test]
    fn should_sanitize_branch_at_build_time() {
        let slug = build_local_slug(
            (Some("agavra".to_string()), "tuicr".to_string()),
            Some("feature/login"),
            "abcdef0123",
            SessionDiffSource::WorkingTree,
            None,
        )
        .unwrap();
        assert_eq!(
            slug.to_string(),
            "agavra/tuicr@feature-login/worktree/abcdef0"
        );
    }

    #[test]
    fn should_use_anonymous_anchor_when_branch_is_none() {
        let slug = build_local_slug(
            (Some("agavra".to_string()), "tuicr".to_string()),
            None,
            "abcdef0123456789",
            SessionDiffSource::WorkingTree,
            None,
        )
        .unwrap();
        assert_eq!(slug.to_string(), "agavra/tuicr@~abcdef0/worktree/abcdef0");
    }

    #[test]
    fn should_change_slug_when_head_advances_for_live_source() {
        // Regression for #378: two `tuicr -w` runs on the same branch but
        // different HEADs must produce distinct slugs so the persisted
        // session from the previous HEAD does not leak its comments into
        // the new run.
        let before = build_local_slug(
            (Some("agavra".to_string()), "tuicr".to_string()),
            Some("main"),
            "abcdef0123",
            SessionDiffSource::WorkingTree,
            None,
        )
        .unwrap();
        let after = build_local_slug(
            (Some("agavra".to_string()), "tuicr".to_string()),
            Some("main"),
            "9999999aaa",
            SessionDiffSource::WorkingTree,
            None,
        )
        .unwrap();
        assert_ne!(before.to_string(), after.to_string());
        assert_eq!(before.to_string(), "agavra/tuicr@main/worktree/abcdef0");
        assert_eq!(after.to_string(), "agavra/tuicr@main/worktree/9999999");
    }

    #[test]
    fn should_use_none_token_for_unborn_head_in_live_source() {
        let slug = build_local_slug(
            (None, "tuicr".to_string()),
            Some("main"),
            "",
            SessionDiffSource::WorkingTree,
            None,
        )
        .unwrap();
        assert_eq!(slug.to_string(), "tuicr@main/worktree/none");
    }

    #[test]
    fn should_build_commits_slug_from_range() {
        let range = vec![
            "def5678aaa".to_string(),
            "intermediate".to_string(),
            "abc1234bbb".to_string(),
        ];
        let slug = build_local_slug(
            (Some("agavra".to_string()), "tuicr".to_string()),
            Some("main"),
            "def5678",
            SessionDiffSource::CommitRange,
            Some(&range),
        )
        .unwrap();
        // commit_range is newest-first: head = first, base = last (short SHAs)
        assert_eq!(
            slug.to_string(),
            "agavra/tuicr@main/commits/abc1234..def5678"
        );
    }

    #[test]
    fn should_reject_build_for_commit_range_without_range() {
        let err = build_local_slug(
            (Some("agavra".to_string()), "tuicr".to_string()),
            Some("main"),
            "def5678",
            SessionDiffSource::CommitRange,
            None,
        )
        .unwrap_err();
        assert!(matches!(err, SlugDeriveError::MissingCommitRange(_)));
    }

    #[test]
    fn should_reject_build_for_pull_request_diff_source() {
        let err = build_local_slug(
            (Some("agavra".to_string()), "tuicr".to_string()),
            Some("main"),
            "def5678",
            SessionDiffSource::PullRequest,
            None,
        )
        .unwrap_err();
        assert!(matches!(err, SlugDeriveError::PullRequestNotLocal));
    }
}
