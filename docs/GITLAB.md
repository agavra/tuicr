# GitLab

tuicr reviews GitLab merge requests the same way it reviews GitHub pull
requests, through the `glab` CLI.

## Setup

Install the [GitLab CLI](https://gitlab.com/gitlab-org/cli) and authenticate:

```bash
glab auth login
```

tuicr shells out to `glab` for every GitLab operation and stores no tokens of
its own. tuicr submits as whatever account `glab` is logged into.

## Open a merge request

```bash
tuicr mr 125
```

`mr` is an alias for `pr`, so `tuicr mr 125`, `tuicr pr 125`, and `tuicr tui mr 125`
all open the same review. The target accepts several forms:

| Target | Example |
|--------|---------|
| MR IID | `125` |
| MR URL | `https://gitlab.com/owner/repo/-/merge_requests/125` |
| `owner/repo#iid` | `mygroup/myrepo#125` |
| `host/owner/repo#iid` | `git.example.com/mygroup/myrepo#125` |

Nested groups and subgroups work in the `owner/repo` slot, for example
`mygroup/subgroup/myrepo#125`.

tuicr detects the forge from the repository's remotes. A remote routes to GitLab
when its host contains `gitlab` or matches a self-hosted host configured in
`glab`. Everything else falls back to GitHub.

## Submit a review

`:submit` opens a picker. On GitLab it offers three events:

| Event | Result |
|-------|--------|
| Comment | Posts your inline and review-level comments without changing approval state. |
| Approve | Posts your comments and approves the merge request. |
| Request changes | Posts your comments and sets the MR reviewer state to changes requested. |

Inline comments land on their lines as merge request discussion notes.
Review-level comments post as the review summary.

Request changes requires that your account is an assigned reviewer on the merge
request. GitLab only lets a reviewer set that state. If you are not a reviewer,
tuicr surfaces the error GitLab returns rather than reporting success. tuicr does
not add you as a reviewer on your behalf.

`:submit draft` remains GitHub-only. Run it against a GitLab MR and tuicr returns
an unsupported-operation error rather than submitting.

## Self-hosted GitLab

Authenticate against your instance once:

```bash
glab auth login --hostname git.example.com
```

After that, tuicr recognizes remotes on `git.example.com` as GitLab because the
host is configured in `glab`. Pass the full URL or a `host/owner/repo#iid`
target, and tuicr builds the `https://git.example.com/owner/repo` repository
argument for `glab` on its own.

## SSH over port 443

If your network only allows SSH on port 443, GitLab's `altssh.gitlab.com`
endpoint works. tuicr maps `altssh.gitlab.com` back to `gitlab.com` when it
reads your remote, so the MR resolves against the real host. `HostName` aliases
in `~/.ssh/config` are resolved the same way.

## Limitations and troubleshooting

Reviewing a commit range within an MR requires a local checkout of the branch.
A remote-only commit-range diff returns a "not yet supported" error.

For verbose tracing of the `glab` calls tuicr makes, set `TUICR_GLAB_DEBUG=1`.
tuicr appends each interaction to `/tmp/tuicr-glab-debug.log`.

Common errors:

| Message | Fix |
|---------|-----|
| `GitLab integration requires glab.` | Install the GitLab CLI, then run `glab auth login`. |
| `GitLab authentication failed.` | Run `glab auth login` for the host named in the error. |
| `GitLab token lacks merge request write permission.` | Authenticate with an account or token that has merge request write access. |
