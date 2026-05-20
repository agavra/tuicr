//! Manifest of saved review sessions.
//!
//! The manifest is the source of truth for slug -> session-file lookups. It
//! lives at `<reviews_dir>/index.json` and carries enough denormalized
//! metadata to drive `session list` without opening every session JSON. Each
//! session file remains self-describing, so the manifest can always be
//! rebuilt by walking the reviews directory.
#![allow(dead_code)]

use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::error::{Result, TuicrError};
use crate::model::ReviewSession;
use crate::model::review::SessionDiffSource;

pub const MANIFEST_FILENAME: &str = "index.json";
pub const MANIFEST_VERSION: &str = "2.0";
/// Subdirectory inside `reviews/` where session JSON files live under the
/// flat layout. The presence of this directory signals "current layout" to
/// the migration check.
pub const SESSIONS_DIRNAME: &str = "sessions";

/// A slug may map to more than one [`ManifestEntry`] when two local checkouts
/// of the same repo are both under review (same slug, different canonical
/// paths). PR slugs always have at most one entry — the current head's
/// session. Lookups disambiguate by canonical path (local) or kind (PR).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Manifest {
    pub version: String,
    pub entries: HashMap<String, Vec<ManifestEntry>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestEntry {
    /// Path to the session JSON, relative to the reviews directory.
    pub path: PathBuf,
    pub kind: ManifestKind,
    pub updated_at: DateTime<Utc>,
    /// Canonical path of the local checkout this session belongs to. `None`
    /// for PR sessions. Lets `session list` disambiguate two checkouts of
    /// the same repo when their slugs collide.
    #[serde(default)]
    pub canonical_repo_path: Option<PathBuf>,
    pub display: DisplayMetadata,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ManifestKind {
    Local,
    Pr { number: u64, head_sha: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct DisplayMetadata {
    pub comment_count: usize,
    pub reviewed_count: usize,
    pub file_count: usize,
    /// Slug anchor segment as it appears in the slug (branch name or short
    /// SHA). Cached so listings don't need to re-parse the slug.
    pub anchor: String,
}

impl Manifest {
    pub fn new() -> Self {
        Self {
            version: MANIFEST_VERSION.to_string(),
            entries: HashMap::new(),
        }
    }

    /// Find the local entry for `slug` that matches `canonical_repo_path`.
    pub fn get_local(&self, slug: &str, canonical_repo_path: &Path) -> Option<&ManifestEntry> {
        self.entries.get(slug)?.iter().find(|e| {
            matches!(e.kind, ManifestKind::Local)
                && e.canonical_repo_path.as_deref() == Some(canonical_repo_path)
        })
    }

    /// Find the PR entry for `slug` (PR entries are singletons per slug).
    pub fn get_pr(&self, slug: &str) -> Option<&ManifestEntry> {
        self.entries
            .get(slug)?
            .iter()
            .find(|e| matches!(e.kind, ManifestKind::Pr { .. }))
    }

    /// Insert or replace an entry. For local entries the matching key is
    /// `(slug, canonical_repo_path)`; for PR entries the slug alone suffices
    /// because each PR slug holds at most one entry.
    pub fn upsert(&mut self, slug: String, entry: ManifestEntry) {
        let bucket = self.entries.entry(slug).or_default();
        let idx = bucket
            .iter()
            .position(|existing| match (&existing.kind, &entry.kind) {
                (ManifestKind::Pr { .. }, ManifestKind::Pr { .. }) => true,
                (ManifestKind::Local, ManifestKind::Local) => {
                    existing.canonical_repo_path == entry.canonical_repo_path
                }
                _ => false,
            });
        match idx {
            Some(i) => bucket[i] = entry,
            None => bucket.push(entry),
        }
    }

    /// Remove all entries for `slug`. Returns the removed entries (empty if
    /// no slug match).
    pub fn remove_all(&mut self, slug: &str) -> Vec<ManifestEntry> {
        self.entries.remove(slug).unwrap_or_default()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&String, &ManifestEntry)> {
        self.entries
            .iter()
            .flat_map(|(slug, bucket)| bucket.iter().map(move |e| (slug, e)))
    }

    /// Total entry count across all slugs (a slug with two local entries
    /// counts twice).
    pub fn len(&self) -> usize {
        self.entries.values().map(|v| v.len()).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.values().all(|v| v.is_empty())
    }
}

/// Load the manifest from `<reviews_dir>/index.json`. Returns an empty
/// manifest if the file does not exist; returns a corruption error when the
/// file exists but cannot be parsed (callers may choose to recover by
/// rebuilding from session files).
pub fn load_manifest(reviews_dir: &Path) -> Result<Manifest> {
    let path = reviews_dir.join(MANIFEST_FILENAME);
    match fs::read_to_string(&path) {
        Ok(contents) => serde_json::from_str(&contents)
            .map_err(|e| TuicrError::CorruptedSession(format!("manifest parse error: {e}"))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Manifest::new()),
        Err(e) => Err(TuicrError::Io(e)),
    }
}

/// Save the manifest atomically: write to a sibling temp file, then rename.
/// A concurrent reader sees either the old version or the new version, never
/// a partial write.
pub fn save_manifest(reviews_dir: &Path, manifest: &Manifest) -> Result<()> {
    fs::create_dir_all(reviews_dir)?;
    let final_path = reviews_dir.join(MANIFEST_FILENAME);
    let tmp_path = reviews_dir.join(format!("{MANIFEST_FILENAME}.tmp"));

    let json = serde_json::to_string_pretty(manifest)?;
    {
        let mut tmp = fs::File::create(&tmp_path)?;
        tmp.write_all(json.as_bytes())?;
        tmp.sync_all().ok();
    }
    fs::rename(&tmp_path, &final_path)?;
    Ok(())
}

/// Build a manifest entry from a loaded session and the session file's path
/// relative to the reviews directory. The caller is responsible for
/// determining the slug (it is the manifest key) and for passing the correct
/// relative path.
pub fn entry_from_session(
    session: &ReviewSession,
    relative_path: PathBuf,
    anchor: String,
) -> ManifestEntry {
    let kind = match session.pr_session_key.as_ref() {
        Some(key) => ManifestKind::Pr {
            number: key.number,
            head_sha: key.head_sha.clone(),
        },
        None => ManifestKind::Local,
    };

    let canonical_repo_path = if matches!(kind, ManifestKind::Local) {
        fs::canonicalize(&session.repo_path)
            .ok()
            .or_else(|| Some(session.repo_path.clone()))
    } else {
        None
    };

    let comment_count = session.review_comments.len()
        + session
            .files
            .values()
            .map(|f| f.comment_count())
            .sum::<usize>();
    let reviewed_count = session.reviewed_count();
    let file_count = session.files.len();

    ManifestEntry {
        path: relative_path,
        kind,
        updated_at: session.updated_at,
        canonical_repo_path,
        display: DisplayMetadata {
            comment_count,
            reviewed_count,
            file_count,
            anchor,
        },
    }
}

/// Walk every `.json` file under `reviews_dir` and invoke `extract` to build
/// a `(slug, ManifestEntry)` pair. Returns the rebuilt manifest. Files that
/// `extract` cannot map are skipped. Ignores the manifest file itself.
pub fn rebuild_from_files<F>(reviews_dir: &Path, mut extract: F) -> Result<Manifest>
where
    F: FnMut(&Path) -> Option<(String, ManifestEntry)>,
{
    let mut manifest = Manifest::new();
    if !reviews_dir.exists() {
        return Ok(manifest);
    }
    walk_json(reviews_dir, &mut |path| {
        if let Some((slug, entry)) = extract(path) {
            manifest.upsert(slug, entry);
        }
    })?;
    Ok(manifest)
}

fn walk_json(dir: &Path, visit: &mut impl FnMut(&Path)) -> Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            walk_json(&path, visit)?;
            continue;
        }
        if !file_type.is_file() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if name == MANIFEST_FILENAME {
            continue;
        }
        if !name.ends_with(".json") {
            continue;
        }
        visit(&path);
    }
    Ok(())
}

