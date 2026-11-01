use chrono::{DateTime, Utc};
use serde::Deserialize;
use std::path::PathBuf;

use crate::error::{Result, TuicrError};
use crate::forge::traits::{
    ForgeRepository, PullRequestCommit, PullRequestDetails, PullRequestSummary,
};
use crate::model::{FilePatch, FileStatus};
use crate::vcs::git::raw::FileMetadata;

/// Machine-readable entry from the pull-request files REST endpoint.
#[derive(Clone, Debug, Deserialize)]
pub struct GhPullRequestFile {
    pub filename: String,
    pub status: String,
    #[serde(default)]
    pub previous_filename: Option<String>,
    /// Hunks for this file. GitHub omits it for binary files and for files
    /// whose own diff is too large, so absent does not mean unchanged.
    #[serde(default)]
    pub patch: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct GhCompare {
    #[serde(default)]
    pub files: Vec<GhPullRequestFile>,
    /// Three-dot merge base of the two compared revisions. Read by
    /// `resolve_diff_base_sha`; absent from responses that predate it or when
    /// GitHub omits it, hence `Option`.
    #[serde(default)]
    pub merge_base_commit: Option<GhComparisonCommit>,
}

#[derive(Debug, Deserialize)]
pub struct GhComparisonCommit {
    #[serde(default)]
    pub sha: String,
}

impl GhPullRequestFile {
    pub(crate) fn into_file_patch(mut self) -> Result<FilePatch> {
        let patch = self.patch.take().unwrap_or_default();
        let metadata = self.into_metadata()?;
        let mut file = FilePatch::new(metadata.old_path, metadata.new_path, metadata.status, patch);
        // GitHub does not say whether an omitted patch is binary or too large.
        file.is_unavailable = file.patch.is_empty();
        Ok(file)
    }

    pub(crate) fn into_metadata(self) -> Result<FileMetadata> {
        let new_path = PathBuf::from(&self.filename);
        let metadata = match self.status.as_str() {
            "added" => FileMetadata {
                old_path: None,
                new_path: Some(new_path),
                status: FileStatus::Added,
            },
            "removed" => FileMetadata {
                old_path: Some(new_path),
                new_path: None,
                status: FileStatus::Deleted,
            },
            "renamed" => FileMetadata {
                old_path: Some(PathBuf::from(self.previous_filename.ok_or_else(|| {
                    TuicrError::Forge(format!(
                        "GitHub renamed file `{}` has no previous_filename",
                        self.filename
                    ))
                })?)),
                new_path: Some(new_path),
                status: FileStatus::Renamed,
            },
            "copied" => FileMetadata {
                old_path: Some(PathBuf::from(self.previous_filename.ok_or_else(|| {
                    TuicrError::Forge(format!(
                        "GitHub copied file `{}` has no previous_filename",
                        self.filename
                    ))
                })?)),
                new_path: Some(new_path),
                status: FileStatus::Copied,
            },
            "modified" | "changed" | "unchanged" => FileMetadata {
                old_path: Some(new_path.clone()),
                new_path: Some(new_path),
                status: FileStatus::Modified,
            },
            status => {
                return Err(TuicrError::Forge(format!(
                    "GitHub returned unsupported file status `{status}` for `{}`",
                    self.filename
                )));
            }
        };
        Ok(metadata)
    }
}

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
            diff_start_sha: None,
        })
    }
}

#[derive(Debug, Deserialize)]
pub struct GhAuthor {
    #[serde(default)]
    pub login: Option<String>,
}

/// Response shape for `gh api repos/<owner>/<repo>/pulls/<num>/commits`.
/// We only consume a small subset of fields; the rest are ignored.
#[derive(Debug, Deserialize)]
pub struct GhPrCommit {
    pub sha: String,
    #[serde(default)]
    pub commit: GhCommitDetails,
}

