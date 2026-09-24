use chrono::{DateTime, Utc};
use serde::Deserialize;
use std::path::PathBuf;

use crate::error::{Result, TuicrError};
use crate::forge::traits::{
    ForgeRepository, PullRequestCommit, PullRequestDetails, PullRequestSummary,
};
use crate::model::FileStatus;
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
}

impl GhPullRequestFile {
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

/// Rebuilds a unified diff from `pulls/<n>/files` rows.
///
/// `gh pr diff` refuses any pull request touching more than 300 files, but
/// the files endpoint paginates and carries each file's hunks, so the diff
/// can be reassembled client side. Emits exactly one `diff --git` block per
/// row, including rows with no patch, because the caller pairs blocks with
/// metadata positionally.
pub(crate) fn synthesize_patch(files: &[GhPullRequestFile]) -> String {
    let mut out = String::new();
    for f in files {
        let new_path = f.filename.as_str();
        let old_path = match f.status.as_str() {
            "renamed" => f.previous_filename.as_deref().unwrap_or(new_path),
            _ => new_path,
        };
        out.push_str(&format!("diff --git a/{old_path} b/{new_path}\n"));
        match f.status.as_str() {
            "added" => {
                out.push_str("--- /dev/null\n");
                out.push_str(&format!("+++ b/{new_path}\n"));
            }
            "removed" => {
                out.push_str(&format!("--- a/{old_path}\n"));
                out.push_str("+++ /dev/null\n");
            }
            _ => {
                out.push_str(&format!("--- a/{old_path}\n"));
                out.push_str(&format!("+++ b/{new_path}\n"));
            }
        }
        if let Some(patch) = f.patch.as_deref() {
            out.push_str(patch);
            if !patch.ends_with('\n') {
                out.push('\n');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {

    /// `gh pr diff` refuses a pull request over 300 files, so the diff is
    /// rebuilt from the files endpoint. The caller pairs blocks with metadata
    /// positionally, so every row must produce exactly one block, including
    /// binary files, which carry no patch at all.
    #[test]
    fn should_emit_one_block_per_file_including_patchless_ones() {
        let files = vec![
            GhPullRequestFile {
                filename: "a.rs".into(),
                status: "modified".into(),
                previous_filename: None,
                patch: Some("@@ -1 +1 @@\n-a\n+b".into()),
            },
            GhPullRequestFile {
                filename: "logo.png".into(),
                status: "modified".into(),
                previous_filename: None,
                patch: None,
            },
        ];

        let out = synthesize_patch(&files);

        assert_eq!(out.matches("diff --git ").count(), 2);
        assert!(out.contains("diff --git a/logo.png b/logo.png"));
        assert!(out.ends_with('\n'));
    }

    #[test]
    fn should_use_dev_null_for_added_and_removed_files() {
        let files = vec![
            GhPullRequestFile {
                filename: "new.rs".into(),
                status: "added".into(),
                previous_filename: None,
                patch: Some("@@ -0,0 +1 @@\n+x\n".into()),
            },
            GhPullRequestFile {
                filename: "gone.rs".into(),
                status: "removed".into(),
                previous_filename: None,
                patch: Some("@@ -1 +0,0 @@\n-y\n".into()),
            },
        ];

        let out = synthesize_patch(&files);

        assert!(out.contains("--- /dev/null\n+++ b/new.rs"));
        assert!(out.contains("--- a/gone.rs\n+++ /dev/null"));
    }

    #[test]
    fn should_name_both_sides_of_a_rename() {
        let files = vec![GhPullRequestFile {
            filename: "after.rs".into(),
            status: "renamed".into(),
            previous_filename: Some("before.rs".into()),
            patch: None,
        }];

        let out = synthesize_patch(&files);

        assert!(out.contains("diff --git a/before.rs b/after.rs"));
        assert!(out.contains("--- a/before.rs\n+++ b/after.rs"));
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
