use std::io::{self, Write};
use std::sync::atomic::{AtomicU8, Ordering};

use crossterm::{
    event::{
        DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
        KeyboardEnhancementFlags, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
    },
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Frame, Terminal, backend::CrosstermBackend};

mod keyboard_probe;

pub use keyboard_probe::supports_keyboard_enhancement;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
enum PanicRestoreTarget {
    None = 0,
    Stdout = 1,
    DevTty = 2,
}

impl PanicRestoreTarget {
    fn from_u8(raw: u8) -> Self {
        match raw {
            1 => Self::Stdout,
            2 => Self::DevTty,
            _ => Self::None,
        }
    }
}

static PANIC_RESTORE_TARGET: AtomicU8 = AtomicU8::new(PanicRestoreTarget::None as u8);

fn set_panic_restore_target(target: PanicRestoreTarget) {
    PANIC_RESTORE_TARGET.store(target as u8, Ordering::Release);
}

/// Terminal capabilities enabled for one tuicr TUI session.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TerminalFeatures {
    mouse_enabled: bool,
    keyboard_enhancements_supported: bool,
    use_dev_tty: bool,
}

impl TerminalFeatures {
    /// Returns an empty feature set for a terminal session.
    pub fn new() -> Self {
        Self::default()
    }

    /// Configures whether mouse capture should be active in the TUI.
    pub fn mouse_enabled(mut self, enabled: bool) -> Self {
        self.mouse_enabled = enabled;
        self
    }

    /// Configures whether keyboard enhancement flags should be pushed.
    ///
    /// tuicr only enables these flags after probing support because unsupported
    /// terminals can echo the probe escape sequences into stdout.
    pub fn keyboard_enhancements_supported(mut self, supported: bool) -> Self {
        self.keyboard_enhancements_supported = supported;
        self
    }

    /// Configures whether panic teardown should write to `/dev/tty` instead of
    /// stdout (used when `--stdout` renders the TUI to `/dev/tty`).
    pub fn use_dev_tty(mut self, use_dev_tty: bool) -> Self {
        self.use_dev_tty = use_dev_tty;
        self
    }

    fn panic_restore_target(self) -> PanicRestoreTarget {
        if self.use_dev_tty {
            PanicRestoreTarget::DevTty
        } else {
            PanicRestoreTarget::Stdout
        }
    }

    /// Enters TUI terminal mode and returns the owning session.
    pub fn enter<W: Write>(self, writer: W) -> anyhow::Result<TerminalSession<W>> {
        TerminalSession::enter(writer, self)
    }
}

/// Owns terminal mode changes for the active TUI.
///
/// `TerminalSession` is the boundary that keeps raw mode,
/// alternate-screen state,
/// mouse capture,
/// bracketed paste,
/// and keyboard enhancement flags paired with the ratatui terminal.
/// Dropping the session performs a best-effort restore so early returns do not
/// leave the user's terminal in TUI mode.
pub struct TerminalSession<W: Write> {
    terminal: Terminal<CrosstermBackend<W>>,
    features: TerminalFeatures,
    active: bool,
    /// The xterm modifyOtherKeys fallback was requested because the kitty
    /// keyboard probe failed. Only then may deactivation reset it.
    modify_other_keys: bool,
}

impl<W: Write> TerminalSession<W> {
    fn enter(writer: W, features: TerminalFeatures) -> anyhow::Result<Self> {
        let backend = CrosstermBackend::new(writer);
        let mut terminal = Terminal::new(backend)?;
        set_panic_restore_target(features.panic_restore_target());
        let modify_other_keys = match activate_writer(terminal.backend_mut(), features) {
            Ok(enabled) => enabled,
            Err(err) => {
                set_panic_restore_target(PanicRestoreTarget::None);
                deactivate_writer_best_effort(
                    terminal.backend_mut(),
                    features.mouse_enabled,
                    false,
                );
                return Err(err);
            }
        };
        Ok(Self {
            terminal,
            features,
            active: true,
            modify_other_keys,
        })
    }

    /// Draws one frame while the TUI terminal session is active.
    pub fn draw<F>(&mut self, render_callback: F) -> std::io::Result<()>
    where
        F: FnOnce(&mut Frame),
    {
        self.terminal.draw(render_callback).map(|_| ())
    }

    /// Returns the terminal backend for low-level terminal commands.
    ///
    /// This is used for synchronized-update escape sequences that ratatui does
    /// not manage directly.
    pub fn backend_mut(&mut self) -> &mut CrosstermBackend<W> {
        self.terminal.backend_mut()
    }

