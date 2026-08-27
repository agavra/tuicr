# Gitea

tuicr reviews Gitea pull requests the same way it reviews GitHub pull requests,
through the [`tea`](https://gitea.com/gitea/tea) CLI.

## Setup

Install `tea` and add a login for your instance:

```bash
tea logins add --name work --url https://gitea.example.com --token <token>
```

The token needs `repository` and `issue` scopes to read pull requests, and write
access to the repository to submit reviews.

tuicr shells out to `tea` for every Gitea operation and stores no tokens of its
own. tuicr submits as whatever account the matching `tea` login is for.

## Open a pull request

```bash
tuicr pr 125
```

The target accepts several forms:

| Target | Example |
|--------|---------|
| PR index | `125` |
| PR URL | `https://gitea.example.com/owner/repo/pulls/125` |
| `host/owner/repo#index` | `gitea.example.com/owner/repo#125` |

Note the plural `/pulls/` in Gitea URLs — GitHub uses the singular `/pull/`,
which is how tuicr tells the two URL shapes apart.

The bare `owner/repo#125` form (no host) resolves to GitHub. Use the
host-qualified form for Gitea.

## Forge detection

tuicr detects the forge from the repository's remotes. A remote routes to Gitea
when either:

- its host contains `gitea`; or
- its host matches a login configured in `tea logins list`.

The second rule is what makes self-hosted instances work: a Gitea server at
`git.example.com` looks like nothing in particular from its hostname alone, so a
configured `tea` login is the signal. Add the login before opening a PR and the
remote is recognized.

## Gitea forks

Forgejo and Codeberg are **not supported**. They are separate projects that
currently serve a compatible API, and tuicr tests against neither.

tuicr does not recognize their hostnames. If you add a `tea` login for such an
instance, its remotes will route to this backend and will most likely work
today — but that is a consequence of the API still being compatible, not a
promise. Expect it to break as the projects diverge, and please don't file
forks' incompatibilities as Gitea bugs.

## Submit a review

`:submit` opens a picker. On Gitea it offers four events:

| Event | Result |
|-------|--------|
| Comment | Posts your inline and review-level comments without changing approval state. |
| Approve | Posts your comments and approves the pull request. |
| Request changes | Posts your comments and sets the review to changes requested. |
| Draft | Files a pending review that only you can see, to finish in the web UI. |

Inline comments land on their lines as review comments. Review-level comments
become the review summary. The whole review is created in a single request, so
comments and the summary post atomically.

### When Gitea requires a summary

Gitea requires a review summary for some events, and **inline comments do not
substitute for one**. Add a summary with `<leader>c` (`;c` by default) — the
same "add review comment" binding used on every other forge. Anything you write
there becomes the review body.

| Event | Needs a summary? |
|-------|------------------|
| Approve | No. A bare approval is fine. |
| Comment | Only if there are no inline comments either. |
| Request changes | **Yes**, always. |
| Draft | **Yes**, always. |

Draft is the surprising one: on GitHub a pending review can hold nothing but
line comments, whereas Gitea refuses it without a body.

tuicr checks all of this before sending, so you get a message naming the fix
rather than Gitea's bare `review event X requires a body`.

One rejection comes from the server itself, since tuicr cannot know it in
advance: you cannot approve or request changes on your own pull request. tuicr
surfaces Gitea's message rather than reporting success.

## Limitations

**Multi-line comments collapse to a single line.** Gitea's review API takes one
line number per comment and has no range form. A comment on a visual selection
posts on the selection's last line, which is where GitHub renders its marker
too. The comment text is never altered.

**Outdated threads are not marked.** Gitea tracks internally whether a
force-push invalidated a review comment but does not expose it on the API, so
tuicr shows every existing thread as current. `:comments unresolved` still hides
resolved ones.

**Review-requested filtering is approximate.** Gitea exposes that filter only on
a cross-repository search endpoint that returns issues rather than pulls, so
those rows carry no branch names and the selector's branch-name filter does not
match against them.

**Commit-range diffs on exotic filenames need a local clone.** Gitea's compare
endpoint returns a raw diff with no structured file list, so tuicr reads file
paths out of the diff headers. Git quotes paths containing control characters or
non-ASCII bytes, which tuicr does not unquote; run tuicr from a clone and the
range diff is computed locally instead.

## Troubleshooting

`tea api` exits 0 for every HTTP response, including errors, so tuicr reads the
status line rather than the exit code. If a request fails you get the status and
Gitea's own message.

Common errors:

| Message | Fix |
|---------|-----|
| ``` `tea` CLI not found. ``` | Install `tea`, then run `tea logins add`. |
| `Not authenticated to Gitea host …` | The token expired or was never added. Re-run `tea logins add`. |
| `Gitea host … refused the request` | The token lacks repository or pull-request scope. |
| `Gitea host … returned not found` | Check the slug, and that the token's account can see the repository. |
| `… did not report head and base commit SHAs` | The instance returned an incomplete pull request; check that it is reachable and not mid-migration. |
