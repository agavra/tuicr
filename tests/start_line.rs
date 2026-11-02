//! Real-binary terminal regressions for start-line positioning and Summary deletion.
//! Reuses isolated tmux panes to cover main-loop wiring that App tests cannot reach.
//! Skipped when tmux is not installed.

use std::path::Path;
use std::process::Command;
use std::thread::sleep;
use std::time::{Duration, Instant};

fn tmux_available() -> bool {
    Command::new("tmux")
        .arg("-V")
        .output()
        .is_ok_and(|o| o.status.success())
}

fn tmux(socket: &Path, args: &[&str]) -> String {
    let out = Command::new("tmux")
        .arg("-S")
        .arg(socket)
        .args(args)
        .output()
        .expect("run tmux");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn should_open_with_the_cursor_on_the_requested_line() {
    if !tmux_available() {
        eprintln!("skipping: tmux not installed");
        return;
    }
    let dir = tempfile::tempdir().expect("tempdir");
    let home = dir.path().join("home");
    std::fs::create_dir_all(&home).expect("home");
    let file = dir.path().join("notes.md");
    let body: String = (1..=120).map(|i| format!("row {i}\n")).collect();
    std::fs::write(&file, body).expect("write file");
    let socket = dir.path().join("tmux.sock");

    // An isolated HOME keeps the session file out of the user's data dir.
    let command = format!(
        "env HOME={home} XDG_CONFIG_HOME={home}/.config XDG_DATA_HOME={home}/.local/share \
         {bin} --no-update-check --file {file} --line 80",
        home = home.display(),
        bin = env!("CARGO_BIN_EXE_tuicr"),
        file = file.display(),
    );
    tmux(
        &socket,
        &["new-session", "-d", "-x", "100", "-y", "30", &command],
    );

    let deadline = Instant::now() + Duration::from_secs(10);
    let mut screen = String::new();
    while Instant::now() < deadline {
        screen = tmux(&socket, &["capture-pane", "-p"]);
        if screen.contains("row 80") {
            break;
        }
        sleep(Duration::from_millis(100));
    }
    tmux(&socket, &["kill-server"]);

    let cursor_row = screen
        .lines()
        .find(|line| line.contains('▶'))
        .unwrap_or_else(|| panic!("no cursor marker on screen:\n{screen}"));
    assert!(
        cursor_row.contains("row 80"),
        "cursor should be on row 80, got {cursor_row:?}\n{screen}"
    );
    assert!(
        !screen.contains("row 1\n") && !screen.lines().any(|l| l.ends_with("row 1")),
        "row 80 should be centered, not shown from the top:\n{screen}"
    );
}

#[test]
fn summary_dd_refreshes_selection_between_queued_inputs_and_persists_deletion() {
    if !tmux_available() {
        eprintln!("skipping: tmux not installed");
        return;
    }
    let dir = tempfile::tempdir().expect("tempdir");
    let home = dir.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let socket = dir.path().join("tmux.sock");
    // Always stop this test's isolated server, including on assertion failure.
    struct Server<'a>(&'a Path);
    impl Drop for Server<'_> {
        fn drop(&mut self) {
            tmux(self.0, &["kill-server"]);
        }
    }
    let _server = Server(&socket);
    let git = |args: &[&str]| {
        let output = Command::new("git")
            .current_dir(dir.path())
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    };
    git(&["init", "-q"]);
    let file = dir.path().join("main.rs");
    std::fs::write(
        &file,
        (1..=40)
            .map(|i| format!("// new {i}\n"))
            .collect::<String>(),
    )
    .unwrap();
    git(&["add", "main.rs"]);
    let command = format!(
        "env HOME={home} XDG_CONFIG_HOME={home}/.config XDG_DATA_HOME={home}/.local/share {bin} --no-update-check -w",
        home = home.display(),
        bin = env!("CARGO_BIN_EXE_tuicr"),
    );
    tmux(
        &socket,
        &[
            "new-session",
            "-d",
            "-x",
            "120",
            "-y",
            "30",
            "-c",
            dir.path().to_str().unwrap(),
            &command,
        ],
    );
    let cli = |args: &[&str]| -> serde_json::Value {
        let output = Command::new(env!("CARGO_BIN_EXE_tuicr"))
            .current_dir(dir.path())
            .env("HOME", &home)
            .env("XDG_CONFIG_HOME", home.join(".config"))
            .env("XDG_DATA_HOME", home.join(".local/share"))
            .arg("review")
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).unwrap()
    };
    let wait_screen = |text: &str| {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let screen = tmux(&socket, &["capture-pane", "-p"]);
            if screen.contains(text) {
                break;
            }
            assert!(Instant::now() < deadline, "missing {text}:\n{screen}");
            sleep(Duration::from_millis(50));
        }
    };
    wait_screen("NORMAL");
    let sessions = cli(&["list", "--repo", dir.path().to_str().unwrap()]);
    let slug = sessions[0]["slug"].as_str().expect("active session");
    let session_path = Path::new(sessions[0]["path"].as_str().unwrap());
    assert!(
        session_path.starts_with(&home),
        "fixture storage must be isolated"
    );
    for (line, body) in [("10", "KEEP"), ("29", "ORPHAN_A"), ("29", "ORPHAN_B")] {
        cli(&[
            "add",
            "--session",
            slug,
            "--target-file",
            "main.rs",
            "--line",
            line,
            "--type",
            "note",
            body,
        ]);
    }
    std::fs::write(
        &file,
        (1..=20)
            .map(|i| format!("// new {i}\n"))
            .collect::<String>(),
    )
    .unwrap();
    tmux(&socket, &["send-keys", ":e", "Enter"]);
    wait_screen("Reloaded");
    // One batch enters Summary, selects an orphan and deletes it before another paint.
    tmux(&socket, &["send-keys", ":summary", "Enter", "j", "d", "d"]);
    wait_screen("Pending Comments (2)");
    let screen = tmux(&socket, &["capture-pane", "-p"]);
    assert!(screen.contains("KEEP"));
    assert_ne!(screen.contains("ORPHAN_A"), screen.contains("ORPHAN_B"));
    // q exits Summary without terminal Escape/Alt timing ambiguity.
    tmux(&socket, &["send-keys", "q", ":w", "Enter"]);
    wait_screen("Saved");
    let remaining = cli(&["comments", "--session", slug]);
    assert_eq!(remaining.as_array().unwrap().len(), 2);
    assert!(
        remaining
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["content"] == "KEEP")
    );
    // A modified second d must not delete anything.
    tmux(&socket, &["send-keys", ":summary", "Enter", "d", "C-d"]);
    wait_screen("Pending Comments (2)");
    // A distinct command-mode frame proves all preceding keys and the save ran.
    tmux(&socket, &["send-keys", "q", ":w", "Enter", ":"]);
    wait_screen("\n:");
    assert_eq!(cli(&["comments", "--session", slug]), remaining);
    tmux(
        &socket,
        &["send-keys", "summary", "Enter", "d", "d", "d", "d"],
    );
    wait_screen("No pending comments.");
    tmux(&socket, &["send-keys", "q", ":w", "Enter"]);
    wait_screen("Saved");
    assert_eq!(cli(&["comments", "--session", slug]), serde_json::json!([]));
}