    /// Restores the terminal state for the normal exit path.
    ///
    /// After this succeeds,
    /// dropping the session will not attempt a second restore.
    pub fn restore(&mut self) -> anyhow::Result<()> {
        if !self.active {
            return Ok(());
        }
        self.deactivate()?;
        Ok(())
    }

    /// Temporarily leaves TUI terminal mode.
    ///
    /// The returned guard re-enters TUI mode on `resume()` or,
    /// as a best-effort fallback,
    /// when the guard is dropped.
    pub fn suspend(&mut self) -> anyhow::Result<TerminalSuspension<'_, W>> {
        self.deactivate()?;
        Ok(TerminalSuspension {
            session: Some(self),
        })
    }

    fn activate(&mut self) -> anyhow::Result<()> {
        if self.active {
            return Ok(());
        }
        set_panic_restore_target(self.features.panic_restore_target());
        match activate_writer(self.terminal.backend_mut(), self.features) {
            Ok(enabled) => self.modify_other_keys = enabled,
            Err(err) => {
                set_panic_restore_target(PanicRestoreTarget::None);
                deactivate_writer_best_effort(
                    self.terminal.backend_mut(),
                    self.features.mouse_enabled,
                    self.modify_other_keys,
                );
                return Err(err);
            }
        }
        self.active = true;
        Ok(())
    }

    fn deactivate(&mut self) -> anyhow::Result<()> {
        if !self.active {
            return Ok(());
        }
        self.active = false;
        set_panic_restore_target(PanicRestoreTarget::None);
        deactivate_writer(
            self.terminal.backend_mut(),
            self.features.mouse_enabled,
            self.modify_other_keys,
        )?;
        self.modify_other_keys = false;
        Ok(())
    }

    /// Clears the screen and drops the back buffer so the next draw repaints
    /// every cell.
    ///
    /// Used after an editor hands the terminal back, and for `Ctrl-l` and
    /// resize events, where the terminal may hold cells that ratatui's diff
    /// against its last frame would otherwise leave alone.
    ///
    /// `Terminal::clear` cannot be used here: it starts by reading the cursor
    /// position, whose `ESC [ 6 n` query goes to stdout. When stdout is not
    /// the terminal the reply never arrives, and the two-second timeout fails
    /// the clear before it resets the back buffer, so the next draw diffs
    /// against the pre-editor screen and paints nothing.
    ///
    /// Resizing to the current size clears and resets the same way, but sizes
    /// itself from `/dev/tty`.
    pub fn repaint_from_scratch(&mut self) -> anyhow::Result<()> {
        let area = self.terminal.size()?.into();
        self.terminal.resize(area)?;
        Ok(())
    }
}

impl<W: Write> Drop for TerminalSession<W> {
    fn drop(&mut self) {
        if self.active {
            self.active = false;
            set_panic_restore_target(PanicRestoreTarget::None);
            deactivate_writer_best_effort(
                self.terminal.backend_mut(),
                self.features.mouse_enabled,
                self.modify_other_keys,
            );
            self.modify_other_keys = false;
        }
    }
}

/// Guard for a temporarily suspended TUI terminal session.
///
/// Holding this value means tuicr has left raw mode and the alternate screen so
/// another foreground process can use the terminal.
/// Dropping the guard re-enters TUI mode on a best-effort basis.
pub struct TerminalSuspension<'a, W: Write> {
    session: Option<&'a mut TerminalSession<W>>,
}

impl<W: Write> TerminalSuspension<'_, W> {
    /// Re-enters TUI terminal mode and consumes the suspension guard.
    ///
    /// Calling this explicit method is preferred on the normal path because it
    /// reports terminal restoration failures to the caller.
    pub fn resume(mut self) -> anyhow::Result<()> {
        let Some(session) = self.session.take() else {
            return Ok(());
        };
        session.activate()?;
        session.repaint_from_scratch()?;
        Ok(())
    }
}

impl<W: Write> Drop for TerminalSuspension<'_, W> {
    fn drop(&mut self) {
        let Some(session) = self.session.take() else {
            return;
        };
        if session.activate().is_ok() {
            let _ = session.repaint_from_scratch();
        }
    }
}

/// Restores terminal mode for the currently active TUI stream without
/// requiring a live session object.
///
/// This is intended for panic hooks,
/// where ownership may already be unwinding and the session value might not be
/// reachable. When no TUI session is active this is a no-op; when `--stdout`
/// rendered the TUI to `/dev/tty`, teardown goes to `/dev/tty` so captured
/// stdout stays clean.
pub fn restore_stdio_best_effort() {
    let target = PanicRestoreTarget::from_u8(
        PANIC_RESTORE_TARGET.swap(PanicRestoreTarget::None as u8, Ordering::AcqRel),
    );
    restore_target_best_effort(target, io::stdout, || {
        std::fs::File::options().write(true).open("/dev/tty")
    });
}

