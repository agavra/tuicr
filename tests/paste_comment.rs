//! Real-binary regressions for a paste that opens a comment. The `d`/`z`/`Z`/
//! leader prefixes and the Ctrl+C exit arm live in the main loop, which App
//! tests cannot reach. Skipped when tmux is not installed.

use std::path::{Path, PathBuf};
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

/// tuicr annotating a six-line file with the cursor on `row 3`, in its own
/// tmux server. The server is stopped on drop, including on a failed assert.
struct Pane {
    socket: PathBuf,
    _dir: tempfile::TempDir,
}

impl Pane {
    fn start() -> Self {
        let dir = tempfile::tempdir().expect("tempdir");
        let home = dir.path().join("home");
        std::fs::create_dir_all(&home).expect("home");
        let file = dir.path().join("notes.md");
        let body: String = (1..=6).map(|i| format!("row {i}\n")).collect();
        std::fs::write(&file, body).expect("write file");
        let pane = Self {
            socket: dir.path().join("tmux.sock"),
            _dir: dir,
        };

        // An isolated HOME keeps the session file out of the user's data dir.
        let command = format!(
            "env HOME={home} XDG_CONFIG_HOME={home}/.config XDG_DATA_HOME={home}/.local/share \
             {bin} --no-update-check --file {file} --line 3",
            home = home.display(),
            bin = env!("CARGO_BIN_EXE_tuicr"),
            file = file.display(),
        );
        tmux(
            &pane.socket,
            &["new-session", "-d", "-x", "100", "-y", "30", &command],
        );
        // Keep the pane once tuicr exits, so a quit fails fast as a quit.
        tmux(&pane.socket, &["set-option", "-g", "remain-on-exit", "on"]);
        pane.wait_for("the cursor on row 3", |screen| {
            screen
                .lines()
                .any(|line| line.contains('▶') && line.contains("row 3"))
        });
        pane
    }

    fn send(&self, key: &str) {
        tmux(&self.socket, &["send-keys", "-l", key]);
    }

    fn send_ctrl_c(&self) {
        tmux(&self.socket, &["send-keys", "C-c"]);
    }

    /// A bracketed paste, as a terminal sends a clipboard paste or a
    /// dictation tool its transcript.
    fn paste(&self, text: &str) {
        tmux(&self.socket, &["set-buffer", "-b", "clip", "--", text]);
        tmux(&self.socket, &["paste-buffer", "-p", "-d", "-b", "clip"]);
    }

    fn wait_for(&self, what: &str, done: impl Fn(&str) -> bool) -> String {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let screen = tmux(&self.socket, &["capture-pane", "-p"]);
            let dead = tmux(&self.socket, &["display-message", "-p", "#{pane_dead}"]);
            assert_ne!(
                dead.trim(),
                "1",
                "tuicr exited while waiting for {what}:\n{screen}"
            );
            if done(&screen) {
                return screen;
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for {what}:\n{screen}"
            );
            sleep(Duration::from_millis(50));
        }
    }
}

impl Drop for Pane {
    fn drop(&mut self) {
        tmux(&self.socket, &["kill-server"]);
    }
}

#[test]
fn should_type_into_the_comment_when_a_paste_interrupts_a_prefix() {
    if !tmux_available() {
        eprintln!("skipping: tmux not installed");
        return;
    }
    // Each prefix, then the key that would finish it: `ZZ` quits, `dd`
    // deletes, `zz` recenters, and `;c` (default leader) opens a review
    // comment in place of the draft. `\;` is a literal `;` to tmux.
    for (prefix, key) in [("Z", "Z"), ("d", "d"), ("z", "z"), ("\\;", "c")] {
        let pane = Pane::start();
        pane.send(prefix);
        pane.paste("dictated ");
        pane.wait_for("the pasted comment", |screen| screen.contains("dictated"));

        pane.send(key);

        let typed = format!("dictated {key}");
        pane.wait_for(
            &format!("{typed:?} after the {prefix:?} prefix"),
            |screen| screen.contains(&typed),
        );
    }
}

#[test]
fn should_not_quit_on_a_ctrl_c_pair_split_by_a_paste() {
    if !tmux_available() {
        eprintln!("skipping: tmux not installed");
        return;
    }
    let pane = Pane::start();
    pane.send_ctrl_c();
    pane.paste("dictated ");
    pane.wait_for("the pasted comment", |screen| screen.contains("dictated"));

    pane.send_ctrl_c();

    // Ctrl+C still cancels the comment, then asks again before quitting.
    pane.wait_for("the cancelled comment and the exit prompt", |screen| {
        !screen.contains("dictated") && screen.contains("Press Ctrl+C again to exit")
    });
}
