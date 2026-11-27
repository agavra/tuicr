# GitHub

tuicr reviews GitHub pull requests through the [GitHub CLI](https://cli.github.com/) (`gh`).

## Setup

Install `gh` and authenticate:

```bash
gh auth login
```

tuicr shells out to `gh` for every GitHub operation and stores no tokens of its own. Reviews are
submitted as whatever account `gh` is logged into. For GitHub Enterprise Server, log `gh` into
that host (`gh auth login --hostname ghe.example.com`); tuicr passes the host to `gh`.

## Open a pull request

```bash
tuicr pr                          # the PR for the branch checked out here
tuicr pr 125                      # a PR on this checkout's repository (a fork's upstream parent)
tuicr pr owner/repo#125           # a PR on another repository
tuicr pr https://github.com/owner/repo/pull/125
tuicr pr 125 --remote upstream    # resolve the repository from a named remote
```

`tuicr` with no arguments opens the review target selector; its Pull Requests tab lists open PRs,
and `r` cycles all open PRs, PRs requesting your review (or the `[forge] pr_search` query when set),
and PRs requesting you by name (leaving out team requests). See [KEYBINDINGS.md](KEYBINDINGS.md#commit-selection--review-target-selector).

GitHub is the fallback forge: a remote whose host no other forge claims is treated as GitHub,
which is how Enterprise hosts are picked up. `--repo-url` overrides detection
([CLI.md](CLI.md#flags)).

When you reopen a pull request you have reviewed before, tuicr preselects the commits newer than
your latest submitted review and marks the commits that review already covered with `✓`
([KEYBINDINGS.md](KEYBINDINGS.md#inline-commit-selector)).

## Submit a review

`:submit` opens a picker with four events:

| Event | Result |
|-------|--------|
| Comment | Posts your inline and review-level comments without changing approval state. |
| Approve | Posts your comments and approves the pull request. |
| Request changes | Posts your comments and requests changes. |
| Draft | Creates a pending review that only you can see, to finish on GitHub. |

`:submit comment`, `:submit approve`, `:submit request-changes` and `:submit draft` skip the
picker. The whole review, including multi-line comments and suggestions (`s` in visual mode), is
created in one request against the head commit you reviewed (in a commit subset, the newest
selected commit). Review-level comments become the
review body. Comments on a commit message cannot be posted inline, so they go into the review body.

## Draft state

`:ready` marks the open draft pull request ready for review (`gh pr ready`), and `:draft` converts
it back (`gh pr ready --undo`). Both ask for confirmation first, since marking ready notifies
reviewers. On success the PR info panel and the `:prs` list drop or show `[draft]` without a reload.

## Existing comments

Unresolved review threads from GitHub show inline in the diff. `:comments all` also shows
resolved and outdated threads, `:comments hide` hides them. Set `show_pr_comments = false` to skip
fetching PR conversation comments, and `show_pr_checks = true` to show CI check rollups
([CONFIG.md](CONFIG.md#options)). A copied review (`y`) lists unresolved threads under
`## Existing GitHub Comments`.

## Troubleshooting

- **`GitHub integration requires gh`**: install the GitHub CLI and run `gh auth login`.
- **`GitHub authentication failed`**: run `gh auth login` for the host named in the message.
- **`GitHub token lacks pull request write permission`**: the `gh` token cannot write reviews to
  this repository; log in with an account or token that can.
- **`Cannot change draft state: you lack write access to this PR`**: `:ready` / `:draft` need
  write access to the pull request (its author or a collaborator with write permission).
- **`You already have a pending review on this PR`**: GitHub allows one pending review per user.
  Submit or discard the existing one on GitHub, then retry.
- **`the selected commit is not part of this PR`**: the PR was force-pushed since you opened it.
  Reload with `:e` and submit again.
