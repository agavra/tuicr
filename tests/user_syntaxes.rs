//! Its own test binary because `load_user_syntaxes` fills a process-wide set once,
//! which would leak into every other test sharing the process.

use std::path::Path;
use tuicr::syntax::{SyntaxHighlighter, load_user_syntaxes};

const PROBE_GRAMMAR: &str = "%YAML 1.2
---
name: Probe
scope: source.probe
file_extensions: [tuicrprobe]
contexts:
  main:
    - match: probe
      scope: keyword.probe
";

#[test]
fn should_highlight_with_a_grammar_from_the_syntaxes_directory() {
    // given a file type no embedded grammar knows
    let path = Path::new("example.tuicrprobe");
    let lines = vec!["probe".to_string()];
    assert!(
        SyntaxHighlighter::default()
            .highlight_file_lines(path, &lines)
            .is_none()
    );

    // when a grammar for it is loaded from the syntaxes directory
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("Probe.sublime-syntax"), PROBE_GRAMMAR).unwrap();
    assert!(load_user_syntaxes(dir.path()).is_empty());

    // then highlighters built afterwards resolve and highlight it
    let highlighted = SyntaxHighlighter::default().highlight_file_lines(path, &lines);
    assert!(highlighted.is_some_and(|lines| lines[0].is_some()));
}
