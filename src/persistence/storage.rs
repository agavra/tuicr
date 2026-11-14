//! Slug-addressed storage layer for review sessions.
//!
//! Layout under the platform data dir's `tuicr/reviews/`:
//!
//! ```text
//! reviews/
//!   index.json                # manifest, source of truth for lookups
//!   sessions/
//!     <16-hex>.json           # one file per session, deterministic name
//! ```
//!
//! The session filename is a hash of the slug plus the canonical repo path
//! (for local) or head SHA (for PR), so the same logical session always
//! lands at the same path without consulting the manifest. The manifest is
//! the authoritative slug -> file mapping; if it goes missing or corrupts,
//! the session JSONs are self-describing and the manifest can be rebuilt by
//! walking `sessions/`.

use std::cmp::Reverse;
use std::collections::HashSet;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use chrono::Utc;
use serde::{Deserialize, Serialize};

#[cfg(not(test))]
use directories::ProjectDirs;

use crate::error::{Result, TuicrError};
use crate::forge::traits::PrSessionKey;
use crate::hash::Fnv1aHasher;
use crate::model::ReviewSession;
use crate::model::review::SessionDiffSource;
use crate::persistence::manifest::{
    self, MANIFEST_FILENAME, ManifestEntry, ManifestKind, SESSIONS_DIRNAME,
};
use crate::slug::{
    self, PrSlug, RepoCoordinate, Slug, default_host, forge_code, normalized_host, same_pr_identity,
};

const STORAGE_LOCK_FILENAME: &str = ".tuicr.lock";
const STORAGE_LOCK_TIMEOUT: Duration = Duration::from_secs(10);
#[cfg(not(test))]
const STORAGE_LOCK_STALE_AFTER: Duration = Duration::from_secs(60);
#[cfg(test)]
const STORAGE_LOCK_STALE_AFTER: Duration = Duration::from_millis(0);
const STORAGE_LOCK_REUSE_GUARD_AFTER: Duration = Duration::from_secs(12 * 60 * 60);
const ACTIVE_SESSIONS_FILENAME: &str = "active_sessions.json";
const ACTIVE_SESSION_STALE_AFTER: Duration = Duration::from_secs(12 * 60 * 60);

// ---------- Public API ----------

/// Save a session to disk and update the manifest. The on-disk path is
/// derived from the session's slug; the slug is computed from the session's
/// fields at save time.
pub fn save_session(session: &ReviewSession) -> Result<PathBuf> {
    let reviews_dir = get_reviews_dir()?;
    save_session_in_dir(session, &reviews_dir)
}

pub(crate) fn save_session_in_dir(session: &ReviewSession, reviews_dir: &Path) -> Result<PathBuf> {
    maybe_migrate(reviews_dir)?;
    with_reviews_dir_lock(reviews_dir, || {
        save_session_in_dir_unlocked(session, reviews_dir)
    })
}

/// Re-register the session file at `path` in the manifest (used when the file
/// exists but its manifest entry may be missing). `path` must be one of the
/// files this session's identity resolves to.
pub(crate) fn reindex_session(session: &ReviewSession, path: &Path) -> Result<()> {
    let reviews_dir = get_reviews_dir()?;
    maybe_migrate(&reviews_dir)?;
    with_reviews_dir_lock(&reviews_dir, || {
        let target = match session.pr_session_key.as_ref() {
            Some(key) => pr_target_for_path(&reviews_dir, key, path).ok_or_else(|| {
                TuicrError::SessionTargetChanged(format!(
                    "{} is not this review's session file",
                    path.display()
                ))
            })?,
            None => target_for_session(session, &reviews_dir)?,
        };
        let full_path = reviews_dir.join(&target.relative);
        if !full_path.is_file() {
            return Err(TuicrError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("session file not found: {}", full_path.display()),
            )));
        }
        if let Some(key) = session.pr_session_key.as_ref() {
            verify_occupant(&full_path, key)?;
        }
        upsert_session_manifest(session, &reviews_dir, &target)
    })
}

/// Update the session behind `session_ref`. A PR session is written back to
/// the verified ref path or not at all: when identity resolution now names a
/// different file, this returns [`TuicrError::SessionTargetChanged`] and
/// writes nothing.
pub(crate) fn update_session_in_dir<T>(
    session_ref: &Path,
    reviews_dir: &Path,
    update: impl FnOnce(&mut ReviewSession) -> Result<T>,
) -> Result<(ReviewSession, T)> {
    maybe_migrate(reviews_dir)?;
    with_reviews_dir_lock(reviews_dir, || {
        let mut session = load_session(session_ref)?;
        let output = update(&mut session)?;
        if session.pr_session_key.is_some() {
            save_session_at_unlocked(&session, reviews_dir, Some(session_ref))?;
        } else {
            write_back_local_unlocked(&session, reviews_dir, session_ref)?;
        }
        Ok((session, output))
    })
}

/// Load-merge-save keyed by session identity. `cached` is the path the caller
/// already holds for this session; when it exists but identity resolution now
/// names another file, the save fails with
/// [`TuicrError::SessionTargetChanged`] instead of silently switching files.
pub(crate) fn save_session_by_identity<T>(
    identity: &ReviewSession,
    cached: Option<&Path>,
    update: impl FnOnce(Option<ReviewSession>) -> Result<(ReviewSession, T)>,
) -> Result<(PathBuf, ReviewSession, T)> {
    let reviews_dir = get_reviews_dir()?;
    save_session_by_identity_in_dir(identity, &reviews_dir, cached, update)
}

pub(crate) fn save_session_by_identity_in_dir<T>(
    identity: &ReviewSession,
    reviews_dir: &Path,
    cached: Option<&Path>,
    update: impl FnOnce(Option<ReviewSession>) -> Result<(ReviewSession, T)>,
) -> Result<(PathBuf, ReviewSession, T)> {
    maybe_migrate(reviews_dir)?;
    with_reviews_dir_lock(reviews_dir, || {
        let path = session_path_in_dir(identity, reviews_dir)?;
        let persisted = if path.exists() {
            Some(load_session(&path)?)
        } else {
            None
        };
        let (session, output) = update(persisted)?;
        let saved_path = save_session_at_unlocked(&session, reviews_dir, cached)?;
        Ok((saved_path, session, output))
    })
}

pub(crate) fn session_path(session: &ReviewSession) -> Result<PathBuf> {
    let reviews_dir = get_reviews_dir()?;
    session_path_in_dir(session, &reviews_dir)
}

pub(crate) fn session_path_in_dir(session: &ReviewSession, reviews_dir: &Path) -> Result<PathBuf> {
    Ok(reviews_dir.join(target_for_session(session, reviews_dir)?.relative))
}

/// The slug tuicr announces for a session (startup marker, registry). For a
/// PR session it is derived from the key: unqualified on the forge's default
/// host, `@<host>` elsewhere. That is also the marker a legacy session of a
/// non-default host needs, because its hostless manifest key must never be
/// reachable by a hostless selector (see [`pr_session_path_in_dir`]).
pub(crate) fn session_marker_slug(session: &ReviewSession) -> Result<String> {
    match session.pr_session_key.as_ref() {
        Some(key) => Ok(Slug::from(key).to_string()),
        None => Ok(slug_for_session(session)?.to_string()),
    }
}

pub(crate) fn delete_session_if_empty(path: &Path) -> Result<bool> {
    let reviews_dir = get_reviews_dir()?;
    delete_session_if_empty_in_dir(path, &reviews_dir)
}

/// Like [`delete_session_if_empty`], but leaves the file alone while a live
/// tuicr has it open. For out-of-process callers such as `tuicr review
/// delete`: the owning TUI polls the file to merge external edits, and
/// cleans up an empty session itself on exit.
pub(crate) fn delete_session_if_empty_and_inactive(path: &Path) -> Result<bool> {
    let reviews_dir = get_reviews_dir()?;
    delete_session_if_empty_and_inactive_in_dir(path, &reviews_dir)
}

pub(crate) fn delete_session_if_empty_and_inactive_in_dir(
    path: &Path,
    reviews_dir: &Path,
) -> Result<bool> {
    maybe_migrate(reviews_dir)?;
    with_reviews_dir_lock(reviews_dir, || {
        if is_session_active_unlocked(reviews_dir, path) {
            return Ok(false);
        }
        delete_session_if_empty_unlocked(path, reviews_dir)
    })
}

pub(crate) fn delete_session_if_empty_in_dir(path: &Path, reviews_dir: &Path) -> Result<bool> {
    maybe_migrate(reviews_dir)?;
    with_reviews_dir_lock(reviews_dir, || {
        delete_session_if_empty_unlocked(path, reviews_dir)
    })
}

fn delete_session_if_empty_unlocked(path: &Path, reviews_dir: &Path) -> Result<bool> {
    if !path.exists() {
        return Ok(false);
    }
    let session = load_session(path)?;
    if session.has_comments() || session.has_reviewed_state() {
        return Ok(false);
    }
    remove_session_at(path, reviews_dir)?;
    Ok(true)
}

/// Whether a live tuicr has `path` open. The caller must hold the reviews-dir
/// lock, which [`mark_session_active_in_dir`] also takes.
fn is_session_active_unlocked(reviews_dir: &Path, path: &Path) -> bool {
    let target = normalize_active_path(path);
    load_active_sessions_unlocked(reviews_dir)
        .unwrap_or_default()
        .sessions
        .iter()
        .any(|entry| entry.is_fresh() && normalize_active_path(&entry.path) == target)
}

/// Delete a session file and its manifest entry, even one with reviewed-file
/// markers (the "discard reviewed-only state on quit" path), but keep a file whose on-disk copy holds
/// comments (for instance ones `tuicr review add` wrote since the last poll).
/// The check and the removal happen under one lock. Returns `false` when the
/// file was kept or absent.
pub(crate) fn delete_session_unless_commented(path: &Path) -> Result<bool> {
    let reviews_dir = get_reviews_dir()?;
    maybe_migrate(&reviews_dir)?;
    with_reviews_dir_lock(&reviews_dir, || {
        if path.exists() && load_session(path)?.has_comments() {
            return Ok(false);
        }
        delete_session_unlocked(path, &reviews_dir)
    })
}

#[cfg(test)]
pub(crate) fn delete_session_in_dir(path: &Path, reviews_dir: &Path) -> Result<bool> {
    maybe_migrate(reviews_dir)?;
    with_reviews_dir_lock(reviews_dir, || delete_session_unlocked(path, reviews_dir))
}

/// Outcome of [`delete_inactive_session_in_dir`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeleteOutcome {
    /// The session file and its manifest entry were removed.
    Deleted,
    /// No file was there; any manifest entry still pointing at it was pruned.
    Missing,
    /// Another live tuicr has the session open, so nothing was removed: its
    /// next autosave would write the file straight back.
    Active,
}

/// Like [`delete_session_in_dir`], but refuses a session a live tuicr has
/// open. The active check runs under the same lock that
/// [`mark_session_active_in_dir`] takes, so it sees the registry as of the
/// delete, not as of whenever the caller listed sessions.
///
/// A process that has loaded a session but not yet registered it can still
/// lose it here; that window is short and only costs it a recreated file.
pub(crate) fn delete_inactive_session_in_dir(
    path: &Path,
    reviews_dir: &Path,
) -> Result<DeleteOutcome> {
    maybe_migrate(reviews_dir)?;
    with_reviews_dir_lock(reviews_dir, || {
        if is_session_active_unlocked(reviews_dir, path) {
            return Ok(DeleteOutcome::Active);
        }
        Ok(if delete_session_unlocked(path, reviews_dir)? {
            DeleteOutcome::Deleted
        } else {
            DeleteOutcome::Missing
        })
    })
}

/// Remove the session at `path` and its manifest entry. The caller must hold
/// the reviews-dir lock. A file that is already gone still has its manifest
/// entry pruned, so listings stop surfacing it.
fn delete_session_unlocked(path: &Path, reviews_dir: &Path) -> Result<bool> {
    if !path.exists() {
        prune_manifest_path(reviews_dir, path)?;
        return Ok(false);
    }
    load_session(path)?;
    remove_session_at(path, reviews_dir)?;
    Ok(true)
}

