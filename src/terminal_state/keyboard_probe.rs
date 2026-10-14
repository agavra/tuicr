//! Startup probe for kitty keyboard protocol support.
//!
//! crossterm's `supports_keyboard_enhancement` sends `CSI ? u` followed by a
//! Primary Device Attributes query (`CSI c`). Once the `CSI ? u` reply arrives
//! it reads the DA1 reply with no timeout, so a terminal that answers the
//! first query but not the second blocks startup forever
//! (crossterm-rs/crossterm#1133). This probe sends the same two queries but
//! bounds every wait.

/// Returns whether the terminal reports kitty keyboard protocol support.
///
/// Call this before the TUI enters raw mode. It never blocks for longer than
/// a couple of seconds, and any I/O error counts as "unsupported".
pub fn supports_keyboard_enhancement() -> bool {
    #[cfg(unix)]
    {
        unix::probe().unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        // crossterm does not query the terminal on Windows, so there is no
        // reply to wait on.
        matches!(
            crossterm::terminal::supports_keyboard_enhancement(),
            Ok(true)
        )
    }
}

/// Tracks the replies to the probe queries in the bytes read from the tty.
#[cfg_attr(not(unix), allow(dead_code))]
#[derive(Debug, Default)]
struct ReplyScanner {
    /// Bytes from an escape sequence that has not been fully read yet.
    pending: Vec<u8>,
    /// The terminal answered `CSI ? u` with its keyboard enhancement flags.
    flags: bool,
    /// The terminal answered `CSI c` with its primary device attributes.
    device_attributes: bool,
}

#[cfg_attr(not(unix), allow(dead_code))]
enum PrivateCsi {
    /// `ESC [ ?`, parameters and the final byte; `len` covers all of it.
    Complete { len: usize, final_byte: u8 },
    /// A prefix of a private CSI sequence; more bytes are needed.
    Incomplete,
    /// Not a private CSI sequence.
    Other,
}

/// Parses `ESC [ ? <digits and ;> <final>` at the start of `bytes`.
#[cfg_attr(not(unix), allow(dead_code))]
fn parse_private_csi(bytes: &[u8]) -> PrivateCsi {
    const PREFIX: &[u8] = b"\x1b[?";
    let prefix_len = bytes.len().min(PREFIX.len());
    if bytes[..prefix_len] != PREFIX[..prefix_len] {
        return PrivateCsi::Other;
    }
    if bytes.len() < PREFIX.len() {
        return PrivateCsi::Incomplete;
    }
    let params = bytes[PREFIX.len()..]
        .iter()
        .take_while(|byte| byte.is_ascii_digit() || **byte == b';')
        .count();
    match bytes.get(PREFIX.len() + params) {
        Some(&final_byte) => PrivateCsi::Complete {
            len: PREFIX.len() + params + 1,
            final_byte,
        },
        None => PrivateCsi::Incomplete,
    }
}

#[cfg_attr(not(unix), allow(dead_code))]
impl ReplyScanner {
    /// Consumes bytes read from the terminal. Anything that is not one of the
    /// two replies, such as a key typed during startup, is dropped.
    fn feed(&mut self, bytes: &[u8]) {
        self.pending.extend_from_slice(bytes);
        let mut pos = 0;
        while let Some(offset) = self.pending[pos..].iter().position(|&b| b == 0x1b) {
            let start = pos + offset;
            match parse_private_csi(&self.pending[start..]) {
                PrivateCsi::Complete { len, final_byte } => {
                    match final_byte {
                        b'u' => self.flags = true,
                        b'c' => self.device_attributes = true,
                        _ => {}
                    }
                    pos = start + len;
                }
                PrivateCsi::Incomplete => {
                    pos = start;
                    self.pending.drain(..pos);
                    return;
                }
                PrivateCsi::Other => pos = start + 1,
            }
        }
        self.pending.clear();
    }

    /// The probe result, once the terminal has answered both queries.
    ///
    /// Terminals reply in order, so a DA1 reply without a flags reply before
    /// it means the keyboard protocol is unsupported.
    fn verdict(&self) -> Option<bool> {
        self.device_attributes.then_some(self.flags)
    }
}

#[cfg(unix)]
mod unix {
    use std::fs::OpenOptions;
    use std::io::{self, Read, Write};
    use std::os::fd::{AsRawFd, RawFd};
    use std::time::{Duration, Instant};

    use crossterm::terminal::{disable_raw_mode, enable_raw_mode};

    use super::ReplyScanner;

    /// Kitty keyboard flags query, then Primary Device Attributes.
    const QUERY: &[u8] = b"\x1b[?u\x1b[c";

    /// Longest the probe waits for any reply, matching crossterm's timeout.
    const PROBE_TIMEOUT: Duration = Duration::from_secs(2);

    /// How long to keep reading for the DA1 reply once the flags reply is in.
    /// The DA1 reply is only drained so it does not reach the event loop, and
    /// terminals that send it do so right behind the flags reply.
    const DA1_GRACE: Duration = Duration::from_millis(100);

    pub(super) fn probe() -> io::Result<bool> {
        let mut tty = OpenOptions::new().read(true).write(true).open("/dev/tty")?;
        enable_raw_mode()?;
        let result = tty
            .write_all(QUERY)
            .and_then(|()| tty.flush())
            .and_then(|()| await_replies(&mut tty, PROBE_TIMEOUT, DA1_GRACE));
        disable_raw_mode()?;
        result
    }

