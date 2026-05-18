---
title: All-Files Pristine View Mode
description: Add `tuicr --all-files` for whole-repo annotation distinct from `--file` directory mode
type: adr
status: proposed
created: 2026-05-18
---

# FEAT-0010 All-Files Pristine View Mode

## Context and Problem Statement

tuicr is diff-first: working tree, commit range, GitHub PR. The only non-diff entry point is `--file <path>`, which after #321 also accepts a directory and walks it with `ignore::WalkBuilder`, rendering every line as a new-file addition.

That mode serves "annotate a folder I don't control" workflows, but it doesn't fit whole-repo annotation for a tracked codebase. The walker can include untracked build artifacts when no `.gitignore` is present, and the green `+` gutter on every line implies "review these changes" when the goal is to annotate code that hasn't been touched. Coding agents like Claude Code, Codex, and Cursor want a surface that says "here's the entire codebase, comment on any line, hand the comments back" — a peer to diff modes, not a workaround inside one.

This feature is inspired by [revdiff][REVDIFF]'s `--all-files` mode (umputun's Go-based TUI), which solves the same "annotate every tracked file and feed it to a coding agent" problem in a separate tool. The shape and ergonomics borrow from revdiff; the implementation reuses tuicr's existing review primitives so the new mode composes with vim navigation, comment types, themes, the clipboard export format, and session persistence without changes.

## Decision Drivers

- The clipboard export pipeline (`y` / `:clip` structured markdown) is already what agents consume; the new mode must plug into it unchanged.
- File enumeration should reflect what is under version control, not a filesystem snapshot.
- Rendering must signal "no diff here" so reviewers don't mistake context lines for additions.
- Session persistence must survive a HEAD advance mid-review (e.g. `git pull`) without orphaning comments.
- Scope of this ADR: ship the surface; defer cross-VCS support and structural cleanups to separate ADRs.

## Considered Options

### 1. Extend `--file <dir>` with VCS-aware enumeration

Promote the existing directory walk in `FileBackend` to call `git ls-files` when inside a repo and fall back to `WalkBuilder` otherwise. One flag, two enumeration modes selected at runtime.

Rejected: silently changes the behavior shipped in #321 for users already running `tuicr --file .`, and bundles two different filtering semantics (filesystem vs VCS-tracked) into one backend whose contract is no longer self-evident.

### 2. New `--all-files` flag with new `DiffSource::Pristine` variant

Introduce a first-class `DiffSource::Pristine { included: Vec<PathBuf> }` variant alongside `WorkingTree`, `CommitRange`, and `PullRequest`. Add `VcsBackend::list_tracked_files(&self) -> Result<Vec<PathBuf>>` to the trait, with per-backend implementations.

Rejected for this PR: touches the trait, the enum, persistence, status bar dispatch, and every backend at once. Correct end state, but pairs poorly with shipping the user-visible feature in a reviewable PR.

### 3. New `--all-files` flag, FileBackend extension, deferred structural cleanup

Add `--all-files` / `-A` as a sibling flag. Reuse `FileBackend` with a new `FileMode { Single, Directory, Pristine }` enum and a `new_pristine(paths, root)` constructor. A small helper `vcs::pristine::collect_tracked_paths` shells out to `git ls-files -z`. Session keying piggybacks on the existing `base_commit: String` field via a `"pristine:{HEAD or none}:{path_hash:016x}"` prefix; the persistence layer prefix-matches on reload so an advancing HEAD does not orphan comments.

## Decision Outcome

Chosen: option 3.

`--all-files` / `-A` is mutually exclusive with `-r`, `-w`, `--file`. File enumeration is git-only; non-git invocation returns `NotARepository` with a tailored hint. Every file renders with `LineOrigin::Context` and `@@ -1,N +1,N @@` hunk headers; status badges (`M`/`A`/`D`) are suppressed in the file list and per-file diff headers, since the files are not changed. Side-by-side view is suppressed in pristine mode because it would render identical panes; the `:diff` command no-ops with a status message in pristine mode. The status bar gains a `PRISTINE · <short-sha> · N files` chip so reviewers always know which mode they are in and which HEAD their comments are anchored against. The clipboard export format is unchanged.

The `.tuicrignore` filter is applied after enumeration, matching every other mode's behavior, so users can elide tracked-but-noisy files (lockfiles, generated docs) from the view surface without affecting git's view of the tracked set.

Pristine session keys reuse the existing `base_commit: String` field with a `pristine:<head>:<path_hash>` prefix; the persistence layer prefix-matches on reload, ignoring the head segment so an advancing HEAD still resolves to the same session. Two pristine sessions over different path subsets (different `<path_hash>`) persist independently. The single-file `--file` mode keeps its existing `"file"` sentinel and does not collide.

## Consequences

- [+] Whole-repo annotation is a first-class mode. The previous workaround — synthesizing a diff against git's empty tree via `commit-tree 4b825dc6...` and `tuicr -r <empty>..HEAD` — is no longer needed.
- [+] Every existing feature composes for free: vim navigation, comment types, clipboard export, theme rendering, session persistence, file-tree expansion.
- [+] Coexists with the merged `--file <dir>` mode from #321. Users keep both surfaces and pick based on intent.
- [-] jj and Mercurial users see "pristine mode requires a git repository." Cross-VCS support is a separate decision; a hard error is preferable to silently producing a different file set.
- [-] One additional `git ls-files` subprocess on startup. Negligible cost for repos up to ~10k tracked files; larger repos may want async enumeration in a follow-up.

## Future Considerations

Candidate follow-up ADRs. Each decision boundary is independent and any subset can land in any order.

- Promote pristine to a typed `DiffSource::Pristine { included: Vec<PathBuf> }` variant with a dedicated `SessionDiffSource::Pristine` payload. Migrates the prefix-matched string keys on load and lets `header_source_chunk` dispatch on the variant instead of the `is_pristine_mode` flag.
- Hoist enumeration into a `VcsBackend::list_tracked_files(&self) -> Result<Vec<PathBuf>>` trait method so `--all-files` works on jj (`jj file list`) and any future backend. Mercurial stays explicit `UnsupportedOperation` until the maintainers decide whether `hg manifest` should drive it.
- Extract pristine rendering into a shared renderer so `--include <prefix>` / `--exclude <prefix>` filters compose without duplicating render code.
- Measure single-file focus mode for large codebases. tuicr renders every file into one continuous scroll today; whether that holds up for pristine views of >100-file repos is an empirical question worth investigating before adding a focus toggle.

[REVDIFF]: https://github.com/umputun/revdiff "umputun/revdiff -- TUI for reviewing diffs, files, and documents with inline annotations"