/// Drop every manifest entry pointing at `path`, whatever its slug. Used when
/// the file is gone and its slug can no longer be read from it.
fn prune_manifest_path(reviews_dir: &Path, path: &Path) -> Result<()> {
    let relative = manifest_relative_path(path, reviews_dir);
    let mut manifest = manifest_for_update(reviews_dir)?;
    let before = manifest.len();
    remove_path_entries(&mut manifest, &relative);
    if manifest.len() != before {
        manifest::save_manifest(reviews_dir, &manifest)?;
    }
    Ok(())
}

fn manifest_relative_path(path: &Path, reviews_dir: &Path) -> PathBuf {
    path.strip_prefix(reviews_dir)
        .map(Path::to_path_buf)
        .unwrap_or_else(|_| path.to_path_buf())
}

/// Remove `path` and prune its manifest entries (whichever bucket holds
/// them). The caller must already hold the reviews-dir lock.
fn remove_session_at(path: &Path, reviews_dir: &Path) -> Result<()> {
    prune_manifest_path(reviews_dir, path)?;
    fs::remove_file(path)?;
    Ok(())
}

pub(crate) fn mark_session_active(session: &ReviewSession, path: &Path) -> Result<()> {
    let reviews_dir = get_reviews_dir()?;
    mark_session_active_in_dir(session, path, &reviews_dir)
}

pub(crate) fn mark_session_active_in_dir(
    session: &ReviewSession,
    path: &Path,
    reviews_dir: &Path,
) -> Result<()> {
    maybe_migrate(reviews_dir)?;
    let slug = session_marker_slug(session)?;
    let path = normalize_active_path(path);
    with_reviews_dir_lock(reviews_dir, || {
        let mut active = load_active_sessions_unlocked(reviews_dir).unwrap_or_default();
        active.prune_stale();
        let pid = std::process::id();
        // Only this process's own entry: another live tuicr may have the same
        // session open, and its entry is what keeps a delete from running.
        active.sessions.retain(|entry| entry.pid != pid);
        active.sessions.push(ActiveSessionEntry {
            pid,
            slug,
            path,
            last_seen_at: Utc::now(),
        });
        save_active_sessions_unlocked(reviews_dir, &active)
    })
}

pub(crate) fn clear_active_session_for_pid() -> Result<()> {
    let reviews_dir = get_reviews_dir()?;
    clear_active_session_for_pid_in_dir(&reviews_dir)
}

pub(crate) fn clear_active_session_for_pid_in_dir(reviews_dir: &Path) -> Result<()> {
    maybe_migrate(reviews_dir)?;
    with_reviews_dir_lock(reviews_dir, || {
        let mut active = load_active_sessions_unlocked(reviews_dir).unwrap_or_default();
        let pid = std::process::id();
        let before = active.sessions.len();
        active.sessions.retain(|entry| entry.pid != pid);
        if active.sessions.len() == before {
            // Nothing to clear: do not create the registry file just to exit.
            return Ok(());
        }
        save_active_sessions_unlocked(reviews_dir, &active)
    })
}

pub(crate) fn active_session_paths_in_dir(reviews_dir: &Path) -> Result<HashSet<PathBuf>> {
    maybe_migrate(reviews_dir)?;
    let active = load_active_sessions_unlocked(reviews_dir).unwrap_or_default();
    Ok(active
        .sessions
        .into_iter()
        .filter(|entry| entry.is_fresh())
        .map(|entry| normalize_active_path(&entry.path))
        .collect())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ActiveSessionsFile {
    version: String,
    #[serde(default)]
    sessions: Vec<ActiveSessionEntry>,
}

impl Default for ActiveSessionsFile {
    fn default() -> Self {
        Self {
            version: "1.0".to_string(),
            sessions: Vec::new(),
        }
    }
}

impl ActiveSessionsFile {
    fn prune_stale(&mut self) {
        self.sessions.retain(ActiveSessionEntry::is_fresh);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ActiveSessionEntry {
    pid: u32,
    slug: String,
    path: PathBuf,
    last_seen_at: chrono::DateTime<Utc>,
}

impl ActiveSessionEntry {
    fn is_fresh(&self) -> bool {
        let age_is_fresh = Utc::now()
            .signed_duration_since(self.last_seen_at)
            .to_std()
            .is_ok_and(|age| age <= ACTIVE_SESSION_STALE_AFTER);
        age_is_fresh && process_is_running(self.pid) && self.path.exists()
    }
}

#[cfg(unix)]
fn process_is_running(pid: u32) -> bool {
    unsafe extern "C" {
        fn kill(pid: i32, sig: i32) -> i32;
    }

    if pid == 0 {
        return false;
    }

    let result = unsafe { kill(pid as i32, 0) };
    if result == 0 {
        return true;
    }
    std::io::Error::last_os_error().kind() == std::io::ErrorKind::PermissionDenied
}

#[cfg(windows)]
fn process_is_running(pid: u32) -> bool {
    use std::ffi::c_void;

    const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
    const STILL_ACTIVE: u32 = 259;
    const ERROR_ACCESS_DENIED: i32 = 5;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn OpenProcess(access: u32, inherit: i32, pid: u32) -> *mut c_void;
        fn GetExitCodeProcess(process: *mut c_void, code: *mut u32) -> i32;
        fn CloseHandle(handle: *mut c_void) -> i32;
    }

    if pid == 0 {
        return false;
    }
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            return std::io::Error::last_os_error().raw_os_error() == Some(ERROR_ACCESS_DENIED);
        }
        let mut code = 0u32;
        let ok = GetExitCodeProcess(handle, &mut code);
        CloseHandle(handle);
        ok == 0 || code == STILL_ACTIVE
    }
}

#[cfg(not(any(unix, windows)))]
fn process_is_running(_pid: u32) -> bool {
    true
}

fn active_sessions_path(reviews_dir: &Path) -> PathBuf {
    reviews_dir.join(ACTIVE_SESSIONS_FILENAME)
}

fn load_active_sessions_unlocked(reviews_dir: &Path) -> Result<ActiveSessionsFile> {
    let path = active_sessions_path(reviews_dir);
    match fs::read_to_string(&path) {
        Ok(contents) => serde_json::from_str(&contents)
            .map_err(|e| TuicrError::CorruptedSession(format!("active sessions parse error: {e}"))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(ActiveSessionsFile::default()),
        Err(e) => Err(TuicrError::Io(e)),
    }
}

fn save_active_sessions_unlocked(reviews_dir: &Path, active: &ActiveSessionsFile) -> Result<()> {
    let json = serde_json::to_string_pretty(active)?;
    write_atomic(&active_sessions_path(reviews_dir), json.as_bytes())
}

fn normalize_active_path(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

pub(crate) fn normalize_path_for_comparison(path: &Path) -> PathBuf {
    normalize_active_path(path)
}

fn save_session_in_dir_unlocked(session: &ReviewSession, reviews_dir: &Path) -> Result<PathBuf> {
    save_session_at_unlocked(session, reviews_dir, None)
}

fn save_session_at_unlocked(
    session: &ReviewSession,
    reviews_dir: &Path,
    cached: Option<&Path>,
) -> Result<PathBuf> {
    let target = target_for_session(session, reviews_dir)?;
    let full_path = reviews_dir.join(&target.relative);

    if let Some(key) = session.pr_session_key.as_ref() {
        ensure_cached_target(cached, &full_path)?;
        // Never overwrite a file that holds anything but this very session
        // (a hash collision or a corrupt file fails closed).
        if path_exists(&full_path)? {
            verify_occupant(&full_path, key)?;
        }
    }

    if let Some(parent) = full_path.parent() {
        fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(session)?;
    write_atomic(&full_path, json.as_bytes())?;

    upsert_session_manifest(session, reviews_dir, &target)?;

    Ok(full_path)
}

/// Write a local session back to the file it was loaded from, whatever its
/// identity now derives (the `origin` changed, the checkout moved, or the
/// caller named a JSON path directly). The caller must hold the lock. A file
/// inside the reviews dir is re-indexed at that exact path, replacing any
/// entry under a stale slug; a file outside it is never indexed.
fn write_back_local_unlocked(
    session: &ReviewSession,
    reviews_dir: &Path,
    path: &Path,
) -> Result<()> {
    let json = serde_json::to_string_pretty(session)?;
    write_atomic(path, json.as_bytes())?;

    let (Ok(canonical), Ok(root)) = (fs::canonicalize(path), fs::canonicalize(reviews_dir)) else {
        return Ok(());
    };
    let Ok(relative) = canonical.strip_prefix(&root) else {
        return Ok(());
    };
    let Ok(mut target) = target_for_session(session, reviews_dir) else {
        return Ok(());
    };
    target.relative = relative.to_path_buf();
    let mut manifest = manifest_for_update(reviews_dir)?;
    if slot_held_by_other_file(&manifest, session, &target, reviews_dir) {
        // The session's current identity already names a live file (the TUI
        // saved it after `origin` changed). Keep that entry and refresh this
        // file's own entries in place, so neither file leaves the index.
        let fresh =
            manifest::entry_from_session(session, target.relative.clone(), target.anchor.clone());
        for entry in manifest.entries.values_mut().flatten() {
            if entry.path == target.relative {
                let kind = entry.kind.clone();
                *entry = fresh.clone();
                entry.kind = kind;
            }
        }
    } else {
        remove_path_entries(&mut manifest, &target.relative);
        insert_session_entry(&mut manifest, session, &target);
    }
    manifest::save_manifest(reviews_dir, &manifest)
}

/// True when `target`'s manifest slot holds a different, existing session file.
fn slot_held_by_other_file(
    manifest: &manifest::Manifest,
    session: &ReviewSession,
    target: &SessionTarget,
    reviews_dir: &Path,
) -> bool {
    let canonical = fs::canonicalize(&session.repo_path)
        .ok()
        .or_else(|| Some(session.repo_path.clone()));
    manifest
        .entries
        .get(&target.manifest_slug)
        .is_some_and(|bucket| {
            bucket.iter().any(|e| {
                matches!(e.kind, ManifestKind::Local)
                    && e.canonical_repo_path == canonical
                    && e.path != target.relative
                    && reviews_dir.join(&e.path).exists()
            })
        })
}

fn remove_path_entries(manifest: &mut manifest::Manifest, relative: &Path) {
    for bucket in manifest.entries.values_mut() {
        bucket.retain(|entry| entry.path != relative);
    }
    manifest.entries.retain(|_, bucket| !bucket.is_empty());
}

fn insert_session_entry(
    manifest: &mut manifest::Manifest,
    session: &ReviewSession,
    target: &SessionTarget,
) {
    let mut entry =
        manifest::entry_from_session(session, target.relative.clone(), target.anchor.clone());
    if let ManifestKind::Pr { host, .. } = &mut entry.kind {
        host.clone_from(&target.host);
    }
    manifest.upsert(target.manifest_slug.clone(), entry);
}

fn upsert_session_manifest(
    session: &ReviewSession,
    reviews_dir: &Path,
    target: &SessionTarget,
) -> Result<()> {
    let mut manifest = manifest_for_update(reviews_dir)?;
    insert_session_entry(&mut manifest, session, target);
    manifest::save_manifest(reviews_dir, &manifest)
}

/// Read the manifest for a lookup or listing. A corrupt `index.json` is
/// recovered (under the lock) instead of reading as empty.
fn read_manifest(reviews_dir: &Path) -> Result<manifest::Manifest> {
    match manifest::load_manifest(reviews_dir) {
        Ok(_) if index_missing_with_sessions(reviews_dir) => {
            with_reviews_dir_lock(reviews_dir, || manifest_for_update(reviews_dir))
        }
        Err(TuicrError::CorruptedSession(_)) => {
            with_reviews_dir_lock(reviews_dir, || manifest_for_update(reviews_dir))
        }
        other => other,
    }
}

/// Load the manifest for a read-modify-write. The caller must hold the lock.
/// A corrupt `index.json` is moved to `index.json.corrupt` and rebuilt from
/// `sessions/`, so the next save does not drop every other session.
fn manifest_for_update(reviews_dir: &Path) -> Result<manifest::Manifest> {
    match manifest::load_manifest(reviews_dir) {
        Ok(_) if index_missing_with_sessions(reviews_dir) => {
            let rebuilt = rebuild_manifest(reviews_dir)?;
            manifest::save_manifest(reviews_dir, &rebuilt)?;
            Ok(rebuilt)
        }
        Err(TuicrError::CorruptedSession(_)) => {
            let index = reviews_dir.join(MANIFEST_FILENAME);
            let _ = fs::rename(
                &index,
                reviews_dir.join(format!("{MANIFEST_FILENAME}.corrupt")),
            );
            let rebuilt = rebuild_manifest(reviews_dir)?;
            manifest::save_manifest(reviews_dir, &rebuilt)?;
            Ok(rebuilt)
        }
        other => other,
    }
}

/// `index.json` is absent while `sessions/` already holds session files
/// (deleted by hand, or lost): reading it as empty would make the next save
/// overwrite it with a single entry.
fn index_missing_with_sessions(reviews_dir: &Path) -> bool {
    !reviews_dir.join(MANIFEST_FILENAME).exists()
        && fs::read_dir(reviews_dir.join(SESSIONS_DIRNAME)).is_ok_and(|mut d| {
            d.any(|e| e.is_ok_and(|e| e.file_name().to_string_lossy().ends_with(".json")))
        })
}

/// Re-index every readable session file under `sessions/`.
fn rebuild_manifest(reviews_dir: &Path) -> Result<manifest::Manifest> {
    manifest::rebuild_from_files(&reviews_dir.join(SESSIONS_DIRNAME), |path| {
        let session = load_session(path).ok()?;
        let relative = manifest_relative_path(path, reviews_dir);
        let mut target = match session.pr_session_key.as_ref() {
            Some(key) => pr_target_for_path(reviews_dir, key, path)?,
            None => target_for_session(&session, reviews_dir).ok()?,
        };
        target.relative = relative;
        let mut manifest = manifest::Manifest::new();
        insert_session_entry(&mut manifest, &session, &target);
        manifest
            .entries
            .remove(&target.manifest_slug)?
            .pop()
            .map(|entry| (target.manifest_slug, entry))
    })
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().ok_or_else(|| {
        TuicrError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("path has no parent: {}", path.display()),
        ))
    })?;
    fs::create_dir_all(parent)?;

    // Fixed-length name: embedding the target's basename would overflow
    // NAME_MAX for a valid long target.
    let tmp_path = parent.join(format!(".{}.tmp", uuid::Uuid::new_v4().simple()));

    {
        let mut tmp = fs::File::create(&tmp_path)?;
        tmp.write_all(bytes)?;
        tmp.sync_all().ok();
    }
    fs::rename(&tmp_path, path)?;
    Ok(())
}

struct ReviewsDirLock {
    path: PathBuf,
}

impl Drop for ReviewsDirLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn with_reviews_dir_lock<T>(reviews_dir: &Path, f: impl FnOnce() -> Result<T>) -> Result<T> {
    let _lock = acquire_reviews_dir_lock(reviews_dir)?;
    f()
}

fn acquire_reviews_dir_lock(reviews_dir: &Path) -> Result<ReviewsDirLock> {
    fs::create_dir_all(reviews_dir)?;
    let path = reviews_dir.join(STORAGE_LOCK_FILENAME);
    let started = Instant::now();
    loop {
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(mut file) => {
                let _ = writeln!(file, "{} {}", std::process::id(), Utc::now());
                let _ = file.sync_all();
                return Ok(ReviewsDirLock { path });
            }
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
                if remove_stale_reviews_dir_lock(&path)? {
                    continue;
                }
                if started.elapsed() >= STORAGE_LOCK_TIMEOUT {
                    return Err(TuicrError::Io(std::io::Error::new(
                        std::io::ErrorKind::TimedOut,
                        format!(
                            "timed out waiting for review storage lock {}",
                            path.display()
                        ),
                    )));
                }
                std::thread::sleep(Duration::from_millis(25));
            }
            Err(err) => return Err(TuicrError::Io(err)),
        }
    }
}