    /// Reads probe replies from `input` until the terminal has answered both
    /// queries or the time runs out.
    ///
    /// No reply within `timeout` counts as unsupported. Once the flags reply
    /// has arrived the answer is known, so the wait for DA1 is cut to `grace`.
    pub(super) fn await_replies<R: Read + AsRawFd>(
        input: &mut R,
        timeout: Duration,
        grace: Duration,
    ) -> io::Result<bool> {
        let mut scanner = ReplyScanner::default();
        let mut deadline = Instant::now() + timeout;
        let mut buf = [0u8; 256];
        loop {
            if let Some(supported) = scanner.verdict() {
                return Ok(supported);
            }
            let now = Instant::now();
            if now >= deadline {
                return Ok(scanner.flags);
            }
            if !wait_readable(input.as_raw_fd(), deadline - now)? {
                continue;
            }
            let read = match input.read(&mut buf) {
                Ok(0) => return Ok(scanner.flags),
                Ok(read) => read,
                Err(err) if err.kind() == io::ErrorKind::Interrupted => continue,
                Err(err) => return Err(err),
            };
            let had_flags = scanner.flags;
            scanner.feed(&buf[..read]);
            if scanner.flags && !had_flags {
                deadline = deadline.min(Instant::now() + grace);
            }
        }
    }

    /// Waits until `fd` is readable or `timeout` passes.
    ///
    /// Uses `select` rather than `poll` because macOS `poll` does not support
    /// tty devices.
    fn wait_readable(fd: RawFd, timeout: Duration) -> io::Result<bool> {
        if fd < 0 || fd as usize >= libc::FD_SETSIZE {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "file descriptor out of range for select",
            ));
        }
        let mut timeout = libc::timeval {
            tv_sec: timeout.as_secs() as libc::time_t,
            tv_usec: timeout.subsec_micros() as libc::suseconds_t,
        };
        // SAFETY: `fd_set` is plain data, `fd` was checked against
        // FD_SETSIZE above, and every pointer passed to `select` refers to a
        // live local.
        let ready = unsafe {
            let mut readable: libc::fd_set = std::mem::zeroed();
            libc::FD_ZERO(&mut readable);
            libc::FD_SET(fd, &mut readable);
            libc::select(
                fd + 1,
                &mut readable,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &mut timeout,
            )
        };
        match ready {
            -1 => {
                let err = io::Error::last_os_error();
                if err.kind() == io::ErrorKind::Interrupted {
                    Ok(false)
                } else {
                    Err(err)
                }
            }
            0 => Ok(false),
            _ => Ok(true),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FLAGS_REPLY: &[u8] = b"\x1b[?0u";
    const DA1_REPLY: &[u8] = b"\x1b[?62;22c";

    fn scan(chunks: &[&[u8]]) -> ReplyScanner {
        let mut scanner = ReplyScanner::default();
        for chunk in chunks {
            scanner.feed(chunk);
        }
        scanner
    }

    #[test]
    fn flags_then_device_attributes_means_supported() {
        let scanner = scan(&[FLAGS_REPLY, DA1_REPLY]);
        assert_eq!(scanner.verdict(), Some(true));
    }

    #[test]
    fn device_attributes_alone_means_unsupported() {
        let scanner = scan(&[DA1_REPLY]);
        assert_eq!(scanner.verdict(), Some(false));
    }

    #[test]
    fn flags_without_device_attributes_is_not_a_verdict_yet() {
        let scanner = scan(&[FLAGS_REPLY]);
        assert!(scanner.flags);
        assert_eq!(scanner.verdict(), None);
    }

    #[test]
    fn replies_split_across_reads_are_reassembled() {
        let scanner = scan(&[b"\x1b", b"[?", b"1", b"5u\x1b[?6", b"2;22", b"c"]);
        assert_eq!(scanner.verdict(), Some(true));
    }

    #[test]
    fn keys_typed_during_the_probe_are_ignored() {
        let scanner = scan(&[b"j\x1b[A\x1bk", FLAGS_REPLY, b"q", DA1_REPLY]);
        assert_eq!(scanner.verdict(), Some(true));
    }

    #[test]
    fn other_private_sequences_are_not_replies() {
        let scanner = scan(&[b"\x1b[?1049h\x1b[?25l"]);
        assert!(!scanner.flags);
        assert_eq!(scanner.verdict(), None);
    }

    #[cfg(unix)]
    mod unix_replies {
        use std::io::Write;
        use std::os::unix::net::UnixStream;
        use std::time::{Duration, Instant};

        use super::super::unix::await_replies;
        use super::{DA1_REPLY, FLAGS_REPLY};

        fn run(reply: &[u8], timeout: Duration) -> (bool, Duration) {
            let (mut terminal, mut tuicr) = UnixStream::pair().unwrap();
            terminal.write_all(reply).unwrap();
            let started = Instant::now();
            let supported = await_replies(&mut tuicr, timeout, Duration::from_millis(50)).unwrap();
            (supported, started.elapsed())
        }

        #[test]
        fn missing_device_attributes_reply_does_not_hang() {
            // The terminal from #751: it answers `CSI ? u` but never `CSI c`.
            let (supported, elapsed) = run(FLAGS_REPLY, Duration::from_secs(30));
            assert!(supported);
            assert!(elapsed < Duration::from_secs(5), "waited {elapsed:?}");
        }

        #[test]
        fn both_replies_answer_without_waiting() {
            let reply = [FLAGS_REPLY, DA1_REPLY].concat();
            let (supported, elapsed) = run(&reply, Duration::from_secs(30));
            assert!(supported);
            assert!(elapsed < Duration::from_secs(5), "waited {elapsed:?}");
        }

        #[test]
        fn device_attributes_only_is_unsupported() {
            let (supported, _) = run(DA1_REPLY, Duration::from_secs(30));
            assert!(!supported);
        }

        #[test]
        fn silent_terminal_times_out_as_unsupported() {
            let (supported, elapsed) = run(b"", Duration::from_millis(50));
            assert!(!supported);
            assert!(elapsed >= Duration::from_millis(50));
        }
    }
}
