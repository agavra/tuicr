use std::io::{self, Write};

use crossterm::{
    event::{
        DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
        KeyboardEnhancementFlags, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
    },
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Frame, Terminal, backend::CrosstermBackend};

/// Terminal capabilities enabled for one tuicr TUI session.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TerminalFeatures {
    mouse_enabled: bool,
    keyboard_enhancements_supported: bool,
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
}

impl<W: Write> TerminalSession<W> {
    fn enter(writer: W, features: TerminalFeatures) -> anyhow::Result<Self> {
        let backend = CrosstermBackend::new(writer);
        let mut terminal = Terminal::new(backend)?;
        if let Err(err) = activate_writer(terminal.backend_mut(), features) {
            deactivate_writer_best_effort(terminal.backend_mut(), features.mouse_enabled);
            return Err(err);
        }
        Ok(Self {
            terminal,
            features,
            active: true,
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
        if let Err(err) = activate_writer(self.terminal.backend_mut(), self.features) {
            deactivate_writer_best_effort(self.terminal.backend_mut(), self.features.mouse_enabled);
            return Err(err);
        }
        self.active = true;
        Ok(())
    }

    fn deactivate(&mut self) -> anyhow::Result<()> {
        if !self.active {
            return Ok(());
        }
        deactivate_writer(self.terminal.backend_mut(), self.features.mouse_enabled)?;
        self.active = false;
        Ok(())
    }
}

impl<W: Write> Drop for TerminalSession<W> {
    fn drop(&mut self) {
        if self.active {
            deactivate_writer_best_effort(self.terminal.backend_mut(), self.features.mouse_enabled);
            self.active = false;
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
        session.terminal.clear()?;
        Ok(())
    }
}

impl<W: Write> Drop for TerminalSuspension<'_, W> {
    fn drop(&mut self) {
        let Some(session) = self.session.take() else {
            return;
        };
        if session.activate().is_ok() {
            let _ = session.terminal.clear();
        }
    }
}

/// Restores terminal mode on stdout without requiring a live session object.
///
/// This is intended for panic hooks,
/// where ownership may already be unwinding and the session value might not be
/// reachable.
pub fn restore_stdio_best_effort() {
    let mut stdout = io::stdout();
    deactivate_writer_best_effort(&mut stdout, true);
}

fn activate_writer<W: Write>(writer: &mut W, features: TerminalFeatures) -> anyhow::Result<()> {
    enable_raw_mode()?;
    execute!(writer, EnterAlternateScreen)?;
    if features.mouse_enabled {
        execute!(writer, EnableMouseCapture)?;
    }

    // Bracketed paste makes multi-line and control-character pastes arrive as
    // one `Event::Paste` instead of letting each character drive normal-mode
    // actions like Enter submit or command-mode entry.
    execute!(writer, EnableBracketedPaste)?;

    // REPORT_EVENT_TYPES distinguishes Press from Repeat from Release so the
    // two-press file walk can require an actual key release between presses.
    // Without it, terminals emit Press for every auto-repeat tick,
    // and held j/k could walk past file boundaries.
    if features.keyboard_enhancements_supported {
        let _ = execute!(
            writer,
            PushKeyboardEnhancementFlags(
                KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
                    | KeyboardEnhancementFlags::REPORT_EVENT_TYPES,
            )
        );
    }
    Ok(())
}

fn deactivate_writer<W: Write>(writer: &mut W, mouse_enabled: bool) -> anyhow::Result<()> {
    let _ = execute!(writer, PopKeyboardEnhancementFlags);
    let _ = execute!(writer, DisableBracketedPaste);
    if mouse_enabled {
        let _ = execute!(writer, DisableMouseCapture);
    }
    disable_raw_mode()?;
    execute!(writer, LeaveAlternateScreen)?;
    Ok(())
}

fn deactivate_writer_best_effort<W: Write>(writer: &mut W, mouse_enabled: bool) {
    let _ = deactivate_writer(writer, mouse_enabled);
}