fn remove_stale_reviews_dir_lock(path: &Path) -> Result<bool> {
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(err) => return Err(TuicrError::Io(err)),
    };
    let lock_age = metadata
        .modified()
        .ok()
        .and_then(|modified| SystemTime::now().duration_since(modified).ok());
    let stale = match read_lock_owner_pid(path) {
        Some(pid) => {
            !process_is_running(pid)
                || lock_age.is_some_and(|age| age >= STORAGE_LOCK_REUSE_GUARD_AFTER)
        }
        None => lock_age.is_some_and(|age| age >= STORAGE_LOCK_STALE_AFTER),
    };
    if !stale {
        return Ok(false);
    }
    match fs::remove_file(path) {
        Ok(()) => Ok(true),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(err) => Err(TuicrError::Io(err)),
    }
}

fn read_lock_owner_pid(path: &Path) -> Option<u32> {
    fs::read_to_string(path)
        .ok()?
        .split_whitespace()
        .next()?
        .parse()
        .ok()
}

/// Load a session JSON file from an absolute path.
pub fn load_session(path: &Path) -> Result<ReviewSession> {
    let contents = fs::read_to_string(path)?;
    serde_json::from_str(&contents).map_err(|e| TuicrError::CorruptedSession(e.to_string()))
}

/// Look up the persisted local session that matches the requested context.
/// Returns `None` if no matching slug is in the manifest, or if a manifest
/// entry exists but belongs to a different canonical checkout (same slug,
/// different path on disk).
pub fn load_latest_session_for_context(
    repo_path: &Path,
    branch_name: Option<&str>,
    head_commit: &str,
    diff_source: SessionDiffSource,
    commit_range: Option<&[String]>,
) -> Result<Option<(PathBuf, ReviewSession)>> {
    // PR sessions are resolved via `load_pr_session`. Mirror the old behavior
    // so callers that pass `PullRequest` here get `None` rather than an error.
    if matches!(diff_source, SessionDiffSource::PullRequest) {
        return Ok(None);
    }

    let reviews_dir = get_reviews_dir()?;
    maybe_migrate(&reviews_dir)?;

    let owner_repo = slug::resolve_owner_repo(repo_path)
        .map_err(|e| TuicrError::CorruptedSession(format!("slug derive: {e}")))?;
    let local = slug::build_local_slug(
        owner_repo,
        branch_name,
        head_commit,
        diff_source,
        commit_range,
    )
    .map_err(|e| TuicrError::CorruptedSession(format!("slug build: {e}")))?;
    let slug = Slug::Local(local);

    let manifest = read_manifest(&reviews_dir)?;
    let canonical = fs::canonicalize(repo_path).unwrap_or_else(|_| repo_path.to_path_buf());
    let Some(entry) = manifest.get_local(&slug.to_string(), &canonical) else {
        return Ok(None);
    };

    let full_path = reviews_dir.join(&entry.path);
    let session = load_session(&full_path)?;
    Ok(Some((full_path, session)))
}

pub(crate) fn list_sessions_for_selector_in_dir(
    reviews_dir: &Path,
    selector: &Path,
) -> Result<Vec<(String, ManifestEntry)>> {
    maybe_migrate(reviews_dir)?;

    let manifest = read_manifest(reviews_dir)?;
    let target = RepoSelector::resolve(selector);
    let mut entries: Vec<_> = manifest
        .iter()
        .filter(|(slug, entry)| target.matches(slug, entry))
        .map(|(slug, entry)| (slug.clone(), entry.clone()))
        .collect();
    entries.sort_by_key(|(_, entry)| Reverse(entry.updated_at));
    Ok(entries)
}

/// How `tuicr review list --repo <value>` interprets its argument.
///
/// A local directory is matched precisely by canonical path (so two checkouts
/// of the same repo stay separate) and contributes an `owner/repo` coordinate
/// derived from its `origin` remote. A non-directory value is parsed directly
/// as a forge coordinate (`owner/repo`, `host/owner/repo`, or a repo/PR URL).
///
/// Local sessions match by canonical path when one is known, otherwise by
/// coordinate. PR sessions have no checkout, so they always match by
/// `owner/repo` — which is what lets `--repo slatedb/slatedb` (or a slatedb
/// checkout) surface its PR sessions.
struct RepoSelector {
    canonical_path: Option<PathBuf>,
    coordinate: Option<RepoCoordinate>,
}

impl RepoSelector {
    fn resolve(selector: &Path) -> Self {
        if selector.is_dir() {
            Self {
                canonical_path: Some(
                    fs::canonicalize(selector).unwrap_or_else(|_| selector.to_path_buf()),
                ),
                coordinate: RepoCoordinate::from_repo_path(selector),
            }
        } else {
            Self {
                canonical_path: None,
                coordinate: RepoCoordinate::parse(&selector.to_string_lossy()),
            }
        }
    }

    fn matches(&self, slug: &str, entry: &ManifestEntry) -> bool {
        match entry.kind {
            // A local session is tied to a physical checkout: match it by
            // canonical path when the selector named one, else by coordinate.
            ManifestKind::Local => match &self.canonical_path {
                Some(canonical) => {
                    entry.canonical_repo_path.as_deref() == Some(canonical.as_path())
                }
                None => self.coordinate_matches(slug),
            },
            // A PR session has no checkout, so it always matches by coordinate
            // — this is what surfaces a repo's PR sessions from its checkout.
            ManifestKind::Pr { .. } => self.coordinate_matches(slug),
        }
    }

    fn coordinate_matches(&self, slug: &str) -> bool {
        let (Some(target), Ok(slug)) = (self.coordinate.as_ref(), slug.parse::<Slug>()) else {
            return false;
        };
        target.matches(&RepoCoordinate::from_slug(&slug))
    }
}

/// List every persisted session (local and PR), newest first. Backs
/// `tuicr review list --all`, which surfaces every session regardless of repo.
pub(crate) fn list_all_sessions_in_dir(reviews_dir: &Path) -> Result<Vec<(String, ManifestEntry)>> {
    maybe_migrate(reviews_dir)?;

    let manifest = read_manifest(reviews_dir)?;
    let mut entries: Vec<_> = manifest
        .iter()
        .map(|(slug, entry)| (slug.clone(), entry.clone()))
        .collect();
    entries.sort_by_key(|(_, entry)| Reverse(entry.updated_at));
    Ok(entries)
}

/// Resolve a PR session's on-disk path from its slug, consulting the manifest.
/// Returns `None` when no PR entry exists for the slug. Unlike
/// [`load_pr_session`], the caller does not need to know the head SHA: the
/// manifest holds a single entry per PR slug (the current head's session).
///
/// A hostless slug reaches only sessions of the forge's default host; an
/// `@host` slug reaches that host's sessions. Legacy hostless entries are
/// trusted only after the file's own host matches (head is not compared).
pub(crate) fn pr_session_path_in_dir(reviews_dir: &Path, slug: &str) -> Result<Option<PathBuf>> {
    maybe_migrate(reviews_dir)?;

    let Ok(Slug::Pr(slug)) = slug.parse::<Slug>() else {
        return Ok(None);
    };
    let Some(host) = slug
        .host
        .clone()
        .or_else(|| default_host(slug.forge).map(str::to_string))
    else {
        return Ok(None);
    };
    let found = find_pr_session(
        reviews_dir,
        &slug,
        &host,
        |_head| true,
        |found| {
            found.repository.kind == slug.forge
                && found.repository.owner == slug.owner
                && found.repository.name == slug.repo
                && found.number == slug.number
                && normalized_host(&found.repository.host) == host
        },
    )?;
    Ok(found.map(|(path, _)| path))
}

/// Look up the persisted PR session for a key. Returns `None` if no entry
/// exists for the slug, or if the manifest's current head differs from the
/// requested head (the old head's file may still be on disk but is not
/// surfaced). When the manifest has no usable entry, the session file the
/// key resolves to is loaded directly (crash between file write and manifest
/// upsert) and re-indexed.
pub fn load_pr_session(key: &PrSessionKey) -> Result<Option<(PathBuf, ReviewSession)>> {
    let reviews_dir = get_reviews_dir()?;
    maybe_migrate(&reviews_dir)?;
    if let Some(found) = load_manifest_pr_session(&reviews_dir, key, |head| head == key.head_sha)? {
        return Ok(Some(found));
    }
    with_reviews_dir_lock(&reviews_dir, || {
        let target = resolve_pr_target(&reviews_dir, key)?;
        let full_path = reviews_dir.join(&target.relative);
        if !path_exists(&full_path)? {
            return Ok(None);
        }
        let session = load_session(&full_path).map_err(|e| {
            TuicrError::CorruptedSession(format!(
                "failed to load PR session {}: {e}",
                full_path.display()
            ))
        })?;
        if !same_full_key(session.pr_session_key.as_ref(), key) {
            return Ok(None);
        }
        upsert_session_manifest(&session, &reviews_dir, &target)?;
        Ok(Some((full_path, session)))
    })
}

