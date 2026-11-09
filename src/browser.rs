use std::process::Command;

/// Run the system URL opener without interpreting the URL as a shell command.
pub fn open_url(url: &str, output_to_stdout: bool) -> anyhow::Result<()> {
    let mut command = browser_command(url)?;
    // Keep launcher output out of exported review markdown.
    #[cfg(unix)]
    if output_to_stdout {
        use std::fs::OpenOptions;
        use std::process::Stdio;
        command
            .stdin(Stdio::from(OpenOptions::new().read(true).open("/dev/tty")?))
            .stdout(Stdio::from(
                OpenOptions::new().write(true).open("/dev/tty")?,
            ))
            .stderr(Stdio::from(
                OpenOptions::new().write(true).open("/dev/tty")?,
            ));
    }
    #[cfg(not(unix))]
    let _ = output_to_stdout;
    let status = command.status()?;
    anyhow::ensure!(status.success(), "URL opener exited with {status}");
    Ok(())
}

fn browser_command(url: &str) -> anyhow::Result<Command> {
    let uri = url.parse::<ureq::http::Uri>()?;
    anyhow::ensure!(
        matches!(uri.scheme_str(), Some("http" | "https"))
            && uri.host().is_some_and(|host| !host.is_empty())
            && !url.contains(['\\', '"']),
        "PR URL must be an absolute HTTP(S) URL"
    );
    #[cfg(target_os = "windows")]
    let mut command = {
        let mut command = Command::new("rundll32.exe");
        command.arg("url.dll,FileProtocolHandler");
        command
    };
    #[cfg(target_os = "macos")]
    let mut command = Command::new("open");
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let mut command = Command::new("xdg-open");
    command.arg(url);
    Ok(command)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_non_web_or_malformed_urls() {
        for url in [
            "",
            "--help",
            "file:///tmp/review",
            "javascript:alert(1)",
            "https:///pull/1",
            "https://",
            "https://host/\nfoo",
            "https://host/\"foo",
            "https://host/\\foo",
        ] {
            assert!(browser_command(url).is_err(), "accepted {url:?}");
        }
    }

    #[test]
    fn passes_web_url_as_one_literal_argument() {
        for url in [
            "https://github.com/owner/repo/pull/42",
            "http://gitlab.example:8080/group/repo/-/merge_requests/1",
            "https://host/review?x=1&y=$(touch%20/tmp/nope)#discussion",
        ] {
            let command = browser_command(url).unwrap();
            assert_eq!(command.get_args().last().unwrap(), url);
            #[cfg(target_os = "windows")]
            assert_eq!(command.get_args().count(), 2);
            #[cfg(not(target_os = "windows"))]
            assert_eq!(command.get_args().count(), 1);
        }
    }
}
