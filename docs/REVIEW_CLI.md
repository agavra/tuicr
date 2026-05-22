# Review Session CLI

`tuicr review` exposes persisted review sessions without opening the TUI. It is
intended for scripts and coding agents that need to inspect or update tuicr's
saved review state.

Session arguments accept either:

- a slug from `tuicr review list`
- a direct path to a session JSON file

## Commands

```bash
tuicr review list --repo .
tuicr review comments --session agavra/tuicr@main/worktree
```

`--repo` defaults to the current directory and is only used when resolving a
session slug. Direct session JSON paths do not need a repo.

All `tuicr review` commands emit JSON by default. Timestamps are RFC3339 strings
so callers can parse them without locale-specific handling.

## Add Comments

Use flags for quick manual comments:

```bash
tuicr review add --session agavra/tuicr@main/worktree \
  --target-file src/main.rs \
  --line 42 \
  --side new \
  --type issue \
  "Handle the empty case here."
```

Target flags:

- omit `--target-file` for a review-level comment
- pass `--target-file <path>` for a file-level comment
- add `--line <n>` for a line comment
- add `--end-line <n>` for a range comment
- use `--side old|new` for inline comments

## JSON Input

For machine input, pass a JSON payload with `--input`. The value can be literal
JSON, `@path/to/payload.json`, or `-` to read stdin.

```bash
tuicr review add --session agavra/tuicr@main/worktree --input - <<'JSON'
{
  "type": "issue",
  "content": "Handle the empty case here.",
  "file": "src/main.rs",
  "line": 42,
  "side": "new"
}
JSON
```

Flat JSON fields:

- `content`: required comment text
- `type` or `comment_type`: comment classification, defaults to `note`
- `file`: file path; omit for a review-level comment
- `line`: line number for a line comment
- `start_line` and `end_line`: range bounds
- `side`: `old` or `new`, defaults to `new`

Nested targets are also accepted:

```json
{
  "comment_type": "suggestion",
  "content": "This range can be simplified.",
  "target": {
    "type": "line_range",
    "file": "src/main.rs",
    "start_line": 10,
    "end_line": 14,
    "side": "old"
  }
}
```

Target types:

- `review`
- `file`
- `line`
- `line_range` or `range`

## Output

`comments` returns a JSON array:

```json
[
  {
    "id": "79c9b3e1-0a7a-4efe-9d43-f7085d7c1a82",
    "location": "src/main.rs:42",
    "path": "src/main.rs",
    "start_line": 42,
    "end_line": 42,
    "side": "new",
    "comment_type": "issue",
    "lifecycle_state": "local_draft",
    "created_at": "2026-05-22T17:20:00Z",
    "content": "Handle the empty case here."
  }
]
```
