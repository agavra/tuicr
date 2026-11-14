//! Its own test binary (see `user_syntaxes.rs`): the process-wide set is filled once.

use std::path::Path;
use tuicr::syntax::{SyntaxHighlighter, load_user_syntaxes};

fn grammar(name: &str, body: &str) -> String {
    format!(
        "%YAML 1.2\n---\nname: {name}\nscope: source.{name}\nfile_extensions: [{name}]\ncontexts:\n{body}"
    )
}

#[test]
fn should_warn_on_crashing_grammars_but_load_valid_ones() {
    let dir = tempfile::tempdir().unwrap();
    let bad = [
        ("PushEmpty", "  main:\n    - match: a\n      push: [[]]\n"),
        ("SetEmpty", "  main:\n    - match: a\n      set: [[]]\n"),
        (
            "EscapeInt",
            "  main:\n    - match: a\n      push: other\n      escape: 5\n  other:\n    - match: b\n",
        ),
    ];
    for (name, body) in bad {
        std::fs::write(
            dir.path().join(format!("{name}.sublime-syntax")),
            grammar(name, body),
        )
        .unwrap();
    }
    // a valid grammar with a context named `escape`
    std::fs::write(
        dir.path().join("EscCtx.sublime-syntax"),
        grammar(
            "EscCtx",
            "  main:\n    - match: a\n      push: escape\n  escape:\n    - match: b\n      scope: keyword.b\n      pop: true\n",
        ),
    )
    .unwrap();

    let warnings = load_user_syntaxes(dir.path());

    assert_eq!(warnings.len(), 3, "{warnings:?}");
    for name in ["EscapeInt", "PushEmpty", "SetEmpty"] {
        assert!(warnings.iter().any(|w| w.contains(name)), "{warnings:?}");
    }
    let lines = vec!["ab".to_string()];
    let highlighted =
        SyntaxHighlighter::default().highlight_file_lines(Path::new("x.EscCtx"), &lines);
    assert!(highlighted.is_some_and(|l| l[0].is_some()));
}
