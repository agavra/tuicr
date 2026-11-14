//! Host-aware identity of NEW PR sessions (T-cases of the p_pr_identity_store
//! design). Legacy fixtures are written as raw JSON at the real legacy hash.

use super::*;
use crate::forge::traits::{ForgeRepository, PrSessionKey};
use crate::model::{Comment, CommentType};
use crate::persistence::manifest::Manifest;

struct Dir {
    path: PathBuf,
}

impl Drop for Dir {
    fn drop(&mut self) {
        set_test_reviews_dir(None);
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn reviews() -> Dir {
    let path = std::env::temp_dir().join(format!("tuicr-identity-test-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&path).unwrap();
    set_test_reviews_dir(Some(path.clone()));
    Dir { path }
}

fn key_on(host: &str, head: &str) -> PrSessionKey {
    PrSessionKey::new(ForgeRepository::github(host, "o", "r"), 42, head)
}

fn session_with(key: &PrSessionKey, note: &str) -> ReviewSession {
    let mut s = ReviewSession::new(
        PathBuf::from(format!("forge:{}/o/r", key.repository.host)),
        key.head_sha.clone(),
        Some("reviews".to_string()),
        SessionDiffSource::PullRequest,
    );
    s.pr_session_key = Some(key.clone());
    s.review_comments.push(Comment::new(
        note.to_string(),
        CommentType::from_id("note"),
        None,
    ));
    s
}

fn dir_of(g: &Dir) -> PathBuf {
    g.path.clone()
}

fn legacy_file(g: &Dir, key: &PrSessionKey) -> PathBuf {
    g.path.join(legacy_pr_target(key).relative)
}

fn new_file(g: &Dir, key: &PrSessionKey) -> PathBuf {
    g.path.join(new_pr_target(key).relative)
}

/// What an old binary leaves behind: a session at the hostless hash plus a
/// hostless manifest entry.
fn write_legacy(g: &Dir, key: &PrSessionKey, note: &str) -> PathBuf {
    let target = legacy_pr_target(key);
    let path = g.path.join(&target.relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let session = session_with(key, note);
    fs::write(&path, serde_json::to_string_pretty(&session).unwrap()).unwrap();
    let entry = manifest::entry_from_session(&session, target.relative, target.anchor);
    let mut m = manifest::load_manifest(&g.path).unwrap_or_default();
    m.upsert(target.manifest_slug, entry);
    manifest::save_manifest(&g.path, &m).unwrap();
    path
}

fn manifest_of(g: &Dir) -> Manifest {
    manifest::load_manifest(&g.path).unwrap()
}

fn hosts_in(m: &Manifest, bucket: &str) -> Vec<Option<String>> {
    m.entries
        .get(bucket)
        .map(|b| {
            b.iter()
                .map(|e| match &e.kind {
                    ManifestKind::Pr { host, .. } => host.clone(),
                    ManifestKind::Local => panic!("local entry"),
                })
                .collect()
        })
        .unwrap_or_default()
}

fn file_count(g: &Dir) -> usize {
    fs::read_dir(g.path.join(SESSIONS_DIRNAME))
        .map(|d| d.count())
        .unwrap_or(0)
}

const HEAD: &str = "abc";
const GHE: &str = "ghe.internal";

#[test]
fn should_pin_new_and_legacy_hash_inputs() {
    let g = reviews();
    let key = key_on(GHE, HEAD);
    assert!(legacy_file(&g, &key).ends_with("sessions/34408a3899b90ad6.json"));
    assert!(new_file(&g, &key).ends_with("sessions/0f4734d2a724a5a3.json"));
    assert!(new_file(&g, &key_on("github.com", HEAD)).ends_with("sessions/3e72591c6bebb1b0.json"));
}

// T1
#[test]
fn should_give_two_hosts_two_files_and_two_buckets() {
    let g = reviews();
    let ghe = key_on(GHE, HEAD);
    let public = key_on("github.com", HEAD);
    let a = save_session(&session_with(&ghe, "ghe")).unwrap();
    let b = save_session(&session_with(&public, "public")).unwrap();
    assert_ne!(a, b);
    assert_eq!(a, new_file(&g, &ghe));
    assert_eq!(b, new_file(&g, &public));
    let m = manifest_of(&g);
    assert_eq!(
        hosts_in(&m, "gh:o/r/pr/42@ghe.internal"),
        vec![Some(GHE.into())]
    );
    assert_eq!(
        hosts_in(&m, "gh:o/r/pr/42"),
        vec![Some("github.com".into())]
    );
    assert_eq!(load_session(&a).unwrap().review_comments[0].content, "ghe");
}

// T2
#[test]
fn should_not_overwrite_legacy_ghe_file_with_public_save() {
    let g = reviews();
    let legacy = write_legacy(&g, &key_on(GHE, HEAD), "ghe draft");
    let before = fs::read(&legacy).unwrap();
    let public = key_on("github.com", HEAD);
    let saved = save_session(&session_with(&public, "public")).unwrap();
    assert_eq!(saved, new_file(&g, &public));
    assert_ne!(saved, legacy);
    assert_eq!(fs::read(&legacy).unwrap(), before);
    let m = manifest_of(&g);
    assert_eq!(
        hosts_in(&m, "gh:o/r/pr/42"),
        vec![None, Some("github.com".into())]
    );
}

// T3
#[test]
fn should_not_overwrite_legacy_public_file_with_ghe_save() {
    let g = reviews();
    let legacy = write_legacy(&g, &key_on("github.com", HEAD), "public draft");
    let before = fs::read(&legacy).unwrap();
    let ghe = key_on(GHE, HEAD);
    let saved = save_session(&session_with(&ghe, "ghe")).unwrap();
    assert_eq!(saved, new_file(&g, &ghe));
    assert_eq!(fs::read(&legacy).unwrap(), before);
}

// T4
#[test]
fn should_reopen_legacy_ghe_session_at_its_own_path_and_slug() {
    let g = reviews();
    let key = key_on(GHE, HEAD);
    let legacy = write_legacy(&g, &key, "old");
    assert_eq!(session_path(&session_with(&key, "x")).unwrap(), legacy);
    let (path, found) = load_pr_session(&key).unwrap().unwrap();
    assert_eq!(path, legacy);
    assert_eq!(found.review_comments[0].content, "old");
    save_session(&session_with(&key, "newer")).unwrap();
    assert_eq!(file_count(&g), 1);
    let m = manifest_of(&g);
    assert_eq!(hosts_in(&m, "gh:o/r/pr/42"), vec![None]);
    assert_eq!(m.len(), 1);
}

// T5
#[test]
fn should_keep_cached_path_stable_over_saves_and_ref_updates() {
    let g = reviews();
    for legacy in [false, true] {
        let key = key_on(GHE, if legacy { "legacyhead" } else { "newhead" });
        let first = if legacy {
            write_legacy(&g, &key, "old")
        } else {
            save_session(&session_with(&key, "one")).unwrap()
        };
        let second = save_session(&session_with(&key, "two")).unwrap();
        let third = save_session(&session_with(&key, "three")).unwrap();
        assert_eq!(first, second);
        assert_eq!(second, third);
        let (updated, ()) = update_session_in_dir(&first, &dir_of(&g), |s| {
            s.review_comments.clear();
            Ok(())
        })
        .unwrap();
        assert!(updated.review_comments.is_empty());
        assert!(load_session(&first).unwrap().review_comments.is_empty());
    }
}

// T6
#[test]
fn should_keep_active_registry_entry_across_saves_and_refuse_delete() {
    let g = reviews();
    let key = key_on(GHE, HEAD);
    let path = write_legacy(&g, &key, "old");
    let session = session_with(&key, "x");
    mark_session_active_in_dir(&session, &path, &g.path).unwrap();
    save_session(&session_with(&key, "again")).unwrap();
    assert!(
        active_session_paths_in_dir(&g.path)
            .unwrap()
            .contains(&normalize_active_path(&path))
    );
    assert_eq!(
        delete_inactive_session_in_dir(&path, &g.path).unwrap(),
        DeleteOutcome::Active
    );
    assert!(path.exists());
}

// T7
#[test]
fn should_never_touch_corrupt_legacy_file_and_error_on_corrupt_new_file() {
    let g = reviews();
    let key = key_on(GHE, HEAD);
    let legacy = legacy_file(&g, &key);
    fs::create_dir_all(legacy.parent().unwrap()).unwrap();
    fs::write(&legacy, "{not json").unwrap();
    let saved = save_session(&session_with(&key, "mine")).unwrap();
    assert_eq!(saved, new_file(&g, &key));
    assert_eq!(fs::read_to_string(&legacy).unwrap(), "{not json");

    // a corrupt NEW file is never overwritten
    let other = key_on(GHE, "otherhead");
    let n = new_file(&g, &other);
    fs::write(&n, "garbage").unwrap();
    assert!(save_session(&session_with(&other, "x")).is_err());
    assert_eq!(fs::read_to_string(&n).unwrap(), "garbage");
}

// T8
#[test]
fn should_let_legacy_and_host_entries_coexist_and_prefer_host_entry() {
    let g = reviews();
    let legacy_key = key_on(GHE, "a");
    write_legacy(&g, &legacy_key, "ghe");
    save_session(&session_with(&key_on("github.com", "b"), "pub")).unwrap();
    let mut m = manifest_of(&g);
    assert_eq!(
        hosts_in(&m, "gh:o/r/pr/42"),
        vec![None, Some("github.com".into())]
    );
    let host_entry = m.get_pr_for("gh:o/r/pr/42", Some("github.com")).unwrap();
    assert!(matches!(
        &host_entry.kind,
        ManifestKind::Pr { host: Some(_), .. }
    ));
    // another host only finds the legacy entry (callers verify content)
    let fallback = m.get_pr_for("gh:o/r/pr/42", Some(GHE)).unwrap();
    assert!(matches!(
        &fallback.kind,
        ManifestKind::Pr { host: None, .. }
    ));
    // a legacy write replaces the legacy entry only
    let legacy_entry = fallback.clone();
    let mut newer = legacy_entry.clone();
    newer.path = PathBuf::from("sessions/other.json");
    m.upsert("gh:o/r/pr/42".into(), newer);
    assert_eq!(m.entries["gh:o/r/pr/42"].len(), 2);
    assert!(m.get_pr_for("gh:o/r/pr/42", Some("github.com")).is_some());
}

// T10
#[test]
fn should_not_surface_previous_head_of_another_host() {
    let g = reviews();
    save_session(&session_with(&key_on(GHE, "old"), "ghe")).unwrap();
    assert!(
        load_previous_head_pr_session(&key_on("github.com", "new"))
            .unwrap()
            .is_none()
    );
    assert!(
        load_previous_head_pr_session(&key_on(GHE, "new"))
            .unwrap()
            .is_some()
    );
    drop(g);

    // legacy bucket fallback: host mismatch is rejected, match is found
    let _g = reviews();
    write_legacy(&_g, &key_on(GHE, "old"), "ghe");
    assert!(
        load_previous_head_pr_session(&key_on("github.com", "new"))
            .unwrap()
            .is_none()
    );
    assert!(
        load_previous_head_pr_session(&key_on(GHE, "new"))
            .unwrap()
            .is_some()
    );
}

// T11
#[test]
fn should_resolve_selectors_and_markers_without_crossing_hosts() {
    let g = reviews();
    let dir = dir_of(&g);
    // new sessions: marker == manifest key
    let public = key_on("github.com", HEAD);
    let ghe = key_on(GHE, HEAD);
    let p = save_session(&session_with(&public, "p")).unwrap();
    let h = save_session(&session_with(&ghe, "h")).unwrap();
    assert_eq!(
        session_marker_slug(&session_with(&public, "x")).unwrap(),
        "gh:o/r/pr/42"
    );
    assert_eq!(
        session_marker_slug(&session_with(&ghe, "x")).unwrap(),
        "gh:o/r/pr/42@ghe.internal"
    );
    assert_eq!(
        pr_session_path_in_dir(&dir, "gh:o/r/pr/42").unwrap(),
        Some(p)
    );
    assert_eq!(
        pr_session_path_in_dir(&dir, "gh:o/r/pr/42@ghe.internal").unwrap(),
        Some(h)
    );
    drop(g);

    // legacy default-host session: marker stays hostless and resolves
    let g = reviews();
    let dir = dir_of(&g);
    let legacy_pub = write_legacy(&g, &public, "p");
    assert_eq!(
        session_marker_slug(&session_with(&public, "x")).unwrap(),
        "gh:o/r/pr/42"
    );
    assert_eq!(
        pr_session_path_in_dir(&dir, "gh:o/r/pr/42").unwrap(),
        Some(legacy_pub)
    );
    drop(g);

    // legacy non-default session: host-qualified marker resolves to its file,
    // a hostless selector never does (no Some entry present at all)
    let g = reviews();
    let dir = dir_of(&g);
    let legacy_ghe = write_legacy(&g, &ghe, "h");
    let marker = session_marker_slug(&session_with(&ghe, "x")).unwrap();
    assert_eq!(marker, "gh:o/r/pr/42@ghe.internal");
    assert_eq!(
        pr_session_path_in_dir(&dir, &marker).unwrap(),
        Some(legacy_ghe)
    );
    assert_eq!(pr_session_path_in_dir(&dir, "gh:o/r/pr/42").unwrap(), None);
}

// T12
#[test]
fn should_leave_local_session_identity_untouched() {
    let g = reviews();
    let repo = std::env::temp_dir().join(format!("tuicr-repo-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&repo).unwrap();
    let session = ReviewSession::new(
        repo,
        "abc1234".to_string(),
        Some("main".to_string()),
        SessionDiffSource::WorkingTree,
    );
    let slug = slug_for_session(&session).unwrap();
    let target = target_for_session(&session, &g.path).unwrap();
    assert_eq!(target.relative, relative_path_for_local(&slug, &session));
    assert_eq!(target.manifest_slug, slug.to_string());
    assert_eq!(target.host, None);
    let path = save_session(&session).unwrap();
    assert_eq!(path, g.path.join(target.relative));
    assert_eq!(session_marker_slug(&session).unwrap(), slug.to_string());
}

// T13
#[test]
fn should_find_session_file_when_crash_left_it_unindexed() {
    let g = reviews();
    let key = key_on(GHE, HEAD);
    let saved = save_session(&session_with(&key, "n")).unwrap();
    fs::remove_file(g.path.join(MANIFEST_FILENAME)).unwrap();
    let (path, found) = load_pr_session(&key).unwrap().unwrap();
    assert_eq!(path, saved);
    assert_eq!(found.review_comments[0].content, "n");
    // reindexed
    assert!(
        manifest_of(&g)
            .get_pr_for("gh:o/r/pr/42@ghe.internal", Some(GHE))
            .is_some()
    );

    // same for a legacy file without manifest entry
    let legacy_key = key_on("ghe2.internal", HEAD);
    let legacy = write_legacy(&g, &legacy_key, "old");
    fs::remove_file(g.path.join(MANIFEST_FILENAME)).unwrap();
    let (path, _) = load_pr_session(&legacy_key).unwrap().unwrap();
    assert_eq!(path, legacy);
}

// T14
#[test]
fn should_not_write_to_repaired_legacy_ref_once_new_file_exists() {
    let g = reviews();
    let key = key_on(GHE, HEAD);
    let legacy = legacy_file(&g, &key);
    fs::create_dir_all(legacy.parent().unwrap()).unwrap();
    fs::write(&legacy, "{broken").unwrap();
    let n = save_session(&session_with(&key, "n-only draft")).unwrap();
    assert_eq!(n, new_file(&g, &key));
    // L becomes readable again
    let repaired = serde_json::to_string_pretty(&session_with(&key, "legacy")).unwrap();
    fs::write(&legacy, &repaired).unwrap();
    let err = update_session_in_dir(&legacy, &g.path, |s| {
        s.review_comments.clear();
        Ok(())
    })
    .unwrap_err();
    assert!(matches!(err, TuicrError::SessionTargetChanged(_)), "{err}");
    assert_eq!(fs::read_to_string(&legacy).unwrap(), repaired);
    assert_eq!(
        load_session(&n).unwrap().review_comments[0].content,
        "n-only draft"
    );
}

// T15
#[test]
fn should_not_cross_hosts_between_coexisting_legacy_ghe_and_new_public() {
    let g = reviews();
    let dir = dir_of(&g);
    let legacy_ghe = write_legacy(&g, &key_on(GHE, "a"), "ghe");
    let public = save_session(&session_with(&key_on("github.com", "b"), "pub")).unwrap();
    // hostless selector (new public marker, CLI) -> public only
    assert_eq!(
        pr_session_path_in_dir(&dir, "gh:o/r/pr/42").unwrap(),
        Some(public.clone())
    );
    assert_eq!(
        pr_session_path_in_dir(&dir, "gh:o/r/pr/42@github.com").unwrap(),
        Some(public)
    );
    // legacy GHE marker -> GHE only
    assert_eq!(
        pr_session_path_in_dir(&dir, "gh:o/r/pr/42@ghe.internal").unwrap(),
        Some(legacy_ghe)
    );
    // manifest reset: the Some(github.com) entry is gone, hostless selector
    // must still never reach the legacy GHE None entry
    let mut m = manifest_of(&g);
    m.entries
        .get_mut("gh:o/r/pr/42")
        .unwrap()
        .retain(|e| matches!(&e.kind, ManifestKind::Pr { host: None, .. }));
    manifest::save_manifest(&g.path, &m).unwrap();
    assert_eq!(pr_session_path_in_dir(&dir, "gh:o/r/pr/42").unwrap(), None);
}

// T16
#[test]
fn should_prune_legacy_bucket_and_register_resolved_marker() {
    let g = reviews();
    let key = key_on(GHE, HEAD);
    let legacy = write_legacy(&g, &key, "old");
    let public = save_session(&session_with(&key_on("github.com", "b"), "pub")).unwrap();
    mark_session_active_in_dir(&session_with(&key, "x"), &legacy, &g.path).unwrap();
    let active = load_active_sessions_unlocked(&g.path).unwrap();
    assert_eq!(active.sessions[0].slug, "gh:o/r/pr/42@ghe.internal");
    clear_active_session_for_pid_in_dir(&g.path).unwrap();

    assert!(delete_session_in_dir(&legacy, &g.path).unwrap());
    assert!(!legacy.exists());
    let m = manifest_of(&g);
    assert_eq!(
        hosts_in(&m, "gh:o/r/pr/42"),
        vec![Some("github.com".into())]
    );
    assert!(public.exists());
}

// T17
#[test]
fn should_fail_closed_on_occupied_foreign_new_file() {
    let g = reviews();
    let key = key_on(GHE, HEAD);
    let n = new_file(&g, &key);
    fs::create_dir_all(n.parent().unwrap()).unwrap();
    let foreign = serde_json::to_string_pretty(&session_with(&key_on(GHE, "zzz"), "f")).unwrap();
    fs::write(&n, &foreign).unwrap();
    assert!(save_session(&session_with(&key, "mine")).is_err());
    assert_eq!(fs::read_to_string(&n).unwrap(), foreign);
}

// T18
#[test]
fn should_round_trip_ports_and_ipv6_hosts() {
    let g = reviews();
    for host in ["ghe.internal:443", "[::1]:443"] {
        let key = key_on(host, HEAD);
        let saved = save_session(&session_with(&key, host)).unwrap();
        let bucket = format!("gh:o/r/pr/42@{host}");
        assert_eq!(hosts_in(&manifest_of(&g), &bucket), vec![Some(host.into())]);
        assert_eq!(
            pr_session_path_in_dir(&g.path, &bucket).unwrap(),
            Some(saved.clone())
        );
        assert_eq!(load_pr_session(&key).unwrap().unwrap().0, saved);
        // the host never reaches a path component
        assert!(
            !saved
                .strip_prefix(&g.path)
                .unwrap()
                .to_string_lossy()
                .contains(':')
        );
    }
    assert_eq!(file_count(&g), 2);
}

// T19
#[test]
fn should_normalize_host_for_hash_slug_field_and_comparisons() {
    let g = reviews();
    let raw = key_on("GHE.Internal", HEAD);
    let lower = key_on("ghe.internal", HEAD);
    let dotted = key_on("ghe.internal.", HEAD);
    for k in [&lower, &dotted] {
        assert_eq!(new_pr_target(k).relative, new_pr_target(&raw).relative);
        assert_eq!(new_pr_target(k).manifest_slug, "gh:o/r/pr/42@ghe.internal");
        assert_eq!(new_pr_target(k).host.as_deref(), Some("ghe.internal"));
    }
    let saved = save_session(&session_with(&raw, "raw")).unwrap();
    assert_eq!(
        hosts_in(&manifest_of(&g), "gh:o/r/pr/42@ghe.internal"),
        vec![Some("ghe.internal".into())]
    );
    // reopen through another spelling: no foreign-key error, same file
    assert_eq!(load_pr_session(&lower).unwrap().unwrap().0, saved);
    assert_eq!(
        save_session(&session_with(&dotted, "again")).unwrap(),
        saved
    );
    assert_eq!(
        pr_session_path_in_dir(&g.path, "gh:o/r/pr/42@ghe.internal").unwrap(),
        Some(saved)
    );
    assert_eq!(file_count(&g), 1);
    // www.github.com is a distinct host
    assert_ne!(
        new_pr_target(&key_on("www.github.com", HEAD)).relative,
        new_pr_target(&key_on("github.com", HEAD)).relative
    );
    assert_eq!(
        new_pr_target(&key_on("www.github.com", HEAD)).manifest_slug,
        "gh:o/r/pr/42@www.github.com"
    );
    // legacy file persisted with a raw host is found by a normalized key
    let legacy_raw = key_on("GHE2.Internal", HEAD);
    let legacy = write_legacy(&g, &legacy_raw, "old");
    assert_eq!(
        session_path(&session_with(&key_on("ghe2.internal.", HEAD), "x")).unwrap(),
        legacy
    );
    assert_eq!(
        pr_session_path_in_dir(&g.path, "gh:o/r/pr/42@ghe2.internal").unwrap(),
        Some(legacy)
    );
    assert!(
        load_previous_head_pr_session(&key_on("ghe2.internal", "newer"))
            .unwrap()
            .is_some()
    );
}

// T20
#[test]
fn should_split_unreadable_legacy_file_into_absent_corrupt_and_io_error() {
    let g = reviews();
    let key = key_on(GHE, HEAD);
    let legacy = legacy_file(&g, &key);
    fs::create_dir_all(legacy.parent().unwrap()).unwrap();

    // NotFound -> N
    assert_eq!(
        resolve_pr_target(&g.path, &key).unwrap().relative,
        new_pr_target(&key).relative
    );
    // parse error -> N, L untouched, lookups skip it
    fs::write(&legacy, "{").unwrap();
    assert_eq!(
        resolve_pr_target(&g.path, &key).unwrap().relative,
        new_pr_target(&key).relative
    );
    assert!(load_pr_session(&key).unwrap().is_none());
    // a corrupt legacy manifest entry is skipped, not an error
    let mut m = Manifest::new();
    let t = legacy_pr_target(&key);
    m.upsert(
        t.manifest_slug.clone(),
        manifest::entry_from_session(&session_with(&key, "x"), t.relative.clone(), t.anchor),
    );
    manifest::save_manifest(&g.path, &m).unwrap();
    assert!(load_pr_session(&key).unwrap().is_none());
    assert!(
        load_previous_head_pr_session(&key_on(GHE, "new"))
            .unwrap()
            .is_none()
    );
    assert_eq!(
        pr_session_path_in_dir(&g.path, "gh:o/r/pr/42@ghe.internal").unwrap(),
        None
    );
    assert_eq!(fs::read_to_string(&legacy).unwrap(), "{");

    // other I/O error (a directory at L) -> error everywhere, no N created
    fs::remove_file(&legacy).unwrap();
    fs::create_dir(&legacy).unwrap();
    assert!(resolve_pr_target(&g.path, &key).is_err());
    assert!(save_session(&session_with(&key, "x")).is_err());
    assert!(!new_file(&g, &key).exists());
    assert!(load_pr_session(&key).is_err());
    assert!(load_previous_head_pr_session(&key_on(GHE, "new")).is_err());
    assert!(pr_session_path_in_dir(&g.path, "gh:o/r/pr/42@ghe.internal").is_err());
    assert!(session_path(&session_with(&key, "x")).is_err());
}

#[test]
fn should_render_and_parse_host_qualified_slugs() {
    // T9 (storage-facing half; grammar tests live in slug.rs)
    for (key, expected) in [
        (key_on("github.com", HEAD), "gh:o/r/pr/42"),
        (key_on("GitHub.com.", HEAD), "gh:o/r/pr/42"),
        (key_on(GHE, HEAD), "gh:o/r/pr/42@ghe.internal"),
        (key_on("[::1]:443", HEAD), "gh:o/r/pr/42@[::1]:443"),
    ] {
        let slug = Slug::from(&key);
        assert_eq!(slug.to_string(), expected);
        assert_eq!(expected.parse::<Slug>().unwrap(), slug);
    }
}

// Reindex covers both files a key can live in, and refuses a foreign occupant.
#[test]
fn should_reindex_legacy_and_new_files_and_refuse_foreign_occupant() {
    let g = reviews();
    let key = key_on(GHE, HEAD);
    let legacy = write_legacy(&g, &key, "old");
    fs::remove_file(g.path.join(MANIFEST_FILENAME)).unwrap();
    reindex_session(&session_with(&key, "x"), &legacy).unwrap();
    assert_eq!(hosts_in(&manifest_of(&g), "gh:o/r/pr/42"), vec![None]);

    let other = key_on("ghe3.internal", HEAD);
    let n = new_file(&g, &other);
    fs::write(
        &n,
        serde_json::to_string_pretty(&session_with(&key_on("ghe3.internal", "zzz"), "f")).unwrap(),
    )
    .unwrap();
    assert!(reindex_session(&session_with(&other, "x"), &n).is_err());
    assert!(reindex_session(&session_with(&other, "x"), &legacy).is_err());
}

// A manifest entry never vouches for a file of another PR: the file's own key
// decides (number, repo, forge, host).
#[test]
fn should_validate_headless_lookup_against_file_content() {
    let g = reviews();
    let key = key_on(GHE, HEAD);
    let saved = save_session(&session_with(&key, "x")).unwrap();
    let relative = new_pr_target(&key).relative;
    let mut m = manifest_of(&g);
    let entry = m.entries["gh:o/r/pr/42@ghe.internal"][0].clone();
    for lie in [
        "gh:o/r/pr/43@ghe.internal",
        "gh:o/other/pr/42@ghe.internal",
        "gh:p/r/pr/42@ghe.internal",
        "gl:o/r/pr/42@ghe.internal",
    ] {
        m.entries.insert(lie.to_string(), vec![entry.clone()]);
    }
    manifest::save_manifest(&g.path, &m).unwrap();
    for lie in [
        "gh:o/r/pr/43@ghe.internal",
        "gh:o/other/pr/42@ghe.internal",
        "gh:p/r/pr/42@ghe.internal",
        "gl:o/r/pr/42@ghe.internal",
    ] {
        assert_eq!(pr_session_path_in_dir(&g.path, lie).unwrap(), None, "{lie}");
    }
    assert_eq!(
        pr_session_path_in_dir(&g.path, "gh:o/r/pr/42@ghe.internal").unwrap(),
        Some(saved)
    );
    let _ = relative;
}

// A cached path spelled differently (`..`, symlinked temp dirs) is still the
// same file, not a changed target.
#[test]
fn should_treat_equivalent_spellings_of_cached_path_as_same_file() {
    let g = reviews();
    let key = key_on(GHE, HEAD);
    let saved = save_session(&session_with(&key, "one")).unwrap();
    let name = saved.file_name().unwrap().to_owned();
    let spelled = g
        .path
        .join("sessions")
        .join("..")
        .join("sessions")
        .join(name);
    let (path, _, ()) = save_session_by_identity_in_dir(
        &session_with(&key, "two"),
        &g.path,
        Some(&spelled),
        |persisted| Ok((persisted.unwrap(), ())),
    )
    .unwrap();
    assert_eq!(path, saved);
}
