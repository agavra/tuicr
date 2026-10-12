//! Opt-in, asynchronous shell callbacks for the TUI review lifecycle.

use std::io::{self, Seek, Write};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

use serde::Serialize;

use crate::app::App;
use crate::config::HooksConfig;

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HookEvent {
    ReviewStarted,
    ReviewTargetChanged,
    ReviewClosed,
}

impl HookEvent {
    fn command(self, config: &HooksConfig) -> Option<&str> {
        match self {
            Self::ReviewStarted => config.review_started.as_deref(),
            Self::ReviewTargetChanged => config.review_target_changed.as_deref(),
            Self::ReviewClosed => config.review_closed.as_deref(),
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::ReviewStarted => "review_started",
            Self::ReviewTargetChanged => "review_target_changed",
            Self::ReviewClosed => "review_closed",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReviewTarget {
    pub kind: &'static str,
    pub session_id: String,
    pub slug: Option<String>,
    pub session_file: Option<PathBuf>,
    pub head_sha: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct HookPayload {
    pub schema_version: u32,
    pub event: HookEvent,
    pub sequence: u64,
    pub pid: u32,
    pub launch_dir: Option<PathBuf>,
    pub cwd: Option<PathBuf>,
    pub worktree: Option<PathBuf>,
    pub review: Option<ReviewTarget>,
}

impl ReviewTarget {
    fn matches_app(&self, app: &App) -> bool {
        self.session_id == app.session.id
            && self.session_file.as_deref() == app.session_path.as_deref()
            && self.head_sha.as_deref()
                == app
                    .session
                    .pr_session_key
                    .as_ref()
                    .map(|key| key.head_sha.as_str())
    }
}

pub struct Hooks {
    config: HooksConfig,
    failure_tx: Sender<String>,
    failures: Receiver<String>,
    payload: HookPayload,
}

impl Hooks {
    pub fn start(config: HooksConfig, app: &App, launch_dir: Option<PathBuf>) -> Option<Self> {
        if [
            HookEvent::ReviewStarted,
            HookEvent::ReviewTargetChanged,
            HookEvent::ReviewClosed,
        ]
        .iter()
        .all(|event| {
            event
                .command(&config)
                .is_none_or(|command| command.trim().is_empty())
        }) {
            return None;
        }
        let (failure_tx, failures) = mpsc::channel();
        Some(Self {
            config,
            failure_tx,
            failures,
            payload: HookPayload {
                schema_version: 1,
                event: HookEvent::ReviewStarted,
                sequence: 0,
                pid: std::process::id(),
                launch_dir,
                cwd: std::env::current_dir().ok(),
                worktree: app.review_hook_worktree(),
                review: app.review_hook_target(),
            },
        })
    }

    pub fn started(&mut self) -> Option<String> {
        self.invoke(HookEvent::ReviewStarted).err()
    }

    /// Called after a complete action/result, so provisional sessions are not published.
    pub fn observe(&mut self, app: &App) -> Option<String> {
        let unchanged = match (app.review_target_selected, &self.payload.review) {
            (false, None) => true,
            (true, Some(target)) => target.matches_app(app),
            _ => false,
        };
        if unchanged {
            return None;
        }
        self.payload.worktree = app.review_hook_worktree();
        self.payload.review = app.review_hook_target();
        self.invoke(HookEvent::ReviewTargetChanged).err()
    }

    pub fn poll_failures(&self) -> Vec<String> {
        self.failures.try_iter().collect()
    }

    pub fn closed(&mut self) -> Option<String> {
        self.invoke(HookEvent::ReviewClosed).err()
    }

    fn invoke(&mut self, event: HookEvent) -> Result<(), String> {
        self.payload.event = event;
        self.payload.sequence += 1;
        self.payload.cwd = std::env::current_dir().ok();
        let Some(command) = event
            .command(&self.config)
            .filter(|command| !command.trim().is_empty())
        else {
            return Ok(());
        };
        let mut json = serde_json::to_vec(&self.payload)
            .map_err(|err| format!("Hook {} payload failed: {err}", event.name()))?;
        json.push(b'\n');
        let input = tempfile::tempfile()
            .and_then(|mut file| {
                file.write_all(&json)?;
                file.rewind()?;
                Ok(file)
            })
            .map_err(|err| format!("Hook {} stdin failed: {err}", event.name()))?;
        #[cfg(unix)]
        let mut shell = {
            use std::os::unix::process::CommandExt;
            let mut shell = Command::new("sh");
            shell.args(["-c", command]).process_group(0);
            shell
        };
        #[cfg(windows)]
        let mut shell = {
            let mut shell = Command::new("cmd");
            shell.args(["/D", "/S", "/C", command]);
            shell
        };
        if let Some(cwd) = &self.payload.cwd {
            shell.current_dir(cwd);
        }
        let child = shell
            .stdin(input)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|err| format!("Hook {} launch failed: {err}", event.name()))?;
        let failures = self.failure_tx.clone();
        let sequence = self.payload.sequence;
        thread::Builder::new()
            .name("tuicr-hook".into())
            .spawn(move || {
                if let Err(err) = run_hook(child) {
                    let _ = failures.send(format!(
                        "Hook {} (event {sequence}) failed: {err}",
                        event.name()
                    ));
                }
            })
            .map(|_| ())
            .map_err(|err| format!("Hook {} thread failed: {err}", event.name()))
    }
}

fn run_hook(mut child: Child) -> io::Result<()> {
    let status = child.wait()?;
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!("exit status {status}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{DiffSource, InputMode};
    use crate::model::{ReviewSession, SessionDiffSource};
    use crate::theme::Theme;
    use crate::vcs::{FileBackend, VcsBackend};

    fn app(dir: &std::path::Path, mode: InputMode) -> App {
        let file = dir.join("sample.txt");
        std::fs::write(&file, "sample\n").unwrap();
        let vcs = FileBackend::new(file.to_str().unwrap()).unwrap();
        let info = vcs.info().clone();
        let session = ReviewSession::new(
            info.root_path.clone(),
            info.head_commit.clone(),
            None,
            SessionDiffSource::WorkingTree,
        );
        App::build(
            Box::new(vcs),
            info,
            Theme::dark(),
            None,
            false,
            Vec::new(),
            session,
            DiffSource::WorkingTree,
            mode,
            Vec::new(),
            None,
            None,
        )
        .unwrap()
    }

    #[cfg(unix)]
    fn wait_for(mut ready: impl FnMut() -> bool) {
        use std::time::{Duration, Instant};
        let deadline = Instant::now() + Duration::from_secs(15);
        while !ready() {
            assert!(Instant::now() < deadline, "callback handshake timed out");
            thread::sleep(Duration::from_millis(1));
        }
    }

    #[cfg(unix)]
    fn payload(path: &std::path::Path) -> Option<serde_json::Value> {
        std::fs::read(path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
    }

    #[test]
    fn disabled_hooks_have_no_runner() {
        let dir = tempfile::tempdir().unwrap();
        let app = app(dir.path(), InputMode::Normal);
        assert!(Hooks::start(HooksConfig::default(), &app, None).is_none());
    }

    #[test]
    fn identity_ignores_view_and_content_changes() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app(dir.path(), InputMode::Normal);
        let target = app.review_hook_target().unwrap();
        app.session.updated_at = chrono::Utc::now();
        app.session.session_notes = Some("notes".into());
        app.session.commit_selection_range = Some((0, 0));
        app.input_mode = InputMode::CommitSelect;
        app.session.base_commit = "new-head".into();
        assert!(target.matches_app(&app));
        app.session_path = Some(dir.path().join("next.json"));
        assert!(!target.matches_app(&app));
        app.session_path = target.session_file.clone();
        app.session.id = "next-session".into();
        assert!(!target.matches_app(&app));
        app.session.pr_session_key = Some(crate::forge::traits::PrSessionKey::new(
            crate::forge::traits::ForgeRepository::github("github.com", "owner", "repo"),
            1,
            "head-a",
        ));
        let target = app.review_hook_target().unwrap();
        app.session.pr_session_key.as_mut().unwrap().head_sha = "head-b".into();
        assert!(!target.matches_app(&app));
    }

    #[test]
    fn worktree_uses_known_checkout_paths_only() {
        let dir = tempfile::tempdir().unwrap();
        git2::Repository::init(dir.path()).unwrap();
        let nested = dir.path().join("unrelated");
        std::fs::create_dir(&nested).unwrap();
        assert!(
            app(&nested, InputMode::Normal)
                .review_hook_worktree()
                .is_none()
        );
        let mut app = app(dir.path(), InputMode::Normal);
        assert!(app.review_hook_worktree().is_none());
        app.is_pristine_mode = true;
        assert_eq!(
            app.review_hook_worktree(),
            Some(dir.path().canonicalize().unwrap())
        );
        app.is_pristine_mode = false;
        for vcs_type in [
            crate::vcs::traits::VcsType::Git,
            crate::vcs::traits::VcsType::Jujutsu,
            crate::vcs::traits::VcsType::Mercurial,
        ] {
            app.vcs_info.vcs_type = vcs_type;
            assert_eq!(
                app.review_hook_worktree(),
                Some(dir.path().canonicalize().unwrap())
            );
        }
        let repository =
            crate::forge::traits::ForgeRepository::github("github.com", "owner", "repo");
        app.session.pr_session_key = Some(crate::forge::traits::PrSessionKey::new(
            repository.clone(),
            1,
            "head",
        ));
        app.forge_repository = Some(repository.clone());
        app.forge_backend = Some(Box::new(
            crate::forge::github::gh::GitHubGhBackend::new(Some(repository))
                .with_local_checkout(Some(nested.clone())),
        ));
        assert_eq!(
            app.review_hook_worktree(),
            Some(nested.canonicalize().unwrap())
        );
        app.forge_repository = None;
        assert!(app.review_hook_worktree().is_none());
        app.forge_backend = None;
        assert!(app.review_hook_worktree().is_none());
    }

    #[cfg(unix)]
    #[test]
    fn callbacks_run_independently_and_close_does_not_wait() {
        let dir = tempfile::tempdir().unwrap();
        let gate = tempfile::NamedTempFile::new_in(dir.path()).unwrap();
        let started = dir.path().join("started.json");
        let changed = dir.path().join("changed.json");
        let closed = dir.path().join("closed.json");
        let quote = |path: &std::path::Path| {
            shlex::try_quote(path.to_str().unwrap())
                .unwrap()
                .into_owned()
        };
        let record = |path: &std::path::Path| {
            format!(
                "while IFS= read -r payload; do printf '%s\\n' \"$payload\"; done > {}",
                quote(path)
            )
        };
        let mut app = app(dir.path(), InputMode::Normal);
        let mut hooks = Hooks::start(
            HooksConfig {
                review_started: Some(format!(
                    "{}; while [ -e {} ]; do sleep 0.01; done; exit 7",
                    record(&started),
                    quote(gate.path())
                )),
                review_target_changed: Some(record(&changed)),
                review_closed: Some(record(&closed)),
            },
            &app,
            None,
        )
        .unwrap();
        assert!(hooks.started().is_none());
        wait_for(|| payload(&started).is_some());
        app.session.id = "next-session".into();
        assert!(hooks.observe(&app).is_none());
        assert!(hooks.closed().is_none());
        wait_for(|| payload(&changed).is_some() && payload(&closed).is_some());
        for (path, event, sequence) in [
            (&started, "review_started", 1),
            (&changed, "review_target_changed", 2),
            (&closed, "review_closed", 3),
        ] {
            let payload = payload(path).unwrap();
            assert_eq!(payload["event"], event);
            assert_eq!(payload["sequence"], sequence);
            assert_eq!(payload["pid"], std::process::id());
        }
        assert_eq!(
            payload(&closed).unwrap()["review"]["session_id"],
            "next-session"
        );
        assert!(hooks.poll_failures().is_empty());
        drop(gate);
        let mut warnings = Vec::new();
        wait_for(|| {
            warnings.extend(hooks.poll_failures());
            !warnings.is_empty()
        });
        assert!(
            warnings
                .iter()
                .any(|warning| warning.contains("exit status"))
        );
    }

    #[cfg(unix)]
    #[test]
    fn close_payload_survives_process_exit() {
        use std::time::{Duration, Instant};

        if let Some(log) = std::env::var_os("TUICR_TEST_CLOSE_LOG") {
            let log = std::path::Path::new(&log);
            let app = app(log.parent().unwrap(), InputMode::Normal);
            let mut hooks = Hooks::start(
                HooksConfig {
                    review_closed: Some(format!(
                        "while kill -0 {} 2>/dev/null; do sleep 0.01; done; \
                         while IFS= read -r payload; do printf '%s\\n' \"$payload\"; done > \"$TUICR_TEST_CLOSE_LOG\"",
                        std::process::id()
                    )),
                    ..HooksConfig::default()
                },
                &app,
                None,
            )
            .unwrap();
            assert!(hooks.closed().is_none());
            std::process::exit(0);
        }

        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("closed.json");
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "hooks::tests::close_payload_survives_process_exit",
            ])
            .env("TUICR_TEST_CLOSE_LOG", &log)
            .stdout(Stdio::null())
            .spawn()
            .unwrap();
        let pid = child.id();
        let deadline = Instant::now() + Duration::from_secs(15);
        let status = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break status;
            }
            if Instant::now() >= deadline {
                child.kill().unwrap();
                child.wait().unwrap();
                panic!("hook launch delayed process exit");
            }
            thread::sleep(Duration::from_millis(1));
        };
        assert!(status.success());
        wait_for(|| payload(&log).is_some());
        let payload = payload(&log).unwrap();
        assert_eq!(payload["event"], "review_closed");
        assert_eq!(payload["pid"], pid);
        assert_eq!(payload["review"]["kind"], "local");
    }
}
