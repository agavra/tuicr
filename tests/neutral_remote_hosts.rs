//! Neutral (name-less) self-hosted forge hosts recognized only through the
//! forge CLI's own config. Each test re-executes itself with a fake `glab` /
//! `tea` on `PATH` and a scratch `HOME`, because host discovery spawns the CLIs
//! and caches the answer per process.
#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::process::Command;
use tuicr::forge::traits::ForgeRepository;

fn run_child(test: &str, marker: &str, program: &str, script_body: &str, ssh_config: &str) {
    let dir = tempfile::tempdir().unwrap();
    let bin = dir.path().join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let script = bin.join(program);
    std::fs::write(&script, format!("#!/bin/sh\n{script_body}\n")).unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::create_dir_all(dir.path().join(".ssh")).unwrap();
    std::fs::write(dir.path().join(".ssh/config"), ssh_config).unwrap();

    let path = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let out = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", test, "--nocapture"])
        .env(marker, "1")
        .env("HOME", dir.path())
        .env("PATH", path)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn glab_configured_neutral_host_shorthand_target_routes_to_gitlab() {
    const CHILD: &str = "TUICR_TEST_NEUTRAL_GITLAB_CHILD";
    if std::env::var_os(CHILD).is_some() {
        let target =
            tuicr::forge::github::gh::parse_pull_request_target("code.corp.internal/owner/repo#5")
                .unwrap();
        assert_eq!(
            target.repository,
            Some(ForgeRepository::gitlab(
                "code.corp.internal",
                "owner",
                "repo"
            ))
        );
        return;
    }
    run_child(
        "glab_configured_neutral_host_shorthand_target_routes_to_gitlab",
        CHILD,
        "glab",
        "echo https://code.corp.internal:8443/gitlab",
        "",
    );
}

#[test]
fn tea_login_neutral_resolved_hostname_replaces_gitea_named_alias() {
    const CHILD: &str = "TUICR_TEST_NEUTRAL_GITEA_CHILD";
    if std::env::var_os(CHILD).is_some() {
        let repo =
            tuicr::forge::gitea::tea::parse_gitea_remote_url("git@gitea-neutral:o/r.git").unwrap();
        assert_eq!(repo.host, "git.corp.internal");
        return;
    }
    run_child(
        "tea_login_neutral_resolved_hostname_replaces_gitea_named_alias",
        CHILD,
        "tea",
        r#"echo '[{"name":"neutral","url":"https://git.corp.internal","default":"true"}]'"#,
        "Host gitea-neutral\n  HostName git.corp.internal\n",
    );
}
