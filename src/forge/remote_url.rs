//! Remote-URL, SSH-config and error-detail helpers shared by the forge
//! backends. Gerrit keeps its own query-only `trim_url_suffix` because `#` is
//! part of its legacy `/#/c/N/` change URLs.

use std::fs;
use std::path::PathBuf;

/// Cap on the characters kept by [`trim_detail`].
pub(crate) const MAX_DETAIL_CHARS: usize = 400;

pub(crate) fn strip_scheme(value: &str) -> Option<&str> {
    value
        .strip_prefix("https://")
        .or_else(|| value.strip_prefix("http://"))
        .or_else(|| value.strip_prefix("ssh://"))
}

/// Drop any `?query` / `#fragment` and trailing slashes.
pub(crate) fn trim_url_suffix(value: &str) -> &str {
    value
        .split(['?', '#'])
        .next()
        .unwrap_or(value)
        .trim_end_matches('/')
}

/// Strip a trailing `:<port>` from a host, e.g. `example.com:2222` ->
/// `example.com`. `ssh://` remotes commonly carry a non-default SSH port that
/// is meaningless for the HTTPS API host used to build API URLs.
pub(crate) fn strip_port(host: &str) -> &str {
    match host.rsplit_once(':') {
        Some((h, port)) if !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit()) => h,
        _ => host,
    }
}

/// Split an SCP-like remote (`git@host:owner/repo`) into `(host, path)`.
pub(crate) fn parse_scp_like_remote(remote_url: &str) -> Option<(&str, &str)> {
    if remote_url.contains("://") {
        return None;
    }
    let (host_part, path) = remote_url.split_once(':')?;
    if host_part.contains('/') || path.is_empty() {
        return None;
    }
    let host = host_part
        .rsplit_once('@')
        .map(|(_, host)| host)
        .unwrap_or(host_part);
    Some((host, path))
}

/// Resolve an SSH host alias against `config` (the text of `~/.ssh/config`).
/// Only exact `Host` patterns are matched; wildcards, negation, `Match` and
/// `Include` fall through to the alias unchanged.
pub(crate) fn resolve_ssh_hostname_from_config(alias: &str, config: &str) -> String {
    let mut in_block = false;
    for raw in config.lines() {
        let line = raw.split_once('#').map_or(raw, |(before, _)| before).trim();
        if line.is_empty() {
            continue;
        }
        // SSH config separates keyword from value by whitespace or '='.
        let (key, value) = line
            .split_once(|c: char| c.is_whitespace() || c == '=')
            .unwrap_or((line, ""));
        let value = value
            .trim_start_matches(|c: char| c.is_whitespace() || c == '=')
            .trim();

        if key.eq_ignore_ascii_case("Host") {
            in_block = value.split_whitespace().any(|pat| pat == alias);
        } else if key.eq_ignore_ascii_case("Match") {
            in_block = false;
        } else if in_block && key.eq_ignore_ascii_case("HostName") {
            return value.to_string();
        }
    }
    alias.to_string()
}

/// Read `~/.ssh/config` and resolve `alias` through it; the alias itself when
/// the file is unreadable.
pub(crate) fn read_ssh_hostname(alias: &str) -> String {
    let Ok(home) = std::env::var("HOME") else {
        return alias.to_string();
    };
    let Ok(content) = fs::read_to_string(PathBuf::from(home).join(".ssh/config")) else {
        return alias.to_string();
    };
    resolve_ssh_hostname_from_config(alias, &content)
}

/// Keep error detail readable: collapse whitespace and cap the length.
///
/// The cap counts characters, not bytes: an error body can quote non-ASCII
/// names or be an HTML page, and slicing at a byte offset panics when it lands
/// inside a multi-byte character.
pub(crate) fn trim_detail(detail: &str) -> String {
    let collapsed = detail.split_whitespace().collect::<Vec<_>>().join(" ");
    match collapsed.char_indices().nth(MAX_DETAIL_CHARS) {
        Some((end, _)) => format!("{}…", &collapsed[..end]),
        None => collapsed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_scheme_handles_http_https_ssh() {
        assert_eq!(strip_scheme("https://h/x"), Some("h/x"));
        assert_eq!(strip_scheme("http://h/x"), Some("h/x"));
        assert_eq!(strip_scheme("ssh://h/x"), Some("h/x"));
        assert_eq!(strip_scheme("h/x"), None);
    }

    #[test]
    fn trim_url_suffix_drops_query_fragment_and_slashes() {
        assert_eq!(trim_url_suffix("o/r/?a=1"), "o/r");
        assert_eq!(trim_url_suffix("o/r#frag"), "o/r");
        assert_eq!(trim_url_suffix("o/r//"), "o/r");
    }

    #[test]
    fn strip_port_only_removes_numeric_ports() {
        assert_eq!(strip_port("h:2222"), "h");
        assert_eq!(strip_port("h:abc"), "h:abc");
        assert_eq!(strip_port("h:"), "h:");
    }

    #[test]
    fn scp_remote_splits_host_and_path() {
        assert_eq!(
            parse_scp_like_remote("git@h.com:o/r.git"),
            Some(("h.com", "o/r.git"))
        );
        assert_eq!(parse_scp_like_remote("https://h/o/r"), None);
        assert_eq!(parse_scp_like_remote("a/b:c"), None);
        assert_eq!(parse_scp_like_remote("h:"), None);
    }

    #[test]
    fn ssh_config_resolves_exact_host_and_stops_at_match() {
        let cfg = "Host work\n  HostName = real.example.com # c\nMatch all\n  HostName no\nHost other\n  HostName x\n";
        assert_eq!(
            resolve_ssh_hostname_from_config("work", cfg),
            "real.example.com"
        );
        assert_eq!(resolve_ssh_hostname_from_config("nope", cfg), "nope");
        let after_match = "Host a\nMatch all\n  HostName z\n";
        assert_eq!(resolve_ssh_hostname_from_config("a", after_match), "a");
    }

    #[test]
    fn trim_detail_collapses_whitespace_and_caps_by_chars() {
        assert_eq!(trim_detail("a \n b"), "a b");
        let long = "é".repeat(MAX_DETAIL_CHARS + 5);
        let out = trim_detail(&long);
        assert_eq!(out.chars().count(), MAX_DETAIL_CHARS + 1);
        assert!(out.ends_with('…'));
    }
}