/// Diff-source label used by the manifest's display metadata when no other
/// label is available. Kept as a free function so callers can derive an
/// anchor before constructing an entry.
pub fn diff_source_label(diff_source: SessionDiffSource) -> &'static str {
    match diff_source {
        SessionDiffSource::WorkingTree => "worktree",
        SessionDiffSource::Staged => "staged",
        SessionDiffSource::Unstaged => "unstaged",
        SessionDiffSource::StagedAndUnstaged => "staged-and-unstaged",
        SessionDiffSource::CommitRange => "commits",
        SessionDiffSource::WorkingTreeAndCommits => "worktree-and-commits",
        SessionDiffSource::StagedUnstagedAndCommits => "staged-and-unstaged-and-commits",
        SessionDiffSource::PullRequest => "pr",
        SessionDiffSource::Pristine => "pristine",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::FileStatus;
    use std::path::PathBuf;

    fn entry(updated_at: DateTime<Utc>, anchor: &str) -> ManifestEntry {
        ManifestEntry {
            path: PathBuf::from("local/abcd/agavra/tuicr/main/worktree.json"),
            kind: ManifestKind::Local,
            updated_at,
            canonical_repo_path: Some(PathBuf::from("/Users/agavra/dev/tuicr")),
            display: DisplayMetadata {
                comment_count: 3,
                reviewed_count: 1,
                file_count: 4,
                anchor: anchor.to_string(),
            },
        }
    }

    fn pr_entry(updated_at: DateTime<Utc>, number: u64, head_sha: &str) -> ManifestEntry {
        ManifestEntry {
            path: PathBuf::from(format!("gh/agavra/tuicr/pr/{number}/{head_sha}.json")),
            kind: ManifestKind::Pr {
                number,
                head_sha: head_sha.to_string(),
            },
            updated_at,
            canonical_repo_path: None,
            display: DisplayMetadata {
                comment_count: 0,
                reviewed_count: 0,
                file_count: 0,
                anchor: format!("pr/{number}"),
            },
        }
    }

    fn repo(name: &str) -> PathBuf {
        PathBuf::from(format!("/Users/agavra/{name}"))
    }

    #[test]
    fn should_start_empty() {
        let manifest = Manifest::new();
        assert!(manifest.is_empty());
        assert_eq!(manifest.len(), 0);
    }

    #[test]
    fn should_insert_and_retrieve_local_entry() {
        let mut manifest = Manifest::new();
        let mut e = entry(Utc::now(), "main");
        e.canonical_repo_path = Some(repo("tuicr"));
        manifest.upsert("agavra/tuicr@main/worktree".to_string(), e.clone());

        assert_eq!(manifest.len(), 1);
        assert_eq!(
            manifest.get_local("agavra/tuicr@main/worktree", &repo("tuicr")),
            Some(&e),
        );
        assert!(
            manifest
                .get_local("agavra/tuicr@main/worktree", &repo("other-checkout"))
                .is_none()
        );
    }

    #[test]
    fn should_replace_local_entry_with_same_canonical_path() {
        let mut manifest = Manifest::new();
        let mut first = entry(Utc::now(), "main");
        first.canonical_repo_path = Some(repo("tuicr"));
        let mut second = first.clone();
        second.display.comment_count = 99;

        manifest.upsert("agavra/tuicr@main/worktree".to_string(), first);
        manifest.upsert("agavra/tuicr@main/worktree".to_string(), second);

        assert_eq!(manifest.len(), 1);
        assert_eq!(
            manifest
                .get_local("agavra/tuicr@main/worktree", &repo("tuicr"))
                .unwrap()
                .display
                .comment_count,
            99,
        );
    }

    #[test]
    fn should_store_multiple_local_entries_for_same_slug_different_checkouts() {
        let mut manifest = Manifest::new();
        let mut work = entry(Utc::now(), "main");
        work.canonical_repo_path = Some(repo("work/tuicr"));
        let mut oss = entry(Utc::now(), "main");
        oss.canonical_repo_path = Some(repo("oss/tuicr"));

        manifest.upsert("agavra/tuicr@main/worktree".to_string(), work);
        manifest.upsert("agavra/tuicr@main/worktree".to_string(), oss);

        assert_eq!(manifest.len(), 2);
        assert!(
            manifest
                .get_local("agavra/tuicr@main/worktree", &repo("work/tuicr"))
                .is_some()
        );
        assert!(
            manifest
                .get_local("agavra/tuicr@main/worktree", &repo("oss/tuicr"))
                .is_some()
        );
    }

    #[test]
    fn should_remove_all_entries_for_slug() {
        let mut manifest = Manifest::new();
        let mut a = entry(Utc::now(), "main");
        a.canonical_repo_path = Some(repo("a"));
        let mut b = entry(Utc::now(), "main");
        b.canonical_repo_path = Some(repo("b"));
        manifest.upsert("agavra/tuicr@main/worktree".to_string(), a);
        manifest.upsert("agavra/tuicr@main/worktree".to_string(), b);

        let removed = manifest.remove_all("agavra/tuicr@main/worktree");
        assert_eq!(removed.len(), 2);
        assert!(manifest.is_empty());
    }

    #[test]
    fn should_replace_pr_entry_when_head_advances() {
        let mut manifest = Manifest::new();
        manifest.upsert(
            "gh:agavra/tuicr/pr/125".to_string(),
            pr_entry(Utc::now(), 125, "abc12345"),
        );
        manifest.upsert(
            "gh:agavra/tuicr/pr/125".to_string(),
            pr_entry(Utc::now(), 125, "def67890"),
        );

        assert_eq!(manifest.len(), 1);
        let entry = manifest.get_pr("gh:agavra/tuicr/pr/125").unwrap();
        match &entry.kind {
            ManifestKind::Pr { head_sha, .. } => assert_eq!(head_sha, "def67890"),
            _ => panic!("expected Pr"),
        }
    }

    #[test]
    fn should_return_empty_manifest_when_file_missing() {
        let dir = tempfile::tempdir().unwrap();
        let manifest = load_manifest(dir.path()).unwrap();
        assert!(manifest.is_empty());
    }

    #[test]
    fn should_roundtrip_manifest_through_disk() {
        let dir = tempfile::tempdir().unwrap();
        let mut manifest = Manifest::new();
        let mut local = entry(Utc::now(), "main");
        local.canonical_repo_path = Some(repo("tuicr"));
        manifest.upsert("agavra/tuicr@main/worktree".to_string(), local.clone());
        manifest.upsert(
            "gh:agavra/tuicr/pr/125".to_string(),
            pr_entry(Utc::now(), 125, "abc12345"),
        );

        save_manifest(dir.path(), &manifest).unwrap();
        let loaded = load_manifest(dir.path()).unwrap();

        assert_eq!(loaded.len(), 2);
        assert_eq!(
            loaded.get_local("agavra/tuicr@main/worktree", &repo("tuicr")),
            Some(&local),
        );
        assert!(loaded.get_pr("gh:agavra/tuicr/pr/125").is_some());
    }

    #[test]
    fn should_overwrite_manifest_atomically() {
        let dir = tempfile::tempdir().unwrap();
        let mut first = Manifest::new();
        first.upsert("a".to_string(), entry(Utc::now(), "main"));
        save_manifest(dir.path(), &first).unwrap();

        let mut second = Manifest::new();
        second.upsert("a".to_string(), entry(Utc::now(), "main"));
        second.upsert("b".to_string(), entry(Utc::now(), "feature"));
        save_manifest(dir.path(), &second).unwrap();

        let loaded = load_manifest(dir.path()).unwrap();
        assert_eq!(loaded.len(), 2);

        let tmp = dir.path().join(format!("{MANIFEST_FILENAME}.tmp"));
        assert!(!tmp.exists(), "tmp file should be cleaned up after rename");
    }

    #[test]
    fn should_surface_corrupted_manifest_as_error() {
        let dir = tempfile::tempdir().unwrap();
        let manifest_path = dir.path().join(MANIFEST_FILENAME);
        fs::write(&manifest_path, "not json {").unwrap();

        let err = load_manifest(dir.path()).unwrap_err();
        assert!(matches!(err, TuicrError::CorruptedSession(_)));
    }

    #[test]
    fn should_walk_only_json_files_excluding_manifest() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("local").join("abcd")).unwrap();
        fs::write(root.join("local/abcd/a.json"), "{}").unwrap();
        fs::write(root.join("local/abcd/b.txt"), "ignored").unwrap();
        fs::write(root.join(MANIFEST_FILENAME), "{}").unwrap();

        let mut visited: Vec<String> = Vec::new();
        walk_json(root, &mut |path| {
            visited.push(path.file_name().unwrap().to_string_lossy().to_string());
        })
        .unwrap();

        assert_eq!(visited, vec!["a.json".to_string()]);
    }

    #[test]
    fn should_rebuild_manifest_via_extractor() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("local").join("abcd")).unwrap();
        fs::write(root.join("local/abcd/a.json"), "{}").unwrap();
        fs::write(root.join("local/abcd/b.json"), "{}").unwrap();

        let manifest = rebuild_from_files(root, |path| {
            let name = path.file_stem()?.to_str()?.to_string();
            let mut e = entry(Utc::now(), &name);
            e.canonical_repo_path = Some(repo(&name));
            Some((format!("repo@{name}/worktree"), e))
        })
        .unwrap();

        assert_eq!(manifest.len(), 2);
        assert!(manifest.get_local("repo@a/worktree", &repo("a")).is_some());
        assert!(manifest.get_local("repo@b/worktree", &repo("b")).is_some());
    }

    #[test]
    fn should_rebuild_empty_manifest_when_dir_missing() {
        let dir = tempfile::tempdir().unwrap();
        let nonexistent = dir.path().join("does-not-exist");
        let manifest =
            rebuild_from_files(&nonexistent, |_| panic!("should not be called")).unwrap();
        assert!(manifest.is_empty());
    }

    // ---- entry_from_session ----

    #[test]
    fn should_extract_metadata_from_local_session() {
        let mut session = ReviewSession::new(
            PathBuf::from("/tmp/test-repo"),
            "abc1234def".to_string(),
            Some("main".to_string()),
            SessionDiffSource::WorkingTree,
        );
        session.add_file(PathBuf::from("src/a.rs"), FileStatus::Modified, 0);
        session.add_file(PathBuf::from("src/b.rs"), FileStatus::Modified, 0);
        session
            .get_file_mut(&PathBuf::from("src/a.rs"))
            .unwrap()
            .reviewed = true;

        let relative = PathBuf::from("local/abcd/foo/bar/main/worktree.json");
        let entry = entry_from_session(&session, relative.clone(), "main".to_string());

        assert_eq!(entry.path, relative);
        assert!(matches!(entry.kind, ManifestKind::Local));
        assert_eq!(entry.updated_at, session.updated_at);
        assert_eq!(entry.display.file_count, 2);
        assert_eq!(entry.display.reviewed_count, 1);
        assert_eq!(entry.display.comment_count, 0);
        assert_eq!(entry.display.anchor, "main");
    }

    #[test]
    fn should_extract_metadata_from_pr_session() {
        use crate::forge::traits::{ForgeRepository, PrSessionKey};
        let mut session = ReviewSession::new(
            PathBuf::from("forge:github.com/agavra/tuicr"),
            "abcdef0123456789".to_string(),
            Some("reviews".to_string()),
            SessionDiffSource::PullRequest,
        );
        session.pr_session_key = Some(PrSessionKey::new(
            ForgeRepository::github("github.com", "agavra", "tuicr"),
            125,
            "abcdef0123456789".to_string(),
        ));

        let entry = entry_from_session(
            &session,
            PathBuf::from("gh/agavra/tuicr/pr/125/abcdef01.json"),
            "pr/125".to_string(),
        );

        match &entry.kind {
            ManifestKind::Pr { number, head_sha } => {
                assert_eq!(*number, 125);
                assert_eq!(head_sha, "abcdef0123456789");
            }
            _ => panic!("expected Pr kind, got {:?}", entry.kind),
        }
        assert_eq!(entry.canonical_repo_path, None);
    }
}