/// Look up the persisted session this PR was last reviewed at when that was
/// a different head (the PR was pushed to or rebased since). Returns `None`
/// if no entry exists for the slug, the entry is already at `key`'s head, or
/// it belongs to another repository, host or PR.
pub fn load_previous_head_pr_session(key: &PrSessionKey) -> Result<Option<ReviewSession>> {
    let reviews_dir = get_reviews_dir()?;
    maybe_migrate(&reviews_dir)?;
    Ok(
        load_manifest_pr_session(&reviews_dir, key, |head| head != key.head_sha)?
            .map(|(_path, session)| session),
    )
}

fn load_manifest_pr_session(
    reviews_dir: &Path,
    key: &PrSessionKey,
    head_matches: impl Fn(&str) -> bool,
) -> Result<Option<(PathBuf, ReviewSession)>> {
    let Slug::Pr(slug) = Slug::from(key) else {
        return Ok(None);
    };
    find_pr_session(
        reviews_dir,
        &slug,
        &normalized_host(&key.repository.host),
        head_matches,
        |found| same_pr_identity(found, key),
    )
}

/// Walk the manifest candidates for `slug` and host `host` (explicit host
/// entries first, legacy hostless entries after) and return the first whose
/// file passes `identity`. Legacy files are read through
/// [`read_legacy_holder`]: a corrupt one is skipped, an I/O error propagates.
fn find_pr_session(
    reviews_dir: &Path,
    slug: &PrSlug,
    host: &str,
    head_matches: impl Fn(&str) -> bool,
    identity: impl Fn(&PrSessionKey) -> bool,
) -> Result<Option<(PathBuf, ReviewSession)>> {
    let manifest = read_manifest(reviews_dir)?;
    let qualified = slug.to_string();
    let hostless = slug.hostless().to_string();
    let mut buckets = vec![qualified.as_str()];
    if hostless != qualified {
        buckets.push(hostless.as_str());
    }
    for entry in manifest.pr_candidates(&buckets, Some(host)) {
        let ManifestKind::Pr {
            head_sha,
            host: entry_host,
            ..
        } = &entry.kind
        else {
            continue;
        };
        if !head_matches(head_sha) {
            continue;
        }
        let full_path = reviews_dir.join(&entry.path);
        let session = if entry_host.is_some() {
            load_session(&full_path)?
        } else {
            match read_legacy_holder(&full_path)? {
                Some(session) => session,
                None => continue,
            }
        };
        if session.pr_session_key.as_ref().is_some_and(&identity) {
            return Ok(Some((full_path, session)));
        }
    }
    Ok(None)
}

/// Derive the slug for a session from its embedded fields. Local sessions
/// require resolving the repo's `origin` remote (I/O); PR sessions are
/// derived purely from the embedded `pr_session_key`.
pub fn slug_for_session(session: &ReviewSession) -> Result<Slug> {
    if let Some(key) = session.pr_session_key.as_ref() {
        return Ok(key.into());
    }
    let owner_repo = slug::resolve_owner_repo(&session.repo_path)
        .map_err(|e| TuicrError::CorruptedSession(format!("slug derive: {e}")))?;
    let local = slug::build_local_slug(
        owner_repo,
        session.branch_name.as_deref(),
        &session.base_commit,
        session.diff_source,
        session.commit_range.as_deref(),
    )
    .map_err(|e| TuicrError::CorruptedSession(format!("slug build: {e}")))?;
    Ok(Slug::Local(local))
}

// ---------- Internals ----------

#[cfg(test)]
thread_local! {
    static TEST_REVIEWS_DIR: std::cell::RefCell<Option<PathBuf>> = const {
        std::cell::RefCell::new(None)
    };
}

#[cfg(test)]
pub(crate) fn set_test_reviews_dir(path: Option<PathBuf>) {
    TEST_REVIEWS_DIR.with(|cell| *cell.borrow_mut() = path);
}

pub(crate) fn get_reviews_dir() -> Result<PathBuf> {
    #[cfg(test)]
    {
        // In tests, the reviews dir is a thread-local so that two parallel
        // tests never share state through it. Tests that touch storage and
        // care about isolation set the thread-local via
        // `set_test_reviews_dir`; tests that hit storage incidentally (e.g.,
        // App tests that toggle save markers) fall back to a per-thread
        // temp dir. The real `~/.local/share/tuicr/reviews` is never used in
        // test mode.
        let configured = TEST_REVIEWS_DIR.with(|cell| cell.borrow().clone());
        if let Some(path) = configured {
            fs::create_dir_all(&path)?;
            return Ok(path);
        }
        let thread_id = std::thread::current().id();
        let path = std::env::temp_dir().join(format!(
            "tuicr-test-thread-{:?}-{}",
            thread_id,
            std::process::id()
        ));
        fs::create_dir_all(&path)?;
        Ok(path)
    }

    #[cfg(not(test))]
    {
        let proj_dirs = ProjectDirs::from("", "", "tuicr").ok_or_else(|| {
            TuicrError::Io(std::io::Error::other("Could not determine data directory"))
        })?;

        let data_dir = proj_dirs.data_dir().join("reviews");
        fs::create_dir_all(&data_dir)?;
        Ok(data_dir)
    }
}

/// On first run under the flat layout, move any pre-existing reviews dir
/// aside. The current layout is identified by the presence of the
/// `sessions/` subdirectory; if it's missing but the reviews dir has any
/// other contents (an older flat layout's `*.json` files, a previous tree
/// layout with `local/` and `gh/` subdirs, or a stale manifest), rename the
/// whole directory to `<reviews>.bak1` and start fresh.
fn maybe_migrate(reviews_dir: &Path) -> Result<()> {
    if !reviews_dir.exists() {
        return Ok(());
    }
    let sessions_dir = reviews_dir.join(SESSIONS_DIRNAME);
    if sessions_dir.exists() {
        return Ok(());
    }
    // Current-layout bookkeeping (registry, lock, atomic-write temp files) is
    // not legacy content: a directory holding only that is just fresh.
    let has_legacy_content = fs::read_dir(reviews_dir)?.any(|entry| {
        entry.map_or(true, |entry| {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            !(name == ACTIVE_SESSIONS_FILENAME
                || name == STORAGE_LOCK_FILENAME
                || (name.starts_with('.') && name.ends_with(".tmp"))
                || name == format!("{MANIFEST_FILENAME}.tmp"))
        })
    });
    if !has_legacy_content {
        fs::create_dir_all(&sessions_dir)?;
        return Ok(());
    }

    let parent = reviews_dir
        .parent()
        .ok_or_else(|| TuicrError::Io(std::io::Error::other("reviews dir has no parent")))?;
    let stem = reviews_dir
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| TuicrError::Io(std::io::Error::other("reviews dir has no name")))?;
    let mut backup = parent.join(format!("{stem}.bak1"));
    let mut suffix = 1u32;
    while backup.exists() {
        suffix += 1;
        backup = parent.join(format!("{stem}.bak{suffix}"));
    }

    fs::rename(reviews_dir, &backup)?;
    fs::create_dir_all(&sessions_dir)?;
    eprintln!(
        "[tuicr] migrating reviews to new layout; previous reviews moved to {}",
        backup.display()
    );
    Ok(())
}

/// Compute the relative path of a local session's JSON file under `reviews/`.
///
/// Files live in a single flat `sessions/` directory; their name is the FNV-1a
/// hash of `slug || canonical_repo_path`, so the same logical session always
/// lands at the same path and no manifest lookup is needed to construct it.
/// Two checkouts of the same repo produce distinct hashes because their
/// canonical paths differ. PR sessions use [`resolve_pr_target`] instead.
fn relative_path_for_local(slug: &Slug, session: &ReviewSession) -> PathBuf {
    let mut hasher = Fnv1aHasher::new();
    hasher.write(b"local|");
    hasher.write(slug.to_string().as_bytes());
    hasher.write(b"|");
    let canonical =
        fs::canonicalize(&session.repo_path).unwrap_or_else(|_| session.repo_path.clone());
    let normalized = canonical.to_string_lossy().to_string();
    let normalized = if cfg!(windows) {
        normalized.to_lowercase()
    } else {
        normalized
    };
    hasher.write(normalized.as_bytes());
    hashed_session_path(hasher)
}

fn hashed_session_path(hasher: Fnv1aHasher) -> PathBuf {
    let hex = format!("{:016x}", hasher.finish());
    PathBuf::from(SESSIONS_DIRNAME).join(format!("{hex}.json"))
}

/// Where a session lives and how it is indexed.
#[derive(Debug, Clone)]
pub(crate) struct SessionTarget {
    /// File path relative to the reviews dir.
    pub(crate) relative: PathBuf,
    pub(crate) manifest_slug: String,
    pub(crate) anchor: String,
    /// Manifest `host` field (`None` for local and legacy sessions).
    pub(crate) host: Option<String>,
}

fn target_for_session(session: &ReviewSession, reviews_dir: &Path) -> Result<SessionTarget> {
    if let Some(key) = session.pr_session_key.as_ref() {
        return resolve_pr_target(reviews_dir, key);
    }
    let slug = slug_for_session(session)?;
    Ok(SessionTarget {
        relative: relative_path_for_local(&slug, session),
        manifest_slug: slug.to_string(),
        anchor: manifest_anchor_for(&slug),
        host: None,
    })
}

/// New (host-aware) file for a PR key: `pr2|<forge>|<host>|<owner>/<repo>|<n>|<head>`.
fn new_pr_target(key: &PrSessionKey) -> SessionTarget {
    let host = normalized_host(&key.repository.host);
    let mut hasher = Fnv1aHasher::new();
    let input = format!(
        "pr2|{}|{}|{}/{}|{}|{}",
        forge_code(key.repository.kind),
        host,
        key.repository.owner,
        key.repository.name,
        key.number,
        key.head_sha
    );
    hasher.write(input.as_bytes());
    let slug = Slug::from(key);
    SessionTarget {
        relative: hashed_session_path(hasher),
        manifest_slug: slug.to_string(),
        anchor: manifest_anchor_for(&slug),
        host: Some(host),
    }
}

/// Legacy (hostless) file for a PR key: `pr|<hostless slug>|<head>`. Kept
/// bit-for-bit so existing sessions are found where they always were.
pub(crate) fn legacy_pr_target(key: &PrSessionKey) -> SessionTarget {
    let Slug::Pr(slug) = Slug::from(key) else {
        unreachable!("a PR key always derives a PR slug")
    };
    let hostless = slug.hostless().to_string();
    let mut hasher = Fnv1aHasher::new();
    hasher.write(b"pr|");
    hasher.write(hostless.as_bytes());
    hasher.write(b"|");
    hasher.write(key.head_sha.as_bytes());
    SessionTarget {
        relative: hashed_session_path(hasher),
        manifest_slug: hostless,
        anchor: format!("pr/{}", key.number),
        host: None,
    }
}

/// The one reader of legacy files. A missing file or one that does not parse
/// as a session holds nothing of ours (`None`): never modified, never deleted,
/// the new file is used. Any other I/O error propagates, so a transient
/// failure cannot fork the session into a second file.
fn read_legacy_holder(path: &Path) -> Result<Option<ReviewSession>> {
    let contents = match fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(TuicrError::Io(e)),
    };
    Ok(serde_json::from_str::<ReviewSession>(&contents).ok())
}

fn path_exists(path: &Path) -> Result<bool> {
    match fs::metadata(path) {
        Ok(_) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(TuicrError::Io(e)),
    }
}

/// Full key equality: identity (normalized host) plus head.
fn same_full_key(found: Option<&PrSessionKey>, key: &PrSessionKey) -> bool {
    found.is_some_and(|found| same_pr_identity(found, key) && found.head_sha == key.head_sha)
}

/// The only place deciding which file holds a PR session. The new
/// host-aware file wins when present; otherwise a readable legacy file with
/// an equal key keeps being used (same path, same manifest slug); otherwise
/// the new file is created. Legacy bytes are never touched by this choice.
fn resolve_pr_target(reviews_dir: &Path, key: &PrSessionKey) -> Result<SessionTarget> {
    let new = new_pr_target(key);
    if path_exists(&reviews_dir.join(&new.relative))? {
        return Ok(new);
    }
    let legacy = legacy_pr_target(key);
    match read_legacy_holder(&reviews_dir.join(&legacy.relative))? {
        Some(holder) if same_full_key(holder.pr_session_key.as_ref(), key) => Ok(legacy),
        _ => Ok(new),
    }
}

