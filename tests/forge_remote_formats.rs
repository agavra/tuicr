use std::path::Path;
use std::process::Command;
use tuicr::forge::{
    detect_forge_repository, detect_github_repository, detect_gitlab_repository,
    local_checkout_for_repo, selector::PullRequestsTab, traits::ForgeRepository,
};

fn git(root: &Path, args: &[&str]) {
    let output = Command::new("git")
        .current_dir(root)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn check_remote_discovery(format: &[&str]) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let mut args = vec!["init", "-q"];
    args.extend_from_slice(format);
    git(root, &args);
    assert_eq!(detect_forge_repository(root), None);
    git(
        root,
        &[
            "remote",
            "add",
            "aaa",
            "https://github.com/other/project.git",
        ],
    );
    git(
        root,
        &[
            "remote",
            "add",
            "origin",
            "https://github.com/agavra/tuicr.git",
        ],
    );
    let target = ForgeRepository::github("github.com", "agavra", "tuicr");
    let nested = root.join("nested");
    std::fs::create_dir(&nested).unwrap();
    for path in [root, nested.as_path()] {
        assert_eq!(detect_forge_repository(path), Some(target.clone()));
        assert_eq!(detect_github_repository(path), Some(target.clone()));
        assert!(matches!(
            PullRequestsTab::new(detect_forge_repository(path)),
            PullRequestsTab::Idle { .. }
        ));
        assert_eq!(
            local_checkout_for_repo(path, &target),
            Some(path.to_path_buf())
        );
        assert_eq!(
            local_checkout_for_repo(
                path,
                &ForgeRepository::github("github.com", "missing", "repo")
            ),
            None
        );
    }
    git(root, &["remote", "remove", "origin"]);
    assert_eq!(
        detect_forge_repository(root),
        Some(ForgeRepository::github("github.com", "other", "project"))
    );
    git(
        root,
        &[
            "remote",
            "add",
            "origin",
            "https://gitlab.com/team/project.git",
        ],
    );
    assert_eq!(
        detect_gitlab_repository(root),
        Some(ForgeRepository::gitlab("gitlab.com", "team", "project"))
    );
}

#[test]
fn sha256_forge_remote_discovery() {
    check_remote_discovery(&["--object-format=sha256"]);
}

#[test]
fn sha1_forge_remote_discovery_control() {
    check_remote_discovery(&["--object-format=sha1"]);
}

#[test]
fn reftable_forge_remote_discovery() {
    // Git versions before 2.45 cannot create a reftable fixture.
    let help = Command::new("git").args(["init", "-h"]).output().unwrap();
    let help = format!(
        "{}{}",
        String::from_utf8_lossy(&help.stdout),
        String::from_utf8_lossy(&help.stderr)
    );
    if !help.contains("--ref-format") {
        eprintln!("Skipping reftable fixture: installed Git lacks --ref-format");
        return;
    }
    check_remote_discovery(&["--object-format=sha1", "--ref-format=reftable"]);
}
