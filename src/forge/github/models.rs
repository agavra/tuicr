use chrono::{DateTime, Utc};
use serde::Deserialize;

use crate::error::{Result, TuicrError};
use crate::forge::traits::{ForgeRepository, PullRequestDetails, PullRequestSummary};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GhPullRequestSummary {
    pub number: u64,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub author: Option<GhAuthor>,
    #[serde(default)]
    pub head_ref_name: String,
    #[serde(default)]
    pub base_ref_name: String,
    #[serde(default)]
    pub updated_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub is_draft: bool,
}

impl GhPullRequestSummary {
    pub fn into_summary(self, repository: &ForgeRepository) -> PullRequestSummary {
        PullRequestSummary {
            repository: repository.clone(),
            number: self.number,
            title: self.title,
            author: self.author.and_then(|author| author.login),
            head_ref_name: self.head_ref_name,
            base_ref_name: self.base_ref_name,
            updated_at: self.updated_at,
            url: self.url,
            state: self.state,
            is_draft: self.is_draft,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GhPullRequestDetails {
    pub number: u64,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub is_draft: bool,
    #[serde(default)]
    pub author: Option<GhAuthor>,
    #[serde(default)]
    pub head_ref_name: String,
    #[serde(default)]
    pub base_ref_name: String,
    #[serde(default)]
    pub head_ref_oid: String,
    #[serde(default)]
    pub base_ref_oid: String,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub updated_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub closed: bool,
    #[serde(default)]
    pub merged_at: Option<DateTime<Utc>>,
}

impl GhPullRequestDetails {
    pub fn into_details(self, repository: &ForgeRepository) -> Result<PullRequestDetails> {
        require_field(&self.head_ref_oid, "headRefOid")?;
        require_field(&self.base_ref_oid, "baseRefOid")?;

        Ok(PullRequestDetails {
            repository: repository.clone(),
            number: self.number,
            title: self.title,
            url: self.url,
            state: self.state,
            is_draft: self.is_draft,
            author: self.author.and_then(|author| author.login),
            head_ref_name: self.head_ref_name,
            base_ref_name: self.base_ref_name,
            head_sha: self.head_ref_oid,
            base_sha: self.base_ref_oid,
            body: self.body,
            updated_at: self.updated_at,
            closed: self.closed,
            merged_at: self.merged_at,
        })
    }
}

#[derive(Debug, Deserialize)]
pub struct GhAuthor {
    #[serde(default)]
    pub login: Option<String>,
}

fn require_field(value: &str, field: &str) -> Result<()> {
    if value.is_empty() {
        Err(TuicrError::Forge(format!(
            "GitHub response did not include required field `{field}`"
        )))
    } else {
        Ok(())
    }
}