#[derive(Debug, Default, Deserialize)]
pub struct GhCommitDetails {
    #[serde(default)]
    pub message: String,
    #[serde(default)]
    pub author: Option<GhCommitAuthor>,
}

#[derive(Debug, Deserialize)]
pub struct GhCommitAuthor {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub date: Option<DateTime<Utc>>,
}

impl GhPrCommit {
    pub fn into_pull_request_commit(self) -> PullRequestCommit {
        let summary = self.commit.message.lines().next().unwrap_or("").to_string();
        let (author, timestamp) = match self.commit.author {
            Some(a) => (
                a.name.or(a.email).unwrap_or_else(|| "unknown".to_string()),
                a.date,
            ),
            None => ("unknown".to_string(), None),
        };
        let short_oid = self.sha.chars().take(7).collect();
        PullRequestCommit {
            oid: self.sha,
            short_oid,
            summary,
            author,
            timestamp,
        }
    }
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

#[cfg(test)]
mod tests {

    #[test]
    fn fallback_preserves_metadata_and_hunks() {
        for (status, previous, old, new) in [
            ("added", None, None, Some("新 b/file.txt")),
            ("removed", None, Some("新 b/file.txt"), None),
            (
                "modified",
                None,
                Some("新 b/file.txt"),
                Some("新 b/file.txt"),
            ),
            (
                "renamed",
                Some("旧 a/file.txt"),
                Some("旧 a/file.txt"),
                Some("新 b/file.txt"),
            ),
            (
                "copied",
                Some("旧 a/file.txt"),
                Some("旧 a/file.txt"),
                Some("新 b/file.txt"),
            ),
        ] {
            let file = GhPullRequestFile {
                filename: "新 b/file.txt".into(),
                status: status.into(),
                previous_filename: previous.map(str::to_owned),
                patch: Some("@@ -1 +1 @@\n-a\n+b".into()),
            }
            .into_file_patch()
            .unwrap();
            assert_eq!(file.old_path.as_deref(), old.map(Path::new));
            assert_eq!(file.new_path.as_deref(), new.map(Path::new));
            assert_eq!(file.patch, "@@ -1 +1 @@\n-a\n+b");
            assert!(!file.is_unavailable);
        }
    }

    #[test]
    fn fallback_marks_missing_and_empty_patches_without_guessing_the_cause() {
        for patch in [None, Some(String::new())] {
            let file = GhPullRequestFile {
                filename: "logo.png".into(),
                status: "modified".into(),
                previous_filename: None,
                patch,
            }
            .into_file_patch()
            .unwrap();
            assert!(file.is_unavailable);
            assert!(!file.is_binary);
            assert!(!file.is_too_large);
        }
    }

    #[test]
    fn fallback_validates_metadata() {
        for status in ["renamed", "copied", "unknown"] {
            let file = GhPullRequestFile {
                filename: "after.rs".into(),
                status: status.into(),
                previous_filename: None,
                patch: None,
            };
            assert!(file.into_file_patch().is_err());
        }
    }
    use super::*;
    use std::path::Path;

    #[test]
    fn file_metadata_uses_json_paths_verbatim() {
        let file: GhPullRequestFile = serde_json::from_str(
            r#"{
                "filename":"新 b/right and side.txt",
                "previous_filename":"旧 b/left and side.txt",
                "status":"renamed"
            }"#,
        )
        .unwrap();
        let metadata = file.into_metadata().unwrap();

        assert_eq!(metadata.status, FileStatus::Renamed);
        assert_eq!(
            metadata.old_path.as_deref(),
            Some(Path::new("旧 b/left and side.txt"))
        );
        assert_eq!(
            metadata.new_path.as_deref(),
            Some(Path::new("新 b/right and side.txt"))
        );
    }

    #[test]
    fn renamed_file_requires_previous_filename() {
        let file: GhPullRequestFile =
            serde_json::from_str(r#"{"filename":"new.txt","status":"renamed"}"#).unwrap();
        assert!(file.into_metadata().is_err());
    }
}
