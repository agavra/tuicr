//! Real-binary rename regression, using the tmux screen-capture pattern in start_line.rs.
//! Requires Unix for HOME/XDG session isolation; skipped when tmux is unavailable.

#![cfg(unix)]

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread::sleep;
use std::time::{Duration, Instant};

struct Terminal {
    dir: Option<tempfile::TempDir>,
    socket: PathBuf,
}

impl Terminal {
    fn tmux(&self, args: &[&str]) -> String {
        let output = Command::new("tmux")
            .arg("-S")
            .arg(&self.socket)
            .arg("-f")
            .arg(self.path().join("tmux.conf"))
            .args(args)
            .output()
            .expect("run tmux");
        assert!(
            output.status.success(),
            "tmux {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).expect("UTF-8 terminal output")
    }

    fn path(&self) -> &Path {
        self.dir.as_ref().unwrap().path()
    }

    fn screen(&self) -> String {
        let screen = self.tmux(&["capture-pane", "-p", "-t", "review:0.0"]);
        fs::write(self.path().join("screen.txt"), &screen).unwrap();
        screen
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        // Only this test's server is stopped, including when an assertion fails.
        let _ = Command::new("tmux")
            .arg("-S")
            .arg(&self.socket)
            .arg("kill-server")
            .output();
        if std::thread::panicking() {
            let path = self.dir.take().unwrap().keep();
            eprintln!(
                "Terminal captures, fixture, and diagnostics retained at {}",
                path.display()
            );
        }
    }
}

fn wait_for(description: &str, mut ready: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while !ready() {
        assert!(
            Instant::now() < deadline,
            "timed out waiting for {description}"
        );
        sleep(Duration::from_millis(100));
    }
}

fn repository_bytes(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn visit(root: &Path, dir: &Path, files: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                visit(root, &path, files);
            } else {
                files.insert(
                    path.strip_prefix(root).unwrap().to_path_buf(),
                    fs::read(path).unwrap(),
                );
            }
        }
    }
    let mut files = BTreeMap::new();
    visit(root, root, &mut files);
    files
}

