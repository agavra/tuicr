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

#[test]
fn should_warn_instead_of_panicking_on_grammars_that_crash_syntect() {
    // given grammars whose `push: [[]]` / `escape: 5` panic syntect's loader
    let dir = tempfile::tempdir().unwrap();
    for (name, rule) in [
        ("PushEmpty", "push: [[]]"),
        ("EscapeInt", "push: other\n      escape: 5"),
    ] {
        let grammar = format!(
            "%YAML 1.2\n---\nname: {name}\nscope: source.{name}\nfile_extensions: [{name}]\ncontexts:\n  main:\n    - match: a\n      {rule}\n  other:\n    - match: b\n"
        );
        std::fs::write(dir.path().join(format!("{name}.sublime-syntax")), grammar).unwrap();
    }

    // when they are loaded
    let warnings = load_user_syntaxes(dir.path());

    // then each is a warning naming its file, and highlighting still works
    assert_eq!(warnings.len(), 2, "{warnings:?}");
    assert!(warnings[0].contains("EscapeInt"));
    assert!(warnings[1].contains("PushEmpty"));
    let _ = SyntaxHighlighter::default();
}