fn restore_target_best_effort<S, T>(
    target: PanicRestoreTarget,
    open_stdout: impl FnOnce() -> S,
    open_dev_tty: impl FnOnce() -> io::Result<T>,
) where
    S: Write,
    T: Write,
{
    match target {
        PanicRestoreTarget::None => {}
        PanicRestoreTarget::Stdout => {
            let mut stdout = open_stdout();
            // The session tracks the modifyOtherKeys fallback and resets it when
            // it is dropped during the same unwind; this blind restore must not
            // reset a mode that was never set.
            deactivate_writer_best_effort(&mut stdout, true, false);
        }
        PanicRestoreTarget::DevTty => {
            if let Ok(mut tty) = open_dev_tty() {
                deactivate_writer_best_effort(&mut tty, true, false);
            }
        }
    }
}

/// xterm modifyOtherKeys level 1: report Shift-Enter and Ctrl-Enter as CSI-u.
const ENABLE_MODIFY_OTHER_KEYS: &[u8] = b"\x1b[>4;1m";
/// Back to the terminal's default modifyOtherKeys level.
const RESET_MODIFY_OTHER_KEYS: &[u8] = b"\x1b[>4;0m";

/// Enters terminal modes for the event loop. Returns whether the
/// modifyOtherKeys fallback was enabled and must be reset on deactivation.
fn activate_writer<W: Write>(writer: &mut W, features: TerminalFeatures) -> anyhow::Result<bool> {
    enable_raw_mode()?;
    execute!(writer, EnterAlternateScreen)?;
    if features.mouse_enabled {
        execute!(writer, EnableMouseCapture)?;
    }

    // Bracketed paste makes multi-line and control-character pastes arrive as
    // one `Event::Paste` instead of letting each character drive normal-mode
    // actions like Enter submit or command-mode entry.
    execute!(writer, EnableBracketedPaste)?;

    Ok(enable_keyboard_modes(writer, features))
}

fn enable_keyboard_modes<W: Write>(writer: &mut W, features: TerminalFeatures) -> bool {
    if features.keyboard_enhancements_supported {
        // REPORT_EVENT_TYPES distinguishes Press from Repeat from Release so
        // the two-press file walk can require an actual key release between
        // presses. Without it, terminals emit Press for every auto-repeat
        // tick, so the walk is turned off and j/k stop at the file's edges.
        let _ = execute!(
            writer,
            PushKeyboardEnhancementFlags(
                KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
                    | KeyboardEnhancementFlags::REPORT_EVENT_TYPES,
            )
        );
        return false;
    }
    // tmux answers the kitty probe with DA1 only, so Shift-Enter would arrive
    // as bare Enter and submit. modifyOtherKeys level 1 makes tmux (extended-keys
    // on, format csi-u) report it as CSI 13;2u, which crossterm parses.
    // Not on Windows, whose consoles have no modifyOtherKeys and could render
    // the raw bytes as text.
    if cfg!(unix) {
        writer
            .write_all(ENABLE_MODIFY_OTHER_KEYS)
            .and_then(|()| writer.flush())
            .is_ok()
    } else {
        false
    }
}

fn disable_keyboard_modes<W: Write>(writer: &mut W, modify_other_keys: bool) {
    let _ = execute!(writer, PopKeyboardEnhancementFlags);
    if modify_other_keys && cfg!(unix) {
        let _ = writer
            .write_all(RESET_MODIFY_OTHER_KEYS)
            .and_then(|()| writer.flush());
    }
}

fn deactivate_writer<W: Write>(
    writer: &mut W,
    mouse_enabled: bool,
    modify_other_keys: bool,
) -> anyhow::Result<()> {
    deactivate_writer_with_raw_mode(writer, mouse_enabled, modify_other_keys, disable_raw_mode)
}

fn deactivate_writer_with_raw_mode<W: Write>(
    writer: &mut W,
    mouse_enabled: bool,
    modify_other_keys: bool,
    disable_raw: impl FnOnce() -> io::Result<()>,
) -> anyhow::Result<()> {
    disable_keyboard_modes(writer, modify_other_keys);
    let _ = execute!(writer, DisableBracketedPaste);
    if mouse_enabled {
        let _ = execute!(writer, DisableMouseCapture);
    }
    let raw_result = disable_raw();
    let leave_result = execute!(writer, LeaveAlternateScreen);
    raw_result?;
    leave_result?;
    Ok(())
}

