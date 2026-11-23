# Library API

tuicr is also a Rust library for tools that build on its persisted review sessions. Add it with
`cargo add tuicr`.

`ReviewStore` lists the sessions saved for a checkout or forge repository, loads a session, and
adds review, file, line, or range comments through the same insertion primitive as the TUI
(`add_comment_to_session`). It also deletes comments and whole reviews. Writes take the same
storage lock and atomic rename as the TUI, so a running TUI picks the new comment up when it next
polls or reloads.

```rust no_run
use tuicr::{AddCommentRequest, CommentTarget, CommentType, LineSide, ReviewStore};

fn main() -> tuicr::Result<()> {
    let store = ReviewStore::new();
    let sessions = store.list_sessions_for_repo("/path/to/repo")?;
    let session = &sessions[0].session_ref;

    store.add_comment(
        session,
        AddCommentRequest {
            target: CommentTarget::Line {
                path: "src/main.rs".into(),
                line: 42,
                side: LineSide::New,
            },
            content: "Handle the empty case here.".into(),
            comment_type: CommentType::from_id("issue"),
            author: "my-tool".into(),
            commit_id: None,
        },
    )?;
    Ok(())
}
```

Notes:

- `list_sessions_for_repo` takes the same selector as `tuicr review --repo`: a checkout path or a
  forge coordinate such as `owner/repo` ([REVIEW_CLI.md](REVIEW_CLI.md#the---repo-selector)).
- `author` is the name shown on the comment. Pick one that identifies your tool.
- `commit_id` scopes a comment to one commit. Leave it `None` unless the comment is about a single
  commit of a multi-commit review.
- `CommentType::from_id("none")` (or `CommentType::None`) adds an untyped comment.
- `ReviewStore::with_reviews_dir` points the store at another reviews directory, which is useful
  for tests.

This example is compiled as a doctest (`cargo test --doc`), so it stays in sync with the API.