/// The identity tuple of the file at `path`, when it is one of the two files
/// `key` can live in. Pure (no I/O beyond canonicalizing).
fn pr_target_for_path(
    reviews_dir: &Path,
    key: &PrSessionKey,
    path: &Path,
) -> Option<SessionTarget> {
    [new_pr_target(key), legacy_pr_target(key)]
        .into_iter()
        .find(|target| same_file(&reviews_dir.join(&target.relative), path))
}

fn same_file(a: &Path, b: &Path) -> bool {
    a == b || normalize_active_path(a) == normalize_active_path(b)
}

/// The existing file at `path` must hold exactly this session.
fn verify_occupant(path: &Path, key: &PrSessionKey) -> Result<()> {
    let occupant = load_session(path)?;
    if same_full_key(occupant.pr_session_key.as_ref(), key) {
        Ok(())
    } else {
        Err(TuicrError::CorruptedSession(format!(
            "{} holds a different review session",
            path.display()
        )))
    }
}

/// A cached path that exists must still be the file resolution names.
fn ensure_cached_target(cached: Option<&Path>, resolved: &Path) -> Result<()> {
    match cached {
        Some(cached) if path_exists(cached)? && !same_file(cached, resolved) => {
            Err(TuicrError::SessionTargetChanged(format!(
                "{} is now stored at {}",
                cached.display(),
                resolved.display()
            )))
        }
        _ => Ok(()),
    }
}

fn manifest_anchor_for(slug: &Slug) -> String {
    match slug {
        Slug::Local(local) => local.anchor.to_string(),
        Slug::Pr(pr) => format!("pr/{}", pr.number),
    }
}

#[allow(dead_code)]
pub(crate) fn manifest_path_for(reviews_dir: &Path) -> PathBuf {
    reviews_dir.join(MANIFEST_FILENAME)
}

// ---------- Tests ----------

