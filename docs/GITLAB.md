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
`mygroup/subgroup/myrepo#125`. A three-segment `a/b/c#iid` is read as
`host/owner/repo` only when `a` contains a `.`; use a full URL for single-label
hosts.

tuicr detects the forge from the repository's remotes. A remote routes to GitLab
when its host contains `gitlab` or matches `glab`'s default host
(`glab config get host`); other hosts you logged into with `glab` are not
checked. Everything else falls back to GitHub. A full
`/-/merge_requests/<iid>` URL is read as GitLab on any host.

## Submit a review

`:submit` opens a picker. On GitLab it offers four events:

| Event | Result |
|-------|--------|
| Comment | Posts your inline and review-level comments without changing approval state. |
| Approve | Posts your comments and approves the merge request. |
| Request changes | Posts your comments and sets the MR reviewer state to changes requested. |
| Draft | Posts your comments as GitLab draft notes without publishing them or changing approval state. |

Inline comments land on their lines as merge request discussion notes.
Review-level comments post as the review summary.

Request changes requires that your account is an assigned reviewer on the merge
request. GitLab only lets a reviewer set that state. If you are not a reviewer,
tuicr surfaces the error GitLab returns rather than reporting success. tuicr does
not add you as a reviewer on your behalf.

`:submit draft` uses GitLab draft notes, the closest primitive to a GitHub pending
review: your inline and review-level comments are stored but not published. Publish
them yourself from GitLab's own "Submit review" control on the merge request.

A review is posted as several requests (one note per comment, then the approval
or reviewer state). If one fails midway, the comments already posted are locked
so a retry does not post them twice, and tuicr reports the partial submit rather
than a clean failure.

## Draft state

`:ready` marks the open draft merge request ready (`glab mr update --ready`), and `:draft` marks
it as a draft again (`glab mr update --draft`). Both ask for confirmation first.

## Self-hosted GitLab

Authenticate against your instance once, and make it `glab`'s default host
unless its name contains `gitlab`:

```bash
glab auth login --hostname git.example.com
glab config set host git.example.com
```

After that, tuicr recognizes remotes on `git.example.com` as GitLab because it
is `glab`'s default host. Only the default host is compared, so a remote on a
second neutral-named instance is not recognized; use a full MR URL for it. Pass the full URL or a `host/owner/repo#iid`
target, and tuicr builds the `https://git.example.com/owner/repo` repository
argument for `glab` on its own.

### Instances under a relative URL

If GitLab is served from a subfolder rather than the domain root
(`https://git.example.com/gitlab/`), tell `glab` about it:

```bash
glab config set subfolder gitlab --host git.example.com
```

Nested paths work too (`apps/gitlab`), and surrounding slashes are ignored.
`GITLAB_SUBFOLDER` overrides the setting for every self-hosted host (never for
`gitlab.com`).

`glab` prepends that subfolder to every API URL it builds, and tuicr reads the
setting so it does not repeat the subfolder inside the project id. Without it,
tuicr has no way to tell the subfolder apart from a top-level group in your
remote URL, and every API call 404s — including the one that fetches remote
comments.

SSH remotes are unaffected: they never carry the subfolder.

## SSH over port 443

If your network only allows SSH on port 443, GitLab's `altssh.gitlab.com`
endpoint works. tuicr maps `altssh.gitlab.com` back to `gitlab.com` when it
reads your remote, so the MR resolves against the real host. `HostName` aliases
in `~/.ssh/config` are resolved the same way.

For a self-hosted instance whose SSH endpoint has its own hostname, set it as
the default host's `ssh_host` in `glab`. tuicr maps a remote on that host back
to the default host (for example `gitlabssh.example.org` to
`gitlab.example.org`), provided the SSH hostname contains `gitlab`.

## Limitations and troubleshooting

- Merge requests with more than 10,000 commits fail to open with an explicit error rather than showing a partial commit list.
- Merge requests with more than 10,000 changed files fail to open with an error.
- Discussions, draft notes and diff versions are read up to 10,000 entries each; anything beyond that is silently left out.

Reviewing a commit range within an MR requires a local checkout of the branch.
A remote-only commit-range diff returns a "not yet supported" error.

For verbose tracing of the `glab` calls tuicr makes, set `TUICR_GLAB_DEBUG=1`.
tuicr appends each interaction to `glab-debug.log` in its per-user cache
directory (`~/.cache/tuicr/` on Linux), readable only by you.

Common errors:

| Message | Fix |
|---------|-----|
| `GitLab integration requires glab.` | Install the GitLab CLI, then run `glab auth login`. |
| `GitLab authentication failed.` | Run `glab auth login` for the host named in the error. |
| `GitLab token lacks merge request write permission.` | Authenticate with an account or token that has merge request write access. |
| `404 Not Found` on an instance served from a subfolder | Set `glab config set subfolder <path> --host <host>`. See [Instances under a relative URL](#instances-under-a-relative-url). |