#[test]
fn should_render_an_unstaged_rename_without_changing_the_repository() {
    if !Command::new("tmux")
        .arg("-V")
        .output()
        .is_ok_and(|output| output.status.success())
    {
        eprintln!("skipping: tmux not installed");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("tmux.sock");
    let terminal = Terminal {
        dir: Some(dir),
        socket,
    };
    let root = terminal.path();
    let home = root.join("home");
    let config = root.join("config/tuicr");
    let data = root.join("data");
    for path in [&home, &config, &data] {
        fs::create_dir_all(path).unwrap();
    }
    fs::write(config.join("config.toml"), "backend = \"libgit2\"\nno_update_check = true\ndiff_view = \"unified\"\nshow_file_list = true\nreview_watch_interval_ms = 0\ndiff_watch_interval_ms = 0\n").unwrap();
    fs::write(root.join("tmux.conf"), "set -g status off\nset -g remain-on-exit on\nset -g set-clipboard off\nset -g default-terminal xterm-256color\n").unwrap();

    let repo_path = root.join("repo");
    let repo = git2::Repository::init(&repo_path).unwrap();
    repo.config()
        .unwrap()
        .set_bool("diff.renames", true)
        .unwrap();
    let original: String = (1..=24)
        .map(|i| format!("KEEP_{i:02} rename fixture stable content\n"))
        .collect();
    fs::write(repo_path.join("b-old.txt"), &original).unwrap();
    fs::write(
        repo_path.join("c-delete.txt"),
        "unrelated deleted amber orchard\n",
    )
    .unwrap();
    fs::write(
        repo_path.join("e-modify.txt"),
        "unrelated modification before\n",
    )
    .unwrap();
    let mut index = repo.index().unwrap();
    for path in ["b-old.txt", "c-delete.txt", "e-modify.txt"] {
        index.add_path(Path::new(path)).unwrap();
    }
    index.write().unwrap();
    let tree = repo.find_tree(index.write_tree().unwrap()).unwrap();
    let signature = git2::Signature::now("UI Acceptance", "accept@example.test").unwrap();
    repo.commit(Some("HEAD"), &signature, &signature, "fixture", &tree, &[])
        .unwrap();
    fs::rename(repo_path.join("b-old.txt"), repo_path.join("a-new.txt")).unwrap();
    fs::write(
        repo_path.join("a-new.txt"),
        original.replace(
            "KEEP_12 rename fixture stable content",
            "EDIT_12 renamed and edited content",
        ),
    )
    .unwrap();
    fs::remove_file(repo_path.join("c-delete.txt")).unwrap();
    fs::write(
        repo_path.join("d-add.txt"),
        "unrelated added violet ocean\n",
    )
    .unwrap();
    fs::write(
        repo_path.join("e-modify.txt"),
        "unrelated modification after\n",
    )
    .unwrap();
    assert!(
        repo.status_file(Path::new("a-new.txt"))
            .unwrap()
            .is_wt_new()
    );
    assert!(
        repo.status_file(Path::new("b-old.txt"))
            .unwrap()
            .is_wt_deleted()
    );
    assert!(index.get_path(Path::new("a-new.txt"), 0).is_none());
    let before = repository_bytes(&repo_path);

    let exit_code = root.join("exit-code.txt");
    let words = [
        "env".to_string(),
        format!("HOME={}", home.display()),
        format!("XDG_CONFIG_HOME={}", root.join("config").display()),
        format!("XDG_DATA_HOME={}", data.display()),
        "GIT_CONFIG_NOSYSTEM=1".into(),
        "GIT_CONFIG_GLOBAL=/dev/null".into(),
        "GIT_OPTIONAL_LOCKS=0".into(),
        env!("CARGO_BIN_EXE_tuicr").into(),
        "--working-tree".into(),
        "--stdout".into(),
        "--no-update-check".into(),
    ];
    let command = format!(
        "{} >{} 2>{}; printf '%s\\n' \"$?\" >{}",
        shlex::try_join(words.iter().map(String::as_str)).unwrap(),
        shlex::try_quote(root.join("stdout.txt").to_str().unwrap()).unwrap(),
        shlex::try_quote(root.join("stderr.txt").to_str().unwrap()).unwrap(),
        shlex::try_quote(exit_code.to_str().unwrap()).unwrap(),
    );
    terminal.tmux(&[
        "new-session",
        "-d",
        "-s",
        "review",
        "-x",
        "180",
        "-y",
        "80",
        "-c",
        repo_path.to_str().unwrap(),
        &command,
    ]);
    let mut screen = String::new();
    // Readiness must not depend on rename detection: upstream's D+A display
    // reaches this gate too, then fails the behavioral assertions below.
    wait_for("review tree, diff hunk, and NORMAL mode", || {
        assert!(
            !exit_code.exists(),
            "tuicr exited before rendering; see stderr.txt"
        );
        screen = terminal.screen();
        [
            "Files",
            "NORMAL",
            "@@",
            "c-delete.txt",
            "d-add.txt",
            "e-modify.txt",
        ]
        .iter()
        .all(|token| screen.contains(token))
    });
    assert!(
        data.join("tuicr/reviews").is_dir(),
        "session storage must be isolated"
    );
    let tree: String = screen
        .lines()
        .filter_map(|line| line.split('│').nth(1))
        .map(|line| format!("{line}\n"))
        .collect();
    assert!(tree.contains("R a-new.txt"), "missing rename:\n{screen}");
    assert!(
        !tree.contains("b-old.txt"),
        "separate old-path deletion:\n{screen}"
    );
    for expected in [
        "D c-delete.txt",
        "A d-add.txt",
        "M e-modify.txt",
        "Files · 0/4",
    ] {
        assert!(screen.contains(expected), "missing {expected}:\n{screen}");
    }
    for expected in [
        "@@ -9,7 +9,7 @@",
        "12 ▌ KEEP_12 rename fixture stable content",
        "12 ▌ EDIT_12 renamed and edited content",
        "11   KEEP_11 rename fixture stable content",
    ] {
        assert!(
            screen.contains(expected),
            "missing localized edit/context {expected}:\n{screen}"
        );
    }
    fs::write(
        root.join("screen-ansi.txt"),
        terminal.tmux(&["capture-pane", "-e", "-p", "-t", "review:0.0"]),
    )
    .unwrap();
    terminal.tmux(&["send-keys", "-t", "review:0.0", "-l", ":q"]);
    terminal.tmux(&["send-keys", "-t", "review:0.0", "Enter"]);
    wait_for("clean exit", || exit_code.exists());
    assert_eq!(fs::read_to_string(exit_code).unwrap().trim(), "0");
    assert_eq!(
        repository_bytes(&repo_path),
        before,
        "repository/index changed during review"
    );
}
