//! JSON deserialization for `bkt` Bitbucket CLI output.

use chrono::{DateTime, Utc};
use serde::Deserialize;

use crate::error::{Result, TuicrError};

// ── bkt pr list ──────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct BktPrListResponse {
    #[serde(default)]
    pub pull_requests: Vec<BktPullRequest>,
}

// ── bkt pr view ──────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct BktPrView {
    #[serde(default)]
    pub pull_request: Option<BktPullRequest>,
}

#[derive(Debug, Deserialize)]
pub struct BktPullRequest {
    pub id: u64,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub draft: bool,
    #[serde(default)]
    pub created_on: Option<DateTime<Utc>>,
    #[serde(default)]
    pub updated_on: Option<DateTime<Utc>>,
    #[serde(default)]
    pub author: Option<BktAuthor>,
    #[serde(default)]
    pub source: Option<BktSourceRef>,
    #[serde(default)]
    pub destination: Option<BktDestinationRef>,
    #[serde(default)]
    pub links: Option<BktLinks>,
    #[serde(default)]
    pub summary: Option<BktSummary>,
}

#[derive(Debug, Deserialize)]
pub struct BktAuthor {
    #[serde(default)]
    pub display_name: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct BktSourceRef {
    #[serde(default)]
    pub branch: Option<BktBranchName>,
    #[serde(default)]
    pub commit: Option<BktCommitRef>,
    #[serde(default)]
    pub repository: Option<BktRepository>,
}

#[derive(Debug, Deserialize)]
pub struct BktDestinationRef {
    #[serde(default)]
    pub branch: Option<BktBranchName>,
    #[serde(default)]
    pub repository: Option<BktRepository>,
}

#[derive(Debug, Deserialize)]
pub struct BktBranchName {
    #[serde(default)]
    pub name: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct BktCommitRef {
    #[serde(default)]
    pub hash: String,
}

#[derive(Debug, Deserialize)]
pub struct BktRepository {
    #[serde(default)]
    pub full_name: String,
    #[serde(default)]
    pub links: Option<BktLinks>,
}

#[derive(Debug, Deserialize)]
pub struct BktSummary {
    #[serde(default)]
    pub raw: String,
}

#[derive(Debug, Deserialize)]
pub struct BktLinks {
    #[serde(default)]
    pub html: Option<BktHtmlLink>,
}

#[derive(Debug, Deserialize)]
pub struct BktHtmlLink {
    #[serde(default)]
    pub href: String,
}

// ── bkt pr comments ─────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct BktCommentsList {
    #[serde(default)]
    pub comments: Vec<BktComment>,
}

#[derive(Debug, Deserialize)]
pub struct BktComment {
    pub id: u64,
    #[serde(default)]
    pub content: Option<BktContent>,
    #[serde(default)]
    pub parent: Option<BktParentRef>,
    #[serde(default)]
    pub inline: Option<BktInlineRef>,
    #[serde(default)]
    pub resolution: Option<BktResolution>,
    #[serde(default)]
    pub created_on: Option<DateTime<Utc>>,
    #[serde(default)]
    pub user: Option<BktCommentUser>,
}

#[derive(Debug, Deserialize)]
pub struct BktContent {
    #[serde(default)]
    pub raw: String,
}

#[derive(Debug, Deserialize)]
pub struct BktParentRef {
    pub id: u64,
}

#[derive(Debug, Deserialize)]
pub struct BktInlineRef {
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub from: Option<u32>,
    #[serde(default)]
    pub to: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub struct BktResolution {
    #[serde(default)]
    pub user: Option<BktCommentUser>,
    #[serde(default)]
    pub created_on: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct BktCommentUser {
    #[serde(default)]
    pub display_name: Option<String>,
}

// ── Conversion helpers ───────────────────────────────────────────────────

/// Map a `bkt pr view` response into our domain `PullRequestDetails`.
pub fn pr_view_to_details(
    view: BktPullRequest,
    host: &str,
    owner: &str,
    repo: &str,
    number: u64,
    base_sha: String,
) -> Result<super::super::traits::PullRequestDetails> {
    let head_sha = view
        .source
        .as_ref()
        .and_then(|s| s.commit.as_ref())
        .map(|c| c.hash.clone())
        .ok_or_else(|| {
            TuicrError::Forge("Bitbucket response did not include source commit hash".to_string())
        })?;

    let url = view
        .links
        .as_ref()
        .and_then(|l| l.html.as_ref())
        .map(|h| h.href.clone())
        .unwrap_or_default();

    let is_closed = view.state == "DECLINED";
    let is_merged = view.state == "MERGED";

    Ok(super::super::traits::PullRequestDetails {
        repository: super::super::traits::ForgeRepository::bitbucket(
            host.to_string(),
            owner.to_string(),
            repo.to_string(),
        ),
        number,
        title: view.title,
        url,
        state: view.state,
        is_draft: view.draft,
        author: view.author.and_then(|a| a.display_name),
        head_ref_name: view
            .source
            .as_ref()
            .and_then(|s| s.branch.as_ref())
            .and_then(|b| b.name.clone())
            .unwrap_or_default(),
        base_ref_name: view
            .destination
            .as_ref()
            .and_then(|d| d.branch.as_ref())
            .and_then(|b| b.name.clone())
            .unwrap_or_default(),
        head_sha,
        base_sha,
        body: view
            .summary
            .as_ref()
            .map(|s| s.raw.clone())
            .unwrap_or_default(),
        updated_at: view.updated_on,
        closed: is_closed,
        merged_at: if is_merged { view.updated_on } else { None },
        diff_start_sha: None,
    })
}

/// Map a `bkt pr list` entry into `PullRequestSummary`.
pub fn pr_list_entry_to_summary(
    entry: BktPullRequest,
    repository: &super::super::traits::ForgeRepository,
) -> super::super::traits::PullRequestSummary {
    let url = entry
        .links
        .as_ref()
        .and_then(|l| l.html.as_ref())
        .map(|h| h.href.clone())
        .unwrap_or_default();

    super::super::traits::PullRequestSummary {
        repository: repository.clone(),
        number: entry.id,
        title: entry.title,
        author: entry.author.and_then(|a| a.display_name),
        head_ref_name: entry
            .source
            .as_ref()
            .and_then(|s| s.branch.as_ref())
            .and_then(|b| b.name.clone())
            .unwrap_or_default(),
        base_ref_name: entry
            .destination
            .as_ref()
            .and_then(|d| d.branch.as_ref())
            .and_then(|b| b.name.clone())
            .unwrap_or_default(),
        updated_at: entry.updated_on,
        url,
        state: entry.state,
        is_draft: entry.draft,
    }
}
