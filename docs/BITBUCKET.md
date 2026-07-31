# Bitbucket

tuicr reviews Bitbucket pull requests the same way it reviews GitHub pull
requests, through the [`bkt`](https://github.com/avivsinai/bitbucket-cli) CLI.

**Bitbucket Cloud only.** Bitbucket Data Center speaks an unrelated REST 1.0
API that tuicr does not implement, so remotes on self-hosted Bitbucket hosts are
not recognized as Bitbucket. See [Data Center](#data-center) below.

## Setup

Install `bkt` and authenticate against Bitbucket Cloud:

```bash
brew install avivsinai/tap/bitbucket-cli
bkt auth login https://bitbucket.org --kind cloud --web-token
```

`--web-token` opens Atlassian's token page and then prompts for the token
locally. Create an **API token with scopes** (not a general API token), select
**Bitbucket** as the application, and grant at least:

- `Account: Read` — required for every command
- `Pull requests: Read` — to open and browse pull requests
- `Pull requests: Write` — only needed to submit reviews from tuicr

Check it worked:

```bash
bkt auth status
```

tuicr shells out to `bkt` for every Bitbucket operation and stores no tokens of
its own. It submits as whatever account `bkt` is logged into.

tuicr always passes the workspace and repository explicitly, so you do **not**
need a `bkt` context for tuicr's sake — though one is convenient for using `bkt`
directly:

```bash
bkt context create work --host bitbucket.org --workspace myteam --set-active
```

## Open a pull request

```bash
tuicr pr 830
```

The target accepts several forms:

| Target | Example |
|--------|---------|
| PR id | `830` |
| PR URL | `https://bitbucket.org/myteam/my-service/pull-requests/830` |
| `host/workspace/repo#id` | `bitbucket.org/myteam/my-service#830` |

A PR URL with trailing path segments — the form Bitbucket puts in your address
bar, like `.../pull-requests/830/my-branch/diff` — works too.

Note that a bare `workspace/repo#830` is **not** treated as Bitbucket. That
shape is ambiguous across forges and resolves to GitHub, so spell out the host
when you need to name a Bitbucket repository explicitly.

tuicr detects the forge from the repository's remotes. A remote routes to
Bitbucket when its host is `bitbucket.org` (or `altssh.bitbucket.org`, the
SSH-over-443 endpoint). Bitbucket is checked before GitLab and GitHub, because
the GitHub parser accepts any host and would otherwise claim your Bitbucket
remotes.

## Submit a review

`:submit` opens a picker. On Bitbucket it offers two events:

| Event | Result |
|-------|--------|
| Comment | Posts your inline and review-level comments without changing approval state. |
| Approve | Posts your comments, then approves the pull request. |

Comments are posted before the approval, so if the approval is rejected — you
may not be a reviewer on the pull request — your feedback is already saved and
tuicr says so rather than reporting a clean failure.

Inline comments land on their lines as Bitbucket inline pull request comments.
Multi-line selections are preserved: Bitbucket's `inline` anchor supports
`start_to` / `start_from`, so a range comment stays a range. Review-level
comments post as general (non-line-anchored) pull request comments.

Because Bitbucket has no "review" object, existing general comments on a pull
request show up in tuicr's review-summary area, and existing inline comments
show up as threads on their lines. Approvals appear through the pull request's
participants rather than as comments.

### Not yet supported

`:submit request-changes` and `:submit draft` return an unsupported-operation
error rather than silently downgrading to a plain comment:

- **Request changes** — Bitbucket Cloud has a request-changes endpoint, but
  tuicr does not drive it yet. Post comments with `:submit`, then request
  changes in Bitbucket.
- **Draft (pending) reviews** — Bitbucket has pending comments, but `bkt` does
  not expose a way to publish them as a batch. Use `:submit` to publish
  directly.

## Data Center

Only Bitbucket Cloud is supported. `bkt` itself handles both, but the two
platforms share almost no API surface: Data Center uses project keys instead of
workspaces, `/rest/api/1.0` instead of `/2.0`, requires comment version numbers
on edits, and expresses "request changes" as a `NEEDS_WORK` participant status.

Rather than half-work against a Data Center host, tuicr does not claim those
remotes at all — `parse_bitbucket_remote_url` accepts `bitbucket.org` only. A
`bitbucket.example.com` remote therefore falls through to the other forge
parsers, and the pull request tab reports no supported remote.

## Limitations and troubleshooting

**Reviewer names, not usernames.** Bitbucket Cloud returns an empty `username`
for every account, so tuicr displays `display_name` (for example, "Example User")
where GitHub would show a handle (`@example-user`). Identity comparisons — "is this PR
awaiting my review", "have I already reviewed this" — use account UUIDs
internally.

**Commits since your last review is approximate.** Bitbucket does not record
which commit an approval was against, so tuicr cannot scope the diff to "what
changed since I approved". The commit selector still works for manual ranges.

**Comment bodies appear in the process list.** `bkt api` has no way to read a
request body from stdin — `-d -` is parsed as literal JSON, not a stdin
sentinel — so tuicr passes comment JSON as a command-line argument. On a shared
machine, other users can see your comment text in `ps` output while the command
runs. GitHub and GitLab submissions use stdin and are not affected.

**Abbreviated commit hashes.** Bitbucket's pull request payloads report 12-char
commit hashes while its commits endpoint reports the full 40. tuicr widens the
short ones when it opens a pull request, at the cost of two extra API calls, so
review sessions stay keyed to a stable SHA.

Common errors:

| Message | Fix |
|---------|-----|
| `Bitbucket integration requires bkt.` | Install `bkt`, then run `bkt auth login https://bitbucket.org --kind cloud --web-token`. |
| `Bitbucket authentication failed.` | Run `bkt auth status`; re-run `bkt auth login` if the token is missing or expired. |
| `your token lacks pull request write access` | Re-run `bkt auth login` with the `Pull requests: Write` scope. |
| `Could not determine the authenticated Bitbucket user.` | Run `bkt auth status`. The reviewer-scoped PR list needs your account UUID. |
| Repeated macOS Keychain prompts | Run `bkt auth doctor`. After `brew upgrade`, the stored token's signing requirement no longer matches the binary; `bkt auth login` recreates it. |

To verify the read path end to end against one of your own pull requests:

```bash
TUICR_BB_WORKSPACE=myteam TUICR_BB_REPO=my-service TUICR_BB_PR=830 \
  cargo test bitbucket_live -- --ignored --nocapture
```

That test is read-only — it never posts a comment or approves anything.