#[cfg(test)]
mod pr_identity_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::forge::traits::{ForgeRepository, PrSessionKey};
    use crate::model::{Comment, CommentType, FileStatus};
    use crate::persistence::manifest::Manifest;
    use std::path::PathBuf;

    struct TestReviewsDirGuard {
        path: PathBuf,
    }

    impl Drop for TestReviewsDirGuard {
        fn drop(&mut self) {
            set_test_reviews_dir(None);
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    fn with_test_reviews_dir() -> TestReviewsDirGuard {
        let path =
            std::env::temp_dir().join(format!("tuicr-reviews-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&path).unwrap();
        set_test_reviews_dir(Some(path.clone()));
        TestReviewsDirGuard { path }
    }

    fn make_repo() -> PathBuf {
        let repo = std::env::temp_dir().join(format!("tuicr-repo-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&repo).unwrap();
        repo
    }

    fn make_local_session(
        repo_path: PathBuf,
        base_commit: &str,
        branch_name: Option<&str>,
        diff_source: SessionDiffSource,
        commit_range: Option<Vec<String>>,
    ) -> ReviewSession {
        let mut s = ReviewSession::new(
            repo_path,
            base_commit.to_string(),
            branch_name.map(|b| b.to_string()),
            diff_source,
        );
        s.commit_range = commit_range;
        s.add_file(PathBuf::from("src/main.rs"), FileStatus::Modified, 0);
        s
    }

    fn make_pr_key(number: u64, head_sha: &str) -> PrSessionKey {
        PrSessionKey::new(
            ForgeRepository::github("github.com", "agavra", "tuicr"),
            number,
            head_sha.to_string(),
        )
    }

    fn make_azure_pr_key(number: u64, head_sha: &str) -> PrSessionKey {
        PrSessionKey::new(
            ForgeRepository::azure("dev.azure.com", "myorg/myproject", "myrepo"),
            number,
            head_sha.to_string(),
        )
    }

    /// A repo in a nested GitLab group: the owner is the whole `org/team`
    /// namespace, not just the group it sits directly under.
    fn make_gitlab_pr_key(number: u64, head_sha: &str) -> PrSessionKey {
        PrSessionKey::new(
            ForgeRepository::gitlab("gitlab.com", "org/team", "svc"),
            number,
            head_sha.to_string(),
        )
    }

    /// A checkout with a real `origin`, so the selector's coordinate comes from
    /// the remote URL rather than the directory name.
    fn make_repo_with_origin(url: &str) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let repo = git2::Repository::init(dir.path()).unwrap();
        repo.remote("origin", url).unwrap();
        dir
    }

    fn make_pr_session(key: &PrSessionKey) -> ReviewSession {
        let mut s = ReviewSession::new(
            PathBuf::from(format!(
                "forge:{}/{}/{}",
                key.repository.host, key.repository.owner, key.repository.name
            )),
            key.head_sha.clone(),
            Some("reviews".to_string()),
            SessionDiffSource::PullRequest,
        );
        s.pr_session_key = Some(key.clone());
        s
    }

    // ---- Save/load round trips ----

    #[test]
    fn should_roundtrip_local_session() {
        let _g = with_test_reviews_dir();
        let repo = make_repo();
        let session = make_local_session(
            repo.clone(),
            "abc1234",
            Some("main"),
            SessionDiffSource::WorkingTree,
            None,
        );
        let path = save_session(&session).unwrap();
        let loaded = load_session(&path).unwrap();
        assert_eq!(session.id, loaded.id);
        assert_eq!(session.base_commit, loaded.base_commit);
    }

    #[test]
    fn should_keep_an_empty_session_a_live_tuicr_has_open() {
        // An agent emptying a session through `tuicr review delete` must not
        // pull the file out from under the TUI polling it (#769).
        let _g = with_test_reviews_dir();
        let reviews_dir = get_reviews_dir().unwrap();
        let session = make_local_session(
            make_repo(),
            "abc1234",
            Some("main"),
            SessionDiffSource::WorkingTree,
            None,
        );
        let path = save_session(&session).unwrap();
        mark_session_active_in_dir(&session, &path, &reviews_dir).unwrap();

        assert!(!delete_session_if_empty_and_inactive_in_dir(&path, &reviews_dir).unwrap());
        assert!(path.exists(), "the open session must survive");

        // Once the TUI is gone, an empty session is cleaned up as before.
        clear_active_session_for_pid_in_dir(&reviews_dir).unwrap();
        assert!(delete_session_if_empty_and_inactive_in_dir(&path, &reviews_dir).unwrap());
        assert!(!path.exists());
    }

    #[test]
    fn should_delete_empty_session_and_manifest_entry() {
        let _g = with_test_reviews_dir();
        let repo = make_repo();
        let session = make_local_session(
            repo.clone(),
            "abc1234",
            Some("main"),
            SessionDiffSource::WorkingTree,
            None,
        );
        let path = save_session(&session).unwrap();
        assert!(path.exists());

        assert!(delete_session_if_empty(&path).unwrap());

        assert!(!path.exists());
        let sessions = load_latest_session_for_context(
            &repo,
            Some("main"),
            "abc1234",
            SessionDiffSource::WorkingTree,
            None,
        )
        .unwrap();
        assert!(sessions.is_none());
    }

    #[test]
    fn should_delete_reviewed_only_session_unconditionally() {
        let _g = with_test_reviews_dir();
        let repo = make_repo();
        let mut session = make_local_session(
            repo.clone(),
            "abc1234",
            Some("main"),
            SessionDiffSource::WorkingTree,
            None,
        );
        // Mark the file reviewed: `delete_session_if_empty` would refuse this,
        // but `delete_session` discards it anyway.
        session
            .get_file_mut(&PathBuf::from("src/main.rs"))
            .unwrap()
            .reviewed = true;
        let path = save_session(&session).unwrap();
        assert!(path.exists());

        assert!(!delete_session_if_empty(&path).unwrap());
        assert!(path.exists(), "reviewed session must survive empty-delete");

        assert!(delete_session_unless_commented(&path).unwrap());
        assert!(!path.exists(), "reviewed-only session should be discarded");
    }

    #[test]
    fn should_report_false_when_deleting_missing_session() {
        let _g = with_test_reviews_dir();
        let reviews_dir = get_reviews_dir().unwrap();
        let missing = reviews_dir.join("does-not-exist.json");
        assert!(!delete_session_unless_commented(&missing).unwrap());
    }

    #[test]
    fn should_prune_manifest_entry_when_deleting_a_session_whose_file_is_gone() {
        let _g = with_test_reviews_dir();
        let reviews_dir = get_reviews_dir().unwrap();
        let session = make_local_session(
            make_repo(),
            "abc1234",
            Some("main"),
            SessionDiffSource::WorkingTree,
            None,
        );
        let path = save_session(&session).unwrap();
        fs::remove_file(&path).unwrap();
        assert_eq!(list_all_sessions_in_dir(&reviews_dir).unwrap().len(), 1);

        let outcome = delete_inactive_session_in_dir(&path, &reviews_dir).unwrap();

        assert_eq!(outcome, DeleteOutcome::Missing);
        assert!(
            list_all_sessions_in_dir(&reviews_dir).unwrap().is_empty(),
            "a stale manifest entry would resurrect the row on the next listing"
        );
    }

    #[test]
    fn should_refuse_to_delete_a_session_marked_active_after_listing() {
        let _g = with_test_reviews_dir();
        let reviews_dir = get_reviews_dir().unwrap();
        let session = make_local_session(
            make_repo(),
            "abc1234",
            Some("main"),
            SessionDiffSource::WorkingTree,
            None,
        );
        let path = save_session(&session).unwrap();
        // Registered only now, as if another tuicr opened it after a caller
        // took its listing snapshot.
        mark_session_active_in_dir(&session, &path, &reviews_dir).unwrap();

        let outcome = delete_inactive_session_in_dir(&path, &reviews_dir).unwrap();

        assert_eq!(outcome, DeleteOutcome::Active);
        assert!(path.exists());

        clear_active_session_for_pid_in_dir(&reviews_dir).unwrap();
        let outcome = delete_inactive_session_in_dir(&path, &reviews_dir).unwrap();
        assert_eq!(outcome, DeleteOutcome::Deleted);
        assert!(!path.exists());
    }

    #[test]
    fn should_mark_and_clear_active_session() {
        let _g = with_test_reviews_dir();
        let reviews_dir = get_reviews_dir().unwrap();
        let repo = make_repo();
        let session = make_local_session(
            repo,
            "abc1234",
            Some("main"),
            SessionDiffSource::WorkingTree,
            None,
        );
        let path = save_session(&session).unwrap();

        mark_session_active_in_dir(&session, &path, &reviews_dir).unwrap();
        let active = active_session_paths_in_dir(&reviews_dir).unwrap();
        assert!(active.contains(&normalize_path_for_comparison(&path)));

        clear_active_session_for_pid_in_dir(&reviews_dir).unwrap();
        let active = active_session_paths_in_dir(&reviews_dir).unwrap();
        assert!(!active.contains(&normalize_path_for_comparison(&path)));
    }

    #[test]
    fn should_recover_stale_reviews_dir_lock() {
        let _g = with_test_reviews_dir();
        let reviews_dir = get_reviews_dir().unwrap();
        fs::write(reviews_dir.join(STORAGE_LOCK_FILENAME), "stale lock").unwrap();
        let repo = make_repo();
        let session = make_local_session(
            repo,
            "abc1234",
            Some("main"),
            SessionDiffSource::WorkingTree,
            None,
        );

        let path = save_session(&session).unwrap();

        assert!(path.exists());
        assert!(!reviews_dir.join(STORAGE_LOCK_FILENAME).exists());
    }

    #[test]
    fn should_keep_session_with_comments_when_deleting_if_empty() {
        let _g = with_test_reviews_dir();
        let repo = make_repo();
        let mut session = make_local_session(
            repo,
            "abc1234",
            Some("main"),
            SessionDiffSource::WorkingTree,
            None,
        );
        session.review_comments.push(Comment::new(
            "keep me".to_string(),
            CommentType::from_id("note"),
            None,
        ));
        let path = save_session(&session).unwrap();

        assert!(!delete_session_if_empty(&path).unwrap());

        assert!(path.exists());
    }

    #[test]
    fn should_keep_session_with_reviewed_files_when_deleting_if_empty() {
        let _g = with_test_reviews_dir();
        let repo = make_repo();
        let mut session = make_local_session(
            repo,
            "abc1234",
            Some("main"),
            SessionDiffSource::WorkingTree,
            None,
        );
        session
            .get_file_mut(&PathBuf::from("src/main.rs"))
            .unwrap()
            .reviewed = true;
        let path = save_session(&session).unwrap();

        assert!(!delete_session_if_empty(&path).unwrap());

        assert!(path.exists());
    }

    #[test]
    fn should_keep_session_with_reviewed_hunks_when_deleting_if_empty() {
        let _g = with_test_reviews_dir();
        let repo = make_repo();
        let mut session = make_local_session(
            repo,
            "abc1234",
            Some("main"),
            SessionDiffSource::WorkingTree,
            None,
        );
        session
            .get_file_mut(&PathBuf::from("src/main.rs"))
            .unwrap()
            .toggle_hunk_reviewed("stable-hunk".to_string());
        let path = save_session(&session).unwrap();

        assert!(!delete_session_if_empty(&path).unwrap());

        assert!(path.exists());
    }

    #[test]
    fn should_save_under_flat_sessions_dir_for_local() {
        let _g = with_test_reviews_dir();
        let repo = make_repo();
        let session = make_local_session(
            repo.clone(),
            "abc1234",
            Some("main"),
            SessionDiffSource::WorkingTree,
            None,
        );
        let path = save_session(&session).unwrap();
        let display = path.display().to_string();
        assert!(
            display.contains("/sessions/"),
            "expected /sessions/ in {display}"
        );
        let name = path.file_name().unwrap().to_str().unwrap();
        assert!(
            name.len() == 21 && name.ends_with(".json"),
            "expected 16-hex-char filename, got {name}"
        );
    }

    #[test]
    fn should_save_under_flat_sessions_dir_for_pr() {
        let _g = with_test_reviews_dir();
        let key = make_pr_key(125, "abcdef0123456789");
        let session = make_pr_session(&key);
        let path = save_session(&session).unwrap();
        let display = path.display().to_string();
        assert!(
            display.contains("/sessions/"),
            "expected /sessions/ in {display}"
        );
        let name = path.file_name().unwrap().to_str().unwrap();
        assert!(
            name.len() == 21 && name.ends_with(".json"),
            "expected 16-hex-char filename, got {name}"
        );
    }

    #[test]
    fn should_produce_distinct_paths_for_different_pr_heads() {
        let _g = with_test_reviews_dir();
        let key_a = make_pr_key(125, "abcdef0123456789");
        let key_b = make_pr_key(125, "9999999999999999");
        let path_a = save_session(&make_pr_session(&key_a)).unwrap();
        let path_b = save_session(&make_pr_session(&key_b)).unwrap();
        assert_ne!(
            path_a, path_b,
            "PR sessions with different heads must hash to different files"
        );
    }

    #[test]
    fn should_update_manifest_on_save() {
        let _g = with_test_reviews_dir();
        let repo = make_repo();
        let session = make_local_session(
            repo.clone(),
            "abc1234",
            Some("main"),
            SessionDiffSource::WorkingTree,
            None,
        );
        save_session(&session).unwrap();

        let reviews_dir = get_reviews_dir().unwrap();
        let manifest = manifest::load_manifest(&reviews_dir).unwrap();
        let slug = slug_for_session(&session).unwrap();
        let canonical = fs::canonicalize(&repo).unwrap_or(repo);
        let entry = manifest
            .get_local(&slug.to_string(), &canonical)
            .expect("entry exists");
        assert!(matches!(entry.kind, ManifestKind::Local));
        assert_eq!(entry.display.file_count, 1);
    }

    // ---- Lookup ----

    #[test]
    fn should_return_none_for_unknown_context() {
        let _g = with_test_reviews_dir();
        let repo = make_repo();
        let loaded = load_latest_session_for_context(
            &repo,
            Some("main"),
            "head",
            SessionDiffSource::WorkingTree,
            None,
        )
        .unwrap();
        assert!(loaded.is_none());
    }

    #[test]
    fn should_load_session_when_head_matches_on_branched_worktree() {
        let _g = with_test_reviews_dir();
        let repo = make_repo();
        let session = make_local_session(
            repo.clone(),
            "abc1234",
            Some("main"),
            SessionDiffSource::WorkingTree,
            None,
        );
        save_session(&session).unwrap();

        let (_, loaded) = load_latest_session_for_context(
            &repo,
            Some("main"),
            "abc1234",
            SessionDiffSource::WorkingTree,
            None,
        )
        .unwrap()
        .expect("session for current head should load");
        assert_eq!(loaded.id, session.id);
    }

    #[test]
    fn should_not_load_worktree_session_after_head_advances() {
        // Regression for #378. A `tuicr -w` session is keyed by (branch, head)
        // so that committing on the same branch starts a fresh session
        // instead of resurrecting comments tied to the previous HEAD.
        let _g = with_test_reviews_dir();
        let repo = make_repo();
        let session = make_local_session(
            repo.clone(),
            "abc1234",
            Some("main"),
            SessionDiffSource::WorkingTree,
            None,
        );
        save_session(&session).unwrap();

        let loaded = load_latest_session_for_context(
            &repo,
            Some("main"),
            "new-head",
            SessionDiffSource::WorkingTree,
            None,
        )
        .unwrap();
        assert!(
            loaded.is_none(),
            "advancing HEAD must yield a fresh session, not the previous one"
        );
    }

    #[test]
    fn should_ignore_sessions_with_different_diff_source() {
        let _g = with_test_reviews_dir();
        let repo = make_repo();
        let session = make_local_session(
            repo.clone(),
            "abc1234",
            Some("main"),
            SessionDiffSource::WorkingTree,
            None,
        );
        save_session(&session).unwrap();

        let loaded = load_latest_session_for_context(
            &repo,
            Some("main"),
            "head",
            SessionDiffSource::Staged,
            None,
        )
        .unwrap();
        assert!(loaded.is_none());
    }

    #[test]
    fn should_require_commit_range_match() {
        let _g = with_test_reviews_dir();
        let repo = make_repo();
        let range_a = vec!["c1".to_string(), "c0".to_string()];
        let range_b = vec!["c3".to_string(), "c2".to_string()];

        let session_a = make_local_session(
            repo.clone(),
            "c1",
            Some("main"),
            SessionDiffSource::CommitRange,
            Some(range_a.clone()),
        );
        save_session(&session_a).unwrap();

        let session_b = make_local_session(
            repo.clone(),
            "c3",
            Some("main"),
            SessionDiffSource::CommitRange,
            Some(range_b.clone()),
        );
        save_session(&session_b).unwrap();

        let (_, loaded_a) = load_latest_session_for_context(
            &repo,
            Some("main"),
            "c1",
            SessionDiffSource::CommitRange,
            Some(range_a.as_slice()),
        )
        .unwrap()
        .unwrap();
        assert_eq!(loaded_a.id, session_a.id);

        let (_, loaded_b) = load_latest_session_for_context(
            &repo,
            Some("main"),
            "c3",
            SessionDiffSource::CommitRange,
            Some(range_b.as_slice()),
        )
        .unwrap()
        .unwrap();
        assert_eq!(loaded_b.id, session_b.id);
    }

    #[test]
    fn should_disambiguate_two_checkouts_with_same_repo_name() {
        let _g = with_test_reviews_dir();
        let base = std::env::temp_dir().join(format!("tuicr-multi-{}", uuid::Uuid::new_v4()));
        let repo_a = base.join("a").join("same-repo");
        let repo_b = base.join("b").join("same-repo");
        fs::create_dir_all(&repo_a).unwrap();
        fs::create_dir_all(&repo_b).unwrap();

        // Same branch *and* same HEAD in both checkouts: this is the
        // collision the manifest's canonical_repo_path must break, since
        // the slug itself is identical.
        let session_a = make_local_session(
            repo_a.clone(),
            "head-x",
            Some("main"),
            SessionDiffSource::WorkingTree,
            None,
        );
        let session_b = make_local_session(
            repo_b.clone(),
            "head-x",
            Some("main"),
            SessionDiffSource::WorkingTree,
            None,
        );
        save_session(&session_a).unwrap();
        save_session(&session_b).unwrap();

        // Same slug for both (no remote, fallback to dir name; same HEAD)
        // but canonical_repo_path disambiguates.
        let slug = slug_for_session(&session_a).unwrap();
        let slug_b = slug_for_session(&session_b).unwrap();
        assert_eq!(slug.to_string(), slug_b.to_string());

        let (_, loaded_a) = load_latest_session_for_context(
            &repo_a,
            Some("main"),
            "head-x",
            SessionDiffSource::WorkingTree,
            None,
        )
        .unwrap()
        .expect("repo_a lookup");
        assert_eq!(loaded_a.id, session_a.id);

        let (_, loaded_b) = load_latest_session_for_context(
            &repo_b,
            Some("main"),
            "head-x",
            SessionDiffSource::WorkingTree,
            None,
        )
        .unwrap()
        .expect("repo_b lookup");
        assert_eq!(loaded_b.id, session_b.id);

        let _ = fs::remove_dir_all(&base);
    }

    // ---- PR sessions ----

    #[test]
    fn should_roundtrip_pr_session() {
        let _g = with_test_reviews_dir();
        let key = make_pr_key(125, "abcdef0123456789");
        let session = make_pr_session(&key);
        let path = save_session(&session).unwrap();
        let (loaded_path, loaded) = load_pr_session(&key).unwrap().unwrap();
        assert_eq!(loaded_path, path);
        assert_eq!(loaded.pr_session_key.as_ref(), Some(&key));
    }

    #[test]
    fn should_roundtrip_pr_description_reviewed_and_keep_session_on_cleanup() {
        let _g = with_test_reviews_dir();
        let key = make_pr_key(126, "abcdef0123456789");
        let mut session = make_pr_session(&key);
        session.pr_description_reviewed = true;
        let path = save_session(&session).unwrap();

        let (_, loaded) = load_pr_session(&key).unwrap().unwrap();
        assert!(loaded.pr_description_reviewed);
        // A reviewed description is review progress, not an empty session.
        assert!(!delete_session_if_empty(&path).unwrap());
    }

    #[test]
    fn should_return_none_when_head_changes_for_pr() {
        let _g = with_test_reviews_dir();
        let old_key = make_pr_key(125, "abcdef0123456789");
        let session = make_pr_session(&old_key);
        save_session(&session).unwrap();

        let new_key = make_pr_key(125, "9999999999999999");
        let loaded = load_pr_session(&new_key).unwrap();
        assert!(loaded.is_none());
    }

    #[test]
    fn should_separate_pr_sessions_by_number() {
        let _g = with_test_reviews_dir();
        let key_a = make_pr_key(125, "abcdef0123456789");
        let key_b = make_pr_key(148, "abcdef0123456789");
        save_session(&make_pr_session(&key_a)).unwrap();
        save_session(&make_pr_session(&key_b)).unwrap();

        let loaded_a = load_pr_session(&key_a).unwrap().unwrap();
        let loaded_b = load_pr_session(&key_b).unwrap().unwrap();
        assert_eq!(loaded_a.1.pr_session_key.as_ref(), Some(&key_a));
        assert_eq!(loaded_b.1.pr_session_key.as_ref(), Some(&key_b));
    }

    #[test]
    fn should_skip_pr_files_in_local_context_lookup() {
        let _g = with_test_reviews_dir();
        let key = make_pr_key(125, "abcdef0123456789");
        save_session(&make_pr_session(&key)).unwrap();

        let repo = make_repo();
        let loaded = load_latest_session_for_context(
            &repo,
            Some("main"),
            "head",
            SessionDiffSource::WorkingTree,
            None,
        )
        .unwrap();
        assert!(loaded.is_none());
    }

    // ---- Repo-coordinate selector ----

    #[test]
    fn should_find_pr_session_by_repo_coordinate() {
        let _g = with_test_reviews_dir();
        let reviews_dir = get_reviews_dir().unwrap();
        let key = make_pr_key(125, "abcdef0123456789");
        save_session(&make_pr_session(&key)).unwrap();

        let listed =
            list_sessions_for_selector_in_dir(&reviews_dir, Path::new("agavra/tuicr")).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].0, "gh:agavra/tuicr/pr/125");
    }

    #[test]
    fn should_not_match_pr_session_for_unrelated_coordinate() {
        let _g = with_test_reviews_dir();
        let reviews_dir = get_reviews_dir().unwrap();
        save_session(&make_pr_session(&make_pr_key(125, "abcdef0123456789"))).unwrap();

        let listed =
            list_sessions_for_selector_in_dir(&reviews_dir, Path::new("other/project")).unwrap();
        assert!(listed.is_empty());
    }

    #[test]
    fn should_match_local_and_pr_sessions_sharing_repo_name_by_coordinate() {
        let _g = with_test_reviews_dir();
        let reviews_dir = get_reviews_dir().unwrap();

        // A local checkout whose directory name is the repo name (no origin,
        // so the local slug carries no owner) plus a PR session for the same
        // repo. A `--repo agavra/tuicr` coordinate should surface both.
        let base = std::env::temp_dir().join(format!("tuicr-coord-{}", uuid::Uuid::new_v4()));
        let local_repo = base.join("tuicr");
        fs::create_dir_all(&local_repo).unwrap();
        save_session(&make_local_session(
            local_repo.clone(),
            "abc1234",
            Some("main"),
            SessionDiffSource::WorkingTree,
            None,
        ))
        .unwrap();
        save_session(&make_pr_session(&make_pr_key(125, "abcdef0123456789"))).unwrap();

        let listed =
            list_sessions_for_selector_in_dir(&reviews_dir, Path::new("agavra/tuicr")).unwrap();
        let slugs: Vec<&str> = listed.iter().map(|(slug, _)| slug.as_str()).collect();
        assert!(slugs.iter().any(|s| s.starts_with("tuicr@main/worktree")));
        assert!(slugs.contains(&"gh:agavra/tuicr/pr/125"));

        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn should_find_azure_pr_session_by_repo_url_coordinate() {
        let _g = with_test_reviews_dir();
        let reviews_dir = get_reviews_dir().unwrap();
        let key = make_azure_pr_key(123, "abcdef0123456789");
        save_session(&make_pr_session(&key)).unwrap();

        let listed = list_sessions_for_selector_in_dir(
            &reviews_dir,
            Path::new("dev.azure.com/myorg/myproject/_git/myrepo"),
        )
        .unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].0, "az:myorg/myproject/myrepo/pr/123");
    }

    #[test]
    fn should_surface_azure_pr_session_from_its_checkout() {
        // Regression for #639: the checkout's coordinate used to come out as
        // owner `_git`, which never matched the `myorg/myproject` owner the PR
        // slug carries, so `review list` in an Azure checkout showed only the
        // local session.
        let _g = with_test_reviews_dir();
        let reviews_dir = get_reviews_dir().unwrap();
        let checkout =
            make_repo_with_origin("https://myorg@dev.azure.com/myorg/myproject/_git/myrepo");

        save_session(&make_local_session(
            checkout.path().to_path_buf(),
            "abc1234",
            Some("main"),
            SessionDiffSource::WorkingTree,
            None,
        ))
        .unwrap();
        let key = make_azure_pr_key(123, "abcdef0123456789");
        save_session(&make_pr_session(&key)).unwrap();

        let listed = list_sessions_for_selector_in_dir(&reviews_dir, checkout.path()).unwrap();
        let slugs: Vec<&str> = listed.iter().map(|(slug, _)| slug.as_str()).collect();
        assert!(
            slugs.contains(&"az:myorg/myproject/myrepo/pr/123"),
            "expected the PR session, got {slugs:?}"
        );
        assert!(
            slugs
                .iter()
                .any(|s| s.starts_with("myorg/myproject/myrepo@main/worktree")),
            "expected the local session under an org/project owner, got {slugs:?}"
        );
    }

    #[test]
    fn should_find_gitlab_subgroup_pr_session_by_repo_url_coordinate() {
        let _g = with_test_reviews_dir();
        let reviews_dir = get_reviews_dir().unwrap();
        let key = make_gitlab_pr_key(123, "abcdef0123456789");
        save_session(&make_pr_session(&key)).unwrap();

        let listed = list_sessions_for_selector_in_dir(
            &reviews_dir,
            Path::new("https://gitlab.com/org/team/svc"),
        )
        .unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].0, "gl:org/team/svc/pr/123");
    }

    #[test]
    fn should_surface_gitlab_subgroup_pr_session_from_its_checkout() {
        let _g = with_test_reviews_dir();
        let reviews_dir = get_reviews_dir().unwrap();

        let checkout = make_repo_with_origin("git@gitlab.com:org/team/svc.git");
        save_session(&make_local_session(
            checkout.path().to_path_buf(),
            "abc1234",
            Some("main"),
            SessionDiffSource::WorkingTree,
            None,
        ))
        .unwrap();
        let key = make_gitlab_pr_key(123, "abcdef0123456789");
        save_session(&make_pr_session(&key)).unwrap();

        let listed = list_sessions_for_selector_in_dir(&reviews_dir, checkout.path()).unwrap();
        let slugs: Vec<&str> = listed.iter().map(|(slug, _)| slug.as_str()).collect();
        assert!(
            slugs.contains(&"gl:org/team/svc/pr/123"),
            "expected the PR session, got {slugs:?}"
        );
        assert!(
            slugs
                .iter()
                .any(|s| s.starts_with("org/team/svc@main/worktree")),
            "expected the local session under the full namespace, got {slugs:?}"
        );
    }

    #[test]
    fn should_match_local_session_by_canonical_path_in_path_mode() {
        let _g = with_test_reviews_dir();
        let reviews_dir = get_reviews_dir().unwrap();
        let repo = make_repo();
        save_session(&make_local_session(
            repo.clone(),
            "abc1234",
            Some("main"),
            SessionDiffSource::WorkingTree,
            None,
        ))
        .unwrap();

        let listed = list_sessions_for_selector_in_dir(&reviews_dir, &repo).unwrap();
        assert_eq!(listed.len(), 1);
        assert!(matches!(listed[0].1.kind, ManifestKind::Local));
    }

    // ---- Branch sanitization in slug ----

    #[test]
    fn should_sanitize_branch_slashes_in_slug() {
        let _g = with_test_reviews_dir();
        let repo = make_repo();
        let session = make_local_session(
            repo.clone(),
            "abc1234",
            Some("feature/login"),
            SessionDiffSource::WorkingTree,
            None,
        );
        save_session(&session).unwrap();

        let slug = slug_for_session(&session).unwrap();
        assert!(
            slug.to_string().contains("@feature-login/worktree"),
            "branch slash not sanitized in {slug}"
        );
    }

    // ---- Migration ----

    #[test]
    fn should_migrate_pre_flat_layout_on_first_run() {
        let _g = with_test_reviews_dir();
        let reviews_dir = get_reviews_dir().unwrap();

        // Pre-flat artifact: a top-level *.json from the original flat
        // layout, or a tree-layout subdir from the intermediate v2 layout.
        let stray = reviews_dir.join("old_session.json");
        fs::write(&stray, "{\"legacy\":true}").unwrap();
        let tree_subdir = reviews_dir.join("local").join("abcd");
        fs::create_dir_all(&tree_subdir).unwrap();
        fs::write(tree_subdir.join("foo.json"), "{}").unwrap();

        let session = make_local_session(
            make_repo(),
            "abc1234",
            Some("main"),
            SessionDiffSource::WorkingTree,
            None,
        );
        save_session(&session).unwrap();

        assert!(
            !stray.exists(),
            "pre-flat artifacts should have moved during migration"
        );
        assert!(reviews_dir.join(SESSIONS_DIRNAME).exists());

        let parent = reviews_dir.parent().unwrap();
        let backup_exists = fs::read_dir(parent)
            .unwrap()
            .flatten()
            .any(|e| e.file_name().to_string_lossy().contains(".bak"));
        assert!(backup_exists, "expected a .bak backup dir under {parent:?}");
    }

    #[test]
    fn should_not_migrate_when_sessions_dir_already_present() {
        let _g = with_test_reviews_dir();
        let reviews_dir = get_reviews_dir().unwrap();
        fs::create_dir_all(reviews_dir.join(SESSIONS_DIRNAME)).unwrap();
        let manifest = Manifest::new();
        manifest::save_manifest(&reviews_dir, &manifest).unwrap();

        let stray = reviews_dir.join("stray.json");
        fs::write(&stray, "{}").unwrap();

        let session = make_local_session(
            make_repo(),
            "abc1234",
            Some("main"),
            SessionDiffSource::WorkingTree,
            None,
        );
        save_session(&session).unwrap();
        assert!(
            stray.exists(),
            "stray .json must survive when sessions/ already exists"
        );
    }

    #[test]
    fn should_not_migrate_when_reviews_dir_is_empty() {
        let _g = with_test_reviews_dir();
        let reviews_dir = get_reviews_dir().unwrap();
        // Reviews dir exists but is empty: no migration trigger.
        maybe_migrate(&reviews_dir).unwrap();
        assert!(reviews_dir.exists());
        let stem = reviews_dir
            .file_name()
            .unwrap()
            .to_string_lossy()
            .to_string();
        let parent = reviews_dir.parent().unwrap();
        let our_backup_exists = fs::read_dir(parent).unwrap().flatten().any(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            name.starts_with(&stem) && name.contains(".bak")
        });
        assert!(
            !our_backup_exists,
            "should not migrate when reviews dir is empty"
        );
    }

    // ---- Write-back, manifest recovery, registry ----

    fn manifest_paths(reviews_dir: &Path) -> Vec<PathBuf> {
        manifest::load_manifest(reviews_dir)
            .unwrap()
            .iter()
            .map(|(_, entry)| entry.path.clone())
            .collect()
    }

    fn add_note(session: &mut ReviewSession) -> Result<()> {
        session.review_comments.push(Comment::new(
            "note".to_string(),
            CommentType::from_id("note"),
            None,
        ));
        Ok(())
    }

    #[test]
    fn should_write_back_to_the_referenced_file_after_origin_changes() {
        let _g = with_test_reviews_dir();
        let reviews_dir = get_reviews_dir().unwrap();
        let checkout = make_repo_with_origin("https://github.com/a/one.git");
        let session = make_local_session(
            checkout.path().to_path_buf(),
            "abc1234",
            Some("main"),
            SessionDiffSource::WorkingTree,
            None,
        );
        let path = save_session(&session).unwrap();
        git2::Repository::open(checkout.path())
            .unwrap()
            .remote_set_url("origin", "https://github.com/b/two.git")
            .unwrap();

        update_session_in_dir(&path, &reviews_dir, add_note).unwrap();

        assert_eq!(load_session(&path).unwrap().review_comments.len(), 1);
        let entries = list_all_sessions_in_dir(&reviews_dir).unwrap();
        assert_eq!(entries.len(), 1, "one file, one entry: {entries:?}");
        assert_eq!(entries[0].1.display.comment_count, 1);
        assert!(entries[0].0.starts_with("b/two"));
    }

    #[test]
    fn should_write_back_to_an_explicit_path_outside_the_reviews_dir() {
        let _g = with_test_reviews_dir();
        let reviews_dir = get_reviews_dir().unwrap();
        let session = make_local_session(
            make_repo(),
            "abc1234",
            Some("main"),
            SessionDiffSource::WorkingTree,
            None,
        );
        let managed = save_session(&session).unwrap();
        let external =
            std::env::temp_dir().join(format!("tuicr-ext-{}.json", uuid::Uuid::new_v4()));
        fs::copy(&managed, &external).unwrap();

        update_session_in_dir(&external, &reviews_dir, add_note).unwrap();

        assert_eq!(load_session(&external).unwrap().review_comments.len(), 1);
        assert!(load_session(&managed).unwrap().review_comments.is_empty());
        assert_eq!(manifest_paths(&reviews_dir).len(), 1);
        assert!(manifest_paths(&reviews_dir)[0].starts_with(SESSIONS_DIRNAME));
        let _ = fs::remove_file(&external);
    }

    #[test]
    fn should_edit_a_session_file_with_a_name_near_name_max() {
        let _g = with_test_reviews_dir();
        let reviews_dir = get_reviews_dir().unwrap();
        let session = make_local_session(
            make_repo(),
            "abc1234",
            Some("main"),
            SessionDiffSource::WorkingTree,
            None,
        );
        let managed = save_session(&session).unwrap();
        let outside = tempfile::tempdir().unwrap();
        let long = outside.path().join(format!("{}.json", "x".repeat(230)));
        assert_eq!(long.file_name().unwrap().len(), 235);
        fs::copy(&managed, &long).unwrap();

        update_session_in_dir(&long, &reviews_dir, add_note).unwrap();

        assert_eq!(load_session(&long).unwrap().review_comments.len(), 1);
    }

    #[test]
    fn should_not_index_an_external_file_whose_slot_is_absent() {
        let _g = with_test_reviews_dir();
        let reviews_dir = get_reviews_dir().unwrap();
        let checkout = make_repo_with_origin("https://github.com/a/one.git");
        let session = make_local_session(
            checkout.path().to_path_buf(),
            "abc1234",
            Some("main"),
            SessionDiffSource::WorkingTree,
            None,
        );
        let managed = save_session(&session).unwrap();
        let outside = tempfile::tempdir().unwrap();
        let external = outside.path().join("custom.json");
        fs::copy(&managed, &external).unwrap();
        git2::Repository::open(checkout.path())
            .unwrap()
            .remote_set_url("origin", "https://github.com/b/two.git")
            .unwrap();

        update_session_in_dir(&external, &reviews_dir, add_note).unwrap();

        assert_eq!(load_session(&external).unwrap().review_comments.len(), 1);
        let listed = manifest_paths(&reviews_dir);
        assert_eq!(listed.len(), 1);
        assert!(listed.iter().all(|p| p.starts_with(SESSIONS_DIRNAME)));
    }

    #[test]
    fn should_not_create_the_registry_when_an_unregistered_pid_exits() {
        let _g = with_test_reviews_dir();
        let reviews_dir = get_reviews_dir().unwrap();
        assert!(!reviews_dir.join(ACTIVE_SESSIONS_FILENAME).exists());

        clear_active_session_for_pid_in_dir(&reviews_dir).unwrap();

        assert!(!reviews_dir.join(ACTIVE_SESSIONS_FILENAME).exists());
    }

    #[test]
    fn should_not_let_another_checkouts_slot_block_a_rekey() {
        let _g = with_test_reviews_dir();
        let reviews_dir = get_reviews_dir().unwrap();
        let a = make_repo_with_origin("https://github.com/a/one.git");
        let b = make_repo_with_origin("https://github.com/b/two.git");
        let make = |p: &Path| {
            make_local_session(
                p.to_path_buf(),
                "abc1234",
                Some("main"),
                SessionDiffSource::WorkingTree,
                None,
            )
        };
        let a_path = save_session(&make(a.path())).unwrap();
        let b_path = save_session(&make(b.path())).unwrap();
        git2::Repository::open(a.path())
            .unwrap()
            .remote_set_url("origin", "https://github.com/b/two.git")
            .unwrap();

        update_session_in_dir(&a_path, &reviews_dir, add_note).unwrap();

        let listed = list_all_sessions_in_dir(&reviews_dir).unwrap();
        assert_eq!(listed.len(), 2);
        assert!(listed.iter().all(|(slug, _)| slug.starts_with("b/two")));
        assert!(
            listed
                .iter()
                .any(|(_, e)| e.path == manifest_relative_path(&b_path, &reviews_dir))
        );
    }

    #[test]
    fn should_not_let_a_missing_slot_file_block_a_rekey() {
        let _g = with_test_reviews_dir();
        let reviews_dir = get_reviews_dir().unwrap();
        let repo = make_repo_with_origin("https://github.com/a/one.git");
        let make = || {
            make_local_session(
                repo.path().to_path_buf(),
                "abc1234",
                Some("main"),
                SessionDiffSource::WorkingTree,
                None,
            )
        };
        let old = save_session(&make()).unwrap();
        git2::Repository::open(repo.path())
            .unwrap()
            .remote_set_url("origin", "https://github.com/b/two.git")
            .unwrap();
        let current = save_session(&make()).unwrap();
        fs::remove_file(current).unwrap();

        update_session_in_dir(&old, &reviews_dir, add_note).unwrap();

        let listed = list_all_sessions_in_dir(&reviews_dir).unwrap();
        assert_eq!(listed.len(), 1);
        assert!(listed[0].0.starts_with("b/two"));
        assert_eq!(listed[0].1.path, manifest_relative_path(&old, &reviews_dir));
    }

    #[test]
    fn should_prune_the_manifest_by_path_when_the_slug_drifted() {
        let _g = with_test_reviews_dir();
        let reviews_dir = get_reviews_dir().unwrap();
        let checkout = make_repo_with_origin("https://github.com/a/one.git");
        let session = make_local_session(
            checkout.path().to_path_buf(),
            "abc1234",
            Some("main"),
            SessionDiffSource::WorkingTree,
            None,
        );
        let path = save_session(&session).unwrap();
        git2::Repository::open(checkout.path())
            .unwrap()
            .remote_set_url("origin", "https://github.com/b/two.git")
            .unwrap();

        assert!(delete_session_in_dir(&path, &reviews_dir).unwrap());

        assert!(manifest_paths(&reviews_dir).is_empty());
    }

    #[test]
    fn should_rebuild_a_corrupt_manifest_instead_of_dropping_sessions() {
        let _g = with_test_reviews_dir();
        let reviews_dir = get_reviews_dir().unwrap();
        for commit in ["aaa1111", "bbb2222"] {
            let session = make_local_session(
                make_repo(),
                commit,
                Some("main"),
                SessionDiffSource::WorkingTree,
                None,
            );
            save_session(&session).unwrap();
        }
        fs::write(reviews_dir.join(MANIFEST_FILENAME), "{corrupt").unwrap();

        assert_eq!(list_all_sessions_in_dir(&reviews_dir).unwrap().len(), 2);

        fs::write(reviews_dir.join(MANIFEST_FILENAME), "{corrupt").unwrap();
        let third = make_local_session(
            make_repo(),
            "ccc3333",
            Some("main"),
            SessionDiffSource::WorkingTree,
            None,
        );
        save_session(&third).unwrap();

        assert_eq!(list_all_sessions_in_dir(&reviews_dir).unwrap().len(), 3);
        assert!(reviews_dir.join("index.json.corrupt").exists());
        assert_eq!(
            manifest::load_manifest(&reviews_dir).unwrap().version,
            manifest::MANIFEST_VERSION
        );
    }

    #[cfg(unix)]
    #[test]
    fn should_keep_another_live_owner_when_a_second_tui_exits() {
        let _g = with_test_reviews_dir();
        let reviews_dir = get_reviews_dir().unwrap();
        let session = make_local_session(
            make_repo(),
            "abc1234",
            Some("main"),
            SessionDiffSource::WorkingTree,
            None,
        );
        let path = save_session(&session).unwrap();
        let mut first_owner = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .unwrap();
        let active = ActiveSessionsFile {
            sessions: vec![ActiveSessionEntry {
                pid: first_owner.id(),
                slug: "x".to_string(),
                path: normalize_active_path(&path),
                last_seen_at: Utc::now(),
            }],
            ..ActiveSessionsFile::default()
        };
        save_active_sessions_unlocked(&reviews_dir, &active).unwrap();

        mark_session_active_in_dir(&session, &path, &reviews_dir).unwrap();
        clear_active_session_for_pid_in_dir(&reviews_dir).unwrap();

        let outcome = delete_inactive_session_in_dir(&path, &reviews_dir).unwrap();
        let _ = first_owner.kill();
        let _ = first_owner.wait();
        assert_eq!(outcome, DeleteOutcome::Active);
        assert!(path.exists());
    }

    #[test]
    fn should_not_treat_registry_bookkeeping_as_a_legacy_layout() {
        // A dir an older tuicr left holding only bookkeeping, with no sessions/.
        let names = [
            ACTIVE_SESSIONS_FILENAME.to_string(),
            STORAGE_LOCK_FILENAME.to_string(),
            format!(".{}.tmp", uuid::Uuid::new_v4().simple()),
            format!("{MANIFEST_FILENAME}.tmp"),
        ];
        for name in names {
            let _g = with_test_reviews_dir();
            let reviews_dir = get_reviews_dir().unwrap();
            let _ = fs::remove_dir_all(reviews_dir.join(SESSIONS_DIRNAME));
            fs::write(reviews_dir.join(&name), "{}").unwrap();

            maybe_migrate(&reviews_dir).unwrap();

            assert!(reviews_dir.join(&name).exists(), "{name} was moved aside");
            assert!(reviews_dir.join(SESSIONS_DIRNAME).is_dir());
            let backup = reviews_dir.with_file_name(format!(
                "{}.bak1",
                reviews_dir.file_name().unwrap().to_string_lossy()
            ));
            assert!(!backup.exists(), "{name}: no spurious migration backup");
        }
    }

    #[test]
    fn should_keep_the_index_of_a_corrupt_manifest_when_deleting_a_session() {
        let _g = with_test_reviews_dir();
        let reviews_dir = get_reviews_dir().unwrap();
        let mut paths = Vec::new();
        for commit in ["aaa1111", "bbb2222"] {
            let session = make_local_session(
                make_repo(),
                commit,
                Some("main"),
                SessionDiffSource::WorkingTree,
                None,
            );
            paths.push(save_session(&session).unwrap());
        }
        fs::write(reviews_dir.join(MANIFEST_FILENAME), "{corrupt").unwrap();

        assert!(delete_session_in_dir(&paths[0], &reviews_dir).unwrap());

        let left = manifest_paths(&reviews_dir);
        assert_eq!(left.len(), 1, "the other session stays indexed: {left:?}");
        assert!(reviews_dir.join("index.json.corrupt").exists());
    }

    #[test]
    fn should_rebuild_a_missing_manifest_when_sessions_exist() {
        let _g = with_test_reviews_dir();
        let reviews_dir = get_reviews_dir().unwrap();
        for commit in ["aaa1111", "bbb2222"] {
            let session = make_local_session(
                make_repo(),
                commit,
                Some("main"),
                SessionDiffSource::WorkingTree,
                None,
            );
            save_session(&session).unwrap();
        }
        fs::remove_file(reviews_dir.join(MANIFEST_FILENAME)).unwrap();

        assert_eq!(list_all_sessions_in_dir(&reviews_dir).unwrap().len(), 2);

        fs::remove_file(reviews_dir.join(MANIFEST_FILENAME)).unwrap();
        let third = make_local_session(
            make_repo(),
            "ccc3333",
            Some("main"),
            SessionDiffSource::WorkingTree,
            None,
        );
        save_session(&third).unwrap();

        assert_eq!(list_all_sessions_in_dir(&reviews_dir).unwrap().len(), 3);
    }

    #[test]
    fn should_not_evict_the_current_identity_file_on_write_back_to_an_old_file() {
        let _g = with_test_reviews_dir();
        let reviews_dir = get_reviews_dir().unwrap();
        let checkout = make_repo_with_origin("https://github.com/a/one.git");
        let make = || {
            make_local_session(
                checkout.path().to_path_buf(),
                "abc1234",
                Some("main"),
                SessionDiffSource::WorkingTree,
                None,
            )
        };
        let old_path = save_session(&make()).unwrap();
        git2::Repository::open(checkout.path())
            .unwrap()
            .remote_set_url("origin", "https://github.com/b/two.git")
            .unwrap();
        let new_path = save_session(&make()).unwrap();
        assert_ne!(old_path, new_path);

        update_session_in_dir(&old_path, &reviews_dir, add_note).unwrap();

        let listed = manifest_paths(&reviews_dir);
        let rel = |p: &Path| manifest_relative_path(p, &reviews_dir);
        assert!(
            listed.contains(&rel(&new_path)),
            "current file dropped: {listed:?}"
        );
        assert!(
            listed.contains(&rel(&old_path)),
            "edited file dropped: {listed:?}"
        );
        let counts: Vec<_> = list_all_sessions_in_dir(&reviews_dir)
            .unwrap()
            .into_iter()
            .map(|(_, e)| (e.path.clone(), e.display.comment_count))
            .collect();
        assert!(counts.contains(&(rel(&old_path), 1)), "{counts:?}");
        assert!(counts.contains(&(rel(&new_path), 0)), "{counts:?}");
    }
}
