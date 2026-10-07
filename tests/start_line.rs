//! End-to-end check for `--line`: runs the real binary in a tmux pane and
//! reads the cursor row off the screen. This covers the `main.rs` wiring
//! (CLI → `App::pending_start_line` → apply after the first draw), which
//! unit tests cannot reach. Skipped when tmux is not installed.

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
