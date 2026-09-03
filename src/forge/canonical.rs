//! Resolve a GitHub repository's "canonical" identity — i.e. the upstream
//! parent of a fork — so PR listings against a local `origin` that points at
//! a fork (e.g. `agavra/slatedb`) reach the repo where PR activity actually
//! lives (`slatedb/slatedb`).
//!
//! The resolver is intentionally non-fatal: any failure (no `gh` binary,
//! auth missing, network down, repo private, non-fork response) falls back
//! to the supplied origin so a tuicr session degrades to the historical
//! behavior instead of erroring out.
#![allow(dead_code)]

use serde::Deserialize;

use crate::forge::github::gh::GhCommandRunner;
use crate::forge::traits::ForgeRepository;

/// Resolve `origin` to its canonical (parent) repository. Order:
///   1. `override_repo` (from `--repo-url`) wins, no I/O.
///   2. `gh api repos/<owner>/<repo>` parent lookup.
///   3. Fall back to `origin` if the lookup fails or the repo is not a fork.
pub fn resolve_canonical_repository(
    origin: &ForgeRepository,
    override_repo: Option<&ForgeRepository>,
    runner: &dyn GhCommandRunner,
) -> ForgeRepository {
    if let Some(repo) = override_repo {
        return repo.clone();
    }
    match fetch_parent(origin, runner) {
        Some(parent) => parent,
        None => origin.clone(),
    }
}

#[derive(Deserialize)]
struct RepoView {
    parent: Option<ParentRepo>,
}

#[derive(Deserialize)]
struct ParentRepo {
    /// `parent.full_name` is `"<owner>/<repo>"`. The host is inherited from
    /// the origin: GitHub does not let a fork live on a different host than
    /// its parent, so we don't need to re-parse it from the response.
    full_name: String,
}

fn fetch_parent(origin: &ForgeRepository, runner: &dyn GhCommandRunner) -> Option<ForgeRepository> {
    // `gh api repos/<owner>/<repo>` returns a JSON object with `parent` set
    // only when the repo is a fork. We ask for just the two fields we need
    // via `--jq` so the response stays small.
    let endpoint = format!("repos/{}/{}", origin.owner, origin.name);
    let mut args = vec![
        "api".to_string(),
        endpoint,
        "--jq".to_string(),
        "{ parent: (.parent | if . == null then null else { full_name: .full_name } end) }"
            .to_string(),
    ];
    if origin.host != "github.com" {
        args.push("--hostname".to_string());
        args.push(origin.host.clone());
    }

    let output = runner.run(&args).ok()?;
    let view: RepoView = serde_json::from_str(&output).ok()?;
    let parent = view.parent?;
    let (owner, name) = parent.full_name.split_once('/')?;
    if owner.is_empty() || name.is_empty() {
        return None;
    }
    Some(ForgeRepository::github(origin.host.clone(), owner, name))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::forge::github::gh::{GhCommandError, GhCommandResult};
    use std::cell::RefCell;

    #[derive(Default)]
    struct FakeRunner {
        response: RefCell<Option<GhCommandResult<String>>>,
        calls: RefCell<Vec<Vec<String>>>,
    }

    impl FakeRunner {
        fn with_ok(body: &str) -> Self {
            let me = Self::default();
            *me.response.borrow_mut() = Some(Ok(body.to_string()));
            me
        }

        fn with_err(err: GhCommandError) -> Self {
            let me = Self::default();
            *me.response.borrow_mut() = Some(Err(err));
            me
        }
    }

    impl GhCommandRunner for FakeRunner {
        fn run(&self, args: &[String]) -> GhCommandResult<String> {
            self.calls.borrow_mut().push(args.to_vec());
            self.response
                .borrow_mut()
                .take()
                .unwrap_or(Err(GhCommandError::Failed {
                    status: Some(1),
                    stderr: "no response configured".to_string(),
                }))
        }
    }

    fn fork() -> ForgeRepository {
        ForgeRepository::github("github.com", "agavra", "slatedb")
    }

    fn upstream() -> ForgeRepository {
        ForgeRepository::github("github.com", "slatedb", "slatedb")
    }

    #[test]
    fn should_return_override_without_calling_gh() {
        // given
        let runner = FakeRunner::default();
        let override_repo = upstream();
        // when
        let resolved = resolve_canonical_repository(&fork(), Some(&override_repo), &runner);
        // then
        assert_eq!(resolved, upstream());
        assert!(
            runner.calls.borrow().is_empty(),
            "override must short-circuit the gh api call"
        );
    }

    #[test]
    fn should_return_parent_when_gh_reports_fork() {
        // given
        let runner = FakeRunner::with_ok(r#"{"parent":{"full_name":"slatedb/slatedb"}}"#);
        // when
        let resolved = resolve_canonical_repository(&fork(), None, &runner);
        // then
        assert_eq!(resolved, upstream());
    }

    #[test]
    fn should_preserve_origin_host_on_enterprise() {
        // given — fork lives on a GitHub Enterprise host
        let ghe_fork = ForgeRepository::github("ghe.internal", "agavra", "slatedb");
        let runner = FakeRunner::with_ok(r#"{"parent":{"full_name":"slatedb/slatedb"}}"#);
        // when
        let resolved = resolve_canonical_repository(&ghe_fork, None, &runner);
        // then — parent inherits the GHE host (GitHub doesn't allow cross-host forks)
        assert_eq!(
            resolved,
            ForgeRepository::github("ghe.internal", "slatedb", "slatedb")
        );
        // and — the --hostname flag was passed to gh
        let calls = runner.calls.borrow();
        let last = calls.last().expect("expected a gh call");
        assert!(last.iter().any(|a| a == "--hostname"));
        assert!(last.iter().any(|a| a == "ghe.internal"));
    }

    #[test]
    fn should_fall_back_to_origin_when_repo_is_not_a_fork() {
        // given — the canonical repo itself; `parent` is null
        let runner = FakeRunner::with_ok(r#"{"parent":null}"#);
        // when
        let resolved = resolve_canonical_repository(&upstream(), None, &runner);
        // then
        assert_eq!(resolved, upstream());
    }

    #[test]
    fn should_fall_back_to_origin_when_gh_fails() {
        // given — gh not installed, auth missing, offline, etc.
        let runner = FakeRunner::with_err(GhCommandError::MissingGh);
        // when
        let resolved = resolve_canonical_repository(&fork(), None, &runner);
        // then — degrade silently to the detected origin
        assert_eq!(resolved, fork());
    }

    #[test]
    fn should_fall_back_to_origin_when_response_is_malformed() {
        // given
        let runner = FakeRunner::with_ok("not json");
        // when
        let resolved = resolve_canonical_repository(&fork(), None, &runner);
        // then
        assert_eq!(resolved, fork());
    }

    #[test]
    fn should_fall_back_to_origin_when_parent_full_name_is_malformed() {
        // given — full_name missing the slash
        let runner = FakeRunner::with_ok(r#"{"parent":{"full_name":"no-slash"}}"#);
        // when
        let resolved = resolve_canonical_repository(&fork(), None, &runner);
        // then
        assert_eq!(resolved, fork());
    }
}