fn deactivate_writer_best_effort<W: Write>(
    writer: &mut W,
    mouse_enabled: bool,
    modify_other_keys: bool,
) {
    let _ = deactivate_writer(writer, mouse_enabled, modify_other_keys);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deactivate_writer_leaves_alternate_screen_even_when_disable_raw_mode_fails() {
        let mut buf = Vec::new();
        let err = deactivate_writer_with_raw_mode(&mut buf, true, false, || {
            Err(io::Error::other("raw mode failed"))
        })
        .expect_err("should propagate raw mode error");
        assert!(err.to_string().contains("raw mode failed"));

        let mut expected_leave = Vec::new();
        execute!(&mut expected_leave, LeaveAlternateScreen).unwrap();
        assert!(
            buf.windows(expected_leave.len())
                .any(|w| w == expected_leave.as_slice()),
            "LeaveAlternateScreen must still be written when disable_raw_mode fails; got: {:?}",
            String::from_utf8_lossy(&buf)
        );
    }

    #[test]
    fn panic_restore_targets_dev_tty_without_polluting_stdout() {
        let mut stdout = Vec::new();
        let mut tty = Vec::new();

        restore_target_best_effort(PanicRestoreTarget::DevTty, || &mut stdout, || Ok(&mut tty));

        assert!(
            stdout.is_empty(),
            "stdout must stay clean when --stdout renders to /dev/tty"
        );
        let mut expected_leave = Vec::new();
        execute!(&mut expected_leave, LeaveAlternateScreen).unwrap();
        assert!(
            tty.windows(expected_leave.len())
                .any(|w| w == expected_leave.as_slice()),
            "expected LeaveAlternateScreen on /dev/tty; got: {:?}",
            String::from_utf8_lossy(&tty)
        );
    }

    #[test]
    fn panic_restore_is_noop_before_tui_entry() {
        let mut stdout = Vec::new();
        let mut tty = Vec::new();

        restore_target_best_effort(PanicRestoreTarget::None, || &mut stdout, || Ok(&mut tty));

        assert!(stdout.is_empty());
        assert!(tty.is_empty());
    }

    #[test]
    fn panic_restore_writes_to_stdout_for_normal_tui() {
        let mut stdout = Vec::new();
        let mut tty = Vec::new();

        restore_target_best_effort(PanicRestoreTarget::Stdout, || &mut stdout, || Ok(&mut tty));

        assert!(tty.is_empty());
        let mut expected_leave = Vec::new();
        execute!(&mut expected_leave, LeaveAlternateScreen).unwrap();
        assert!(
            stdout
                .windows(expected_leave.len())
                .any(|w| w == expected_leave.as_slice()),
            "expected LeaveAlternateScreen on stdout; got: {:?}",
            String::from_utf8_lossy(&stdout)
        );
    }

    #[test]
    fn terminal_features_selects_panic_restore_target() {
        assert_eq!(
            TerminalFeatures::new().panic_restore_target(),
            PanicRestoreTarget::Stdout
        );
        assert_eq!(
            TerminalFeatures::new()
                .use_dev_tty(true)
                .panic_restore_target(),
            PanicRestoreTarget::DevTty
        );
    }

    fn features(keyboard_enhancements_supported: bool) -> TerminalFeatures {
        TerminalFeatures::new()
            .mouse_enabled(false)
            .keyboard_enhancements_supported(keyboard_enhancements_supported)
    }

    #[test]
    fn kitty_supported_pushes_flags_and_does_not_request_modify_other_keys() {
        let mut out = Vec::new();
        assert!(!enable_keyboard_modes(&mut out, features(true)));
        assert_eq!(out.as_slice(), b"\x1b[>3u");
    }

    #[test]
    fn fallback_requests_modify_other_keys_level_1() {
        let mut out = Vec::new();
        let enabled = enable_keyboard_modes(&mut out, features(false));
        if cfg!(unix) {
            assert!(enabled);
            assert_eq!(out.as_slice(), ENABLE_MODIFY_OTHER_KEYS);
        } else {
            assert!(!enabled);
            assert!(out.is_empty());
        }
    }

    #[test]
    fn release_resets_modify_other_keys_only_when_it_was_enabled() {
        let mut out = Vec::new();
        disable_keyboard_modes(&mut out, true);
        if cfg!(unix) {
            assert_eq!(out.as_slice(), b"\x1b[<1u\x1b[>4;0m");
        } else {
            assert_eq!(out.as_slice(), b"\x1b[<1u");
        }

        let mut out = Vec::new();
        disable_keyboard_modes(&mut out, false);
        assert_eq!(out.as_slice(), b"\x1b[<1u");
    }
}
