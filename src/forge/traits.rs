use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::error::Result;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ForgeKind {
    GitHub,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForgeRepository {
    pub kind: ForgeKind,
    pub host: String,
    pub owner: String,
    pub name: String,
}

impl ForgeRepository {
    pub fn github(
        host: impl Into<String>,
        owner: impl Into<String>,
        name: impl Into<String>,
    ) -> Self {
        Self {
            kind: ForgeKind::GitHub,
            host: host.into(),
            owner: owner.into(),
            name: name.into(),
        }
    }

    pub fn slug(&self) -> String {
        format!("{}/{}", self.owner, self.name)
    }

    pub fn display_name(&self) -> String {
        if self.host == "github.com" {
            self.slug()
        } else {
            format!("{}/{}", self.host, self.slug())
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PullRequestTarget {
    pub repository: Option<ForgeRepository>,
    pub number: u64,
    pub original: String,
}

impl PullRequestTarget {
    pub fn number(number: u64, original: impl Into<String>) -> Self {
        Self {
            repository: None,
            number,
            original: original.into(),
        }
    }

    pub fn with_repository(
        repository: ForgeRepository,
        number: u64,
        original: impl Into<String>,
    ) -> Self {
        Self {
            repository: Some(repository),
            number,
            original: original.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PullRequestListQuery {
    pub repository: ForgeRepository,
    pub already_loaded: usize,
    pub page_size: usize,
}

impl PullRequestListQuery {
    pub fn first_page(repository: ForgeRepository, page_size: usize) -> Self {
        Self {
            repository,
            already_loaded: 0,
            page_size,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PullRequestSummary {
    pub repository: ForgeRepository,
    pub number: u64,
    pub title: String,
    pub author: Option<String>,
    pub head_ref_name: String,
    pub base_ref_name: String,
    pub updated_at: Option<DateTime<Utc>>,
    pub url: String,
    pub state: String,
    pub is_draft: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PagedPullRequests {
    pub pull_requests: Vec<PullRequestSummary>,
    pub has_more: bool,
    pub total_loaded: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PullRequestDetails {
    pub repository: ForgeRepository,
    pub number: u64,
    pub title: String,
    pub url: String,
    pub state: String,
    pub is_draft: bool,
    pub author: Option<String>,
    pub head_ref_name: String,
    pub base_ref_name: String,
    pub head_sha: String,
    pub base_sha: String,
    pub body: String,
    pub updated_at: Option<DateTime<Utc>>,
    pub closed: bool,
    pub merged_at: Option<DateTime<Utc>>,
}

impl PullRequestDetails {
    pub fn is_read_only(&self) -> bool {
        self.closed || self.merged_at.is_some()
    }
}

pub trait ForgeBackend {
    fn list_pull_requests(&self, query: PullRequestListQuery) -> Result<PagedPullRequests>;
    fn get_pull_request(&self, target: PullRequestTarget) -> Result<PullRequestDetails>;
    fn get_pull_request_diff(&self, pr: &PullRequestDetails) -> Result<String>;
}
