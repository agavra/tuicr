# Gerrit

tuicr can review **Gerrit changes** the same way it reviews GitHub PRs and GitLab MRs: open a
change, read its diff, leave inline comments, and push the review back. A Gerrit change takes the
place of a pull request, and its change number is the "PR number" everywhere in the UI.

Gerrit ships no companion CLI (the other backends wrap `gh`, `glab`, `bkt`, and `az`), so tuicr
talks to the [Gerrit REST API](https://gerrit-review.googlesource.com/Documentation/rest-api.html)
directly over HTTPS.

## Setup

Gerrit authenticates the REST API with an **HTTP password**, not your account password. Generate
one in Gerrit under *Settings → HTTP Credentials*, then:

```bash
# bash/zsh
export GERRIT_USERNAME=jdoe
export GERRIT_PASSWORD=xxxxxxxx...

# PowerShell (persist for your user)
setx GERRIT_USERNAME "jdoe"
setx GERRIT_PASSWORD "xxxxxxxx..."
```

Without credentials tuicr still reads public changes anonymously, but `:submit` needs them.

## How tuicr recognizes a Gerrit remote

Gerrit is always self-hosted, so there is no reserved domain to key on the way `github.com` or
`bitbucket.org` work. tuicr treats a remote as Gerrit when any of these hold:

1. The remote uses Gerrit's canonical SSH port **29418** —
   `ssh://jdoe@review.internal:29418/platform/base`. This is the strongest signal and needs no
   configuration.
2. `GERRIT_URL` names the host.
3. The hostname contains **"gerrit"** — `https://gerrit.example.com/platform/base`. (The same
   heuristic the GitLab backend uses for self-hosted instances.)

If your Gerrit lives on a neutral hostname *and* you clone over HTTPS, set `GERRIT_URL` to the
server root:

```bash
export GERRIT_URL=https://review.internal
```

## Which server tuicr talks to

With `GERRIT_URL` unset, tuicr assumes the REST API is served over HTTPS at the root of the git
remote's own host — `ssh://jdoe@review.internal:29418/platform/base` implies
`https://review.internal`. That is the stock Gerrit layout and needs no configuration.

**When `GERRIT_URL` is set it wins**, for every Gerrit repo. It is the override for the
deployments a git remote cannot describe:

| Situation | Set |
| --- | --- |
| Gerrit under a path prefix | `GERRIT_URL=https://review.internal/gerrit` |
| REST API on a non-default port | `GERRIT_URL=https://review.internal:8443` |
| Plain HTTP | `GERRIT_URL=http://review.internal` |
| Web host differs from the SSH gateway in the remote | `GERRIT_URL=https://review.corp.com` |

That last row is the one you cannot express any other way: with a remote of
`ssh://gerrit-ssh.corp.com:29418/platform/base`, nothing in the URL names the web host.

A value without a scheme (`review.internal`, `review.internal:8443`) or an SSH URL pasted from
`git remote -v` (`ssh://jdoe@review.internal:29418`) is normalized onto HTTPS. A `29418` port is
dropped — that is Gerrit's *SSH* port and means nothing over HTTPS — but any other port you write
is kept. Use the full `http://…` form if your REST API is not on HTTPS.

Note that `GERRIT_URL` is a process-global environment variable, so it names *one* server. If you
review on two Gerrits, scope it per checkout (direnv, a shell wrapper) or leave it unset where the
hostnames already carry the signal. Your review sessions are still keyed by the *remote's* host, so
setting the variable never changes which saved session a checkout resolves to.

The `/a/` prefix Gerrit uses for authenticated git clones (`https://host/a/my/project`) is
transport, not part of the project name, and is stripped automatically.

## Reviewing a change

Run tuicr from inside a **local clone** of the project (see the diff note below), then:

```bash
tuicr pr 3965                                            # by change number
tuicr pr https://gerrit.example.com/c/platform/base/+/3965   # by URL
tuicr pr https://gerrit.example.com/#/c/3965/                # legacy URL form
```

The **Pull Requests** tab lists open changes for the project your remote points at; `r` toggles
between all open changes and the ones **waiting on you**.

That second list is driven by Gerrit's **attention set** — the same signal behind the *Your Turn*
section of the Gerrit dashboard — as `attention:self -owner:self`. The alternative,
`reviewer:self`, matches every change you are a reviewer on including the ones you already voted
on and handed back to the author, which is the opposite of "needs my review". `-owner:self` drops
your own changes: the attention set holds those too when a reviewer replies, but that is your turn
to *answer*, not to review.

Both operators need credentials, so an anonymous session shows all open changes for either
setting of the toggle.

Inside tuicr, review as usual (`j`/`k` to move, `c` to comment, `v` for a range comment), then
`:submit` to push:

- **Comment** — posts your inline comments and your review-level comment as the change message.
- **Approve** — the same, plus a `Code-Review +2` vote.
- **Request changes** — the same, plus a `Code-Review -1` vote. (`-2` is a hard veto in Gerrit,
  stronger than what "request changes" means on the other forges, so tuicr does not cast it.)
- **Draft** — stores every inline comment as a Gerrit *draft* comment, and the review-level
  comment as a draft on Gerrit's `/PATCHSET_LEVEL` pseudo-file. Nothing is published until you
  hit **Reply** in the Gerrit web UI.

Publishing a review passes `drafts: KEEP`, so drafts you left in the Gerrit web UI are not swept
up by a tuicr submit.

Existing comments are fetched when you open the change and render inline on their lines. Comments
left on an earlier patch set still show, marked outdated, matching the Gerrit web UI. Comments on
Gerrit's magic paths (`/COMMIT_MSG`, `/MERGE_LIST`, `/PATCHSET_LEVEL`) have no line in the file
diff and are not rendered.

## The local-clone requirement

Gerrit's `/revisions/{id}/patch` endpoint returns a patch ordered by git's rename-aware diff
queue, while its file list is keyed and sorted by path — the two cannot be paired positionally the
way tuicr's `pair_metadata_with_patch` requires, and tuicr does not read file paths out of patch
text. So the diff is built from a local clone with `git diff <base>..<head>`, which yields
authoritative `git diff --raw` metadata. Azure DevOps takes the same route.

A Gerrit change lives in `refs/changes/*`, which a normal clone does not carry, so tuicr fetches
the change's patch-set ref first. That is a bare `git fetch <remote> refs/changes/NN/CCCC/P`: it
writes `FETCH_HEAD` only, creating no branch and moving no existing ref in your repo.

File content for context expansion is read from the local clone first, falling back to the REST
API when a revision isn't present locally.

## MVP limitations

This is the first Gerrit slice. Not yet supported:

- **Patch-set comparison.** tuicr reviews the current patch set against its parent commit. There
  is no "diff patch set 2 against 4" selector — a Gerrit change is a single commit, so the inline
  commit selector shows exactly one entry.
- **The `Code-Review` label is assumed.** Installs that rename it, or that need a different label
  (`Verified`, a custom gate), cannot vote from tuicr yet.
- **Replying to a thread.** Existing comments are read-only in tuicr, as on every other forge.
- **SSH transport.** Only the REST API is used; `ssh -p 29418 host gerrit review` is not called.
- **Gerrit 3.3+ for the `r` toggle.** The attention set was introduced in Gerrit 3.3; an older
  server rejects `attention:self` outright. The rest of the backend has no such floor.

## Troubleshooting

- **"Gerrit needs authentication for this request."** — set `GERRIT_USERNAME` and
  `GERRIT_PASSWORD`.
- **"Gerrit rejected the credentials…"** — `GERRIT_PASSWORD` must be the HTTP password from
  *Settings → HTTP Credentials*, not your account password.
- **The Pull Requests tab says there is no supported remote** — your Gerrit host shows none of the
  three signals above. Set `GERRIT_URL` to the server root.
- **Requests go to the wrong host** — the error names the server actually contacted. If that is
  your SSH gateway rather than the web host, set `GERRIT_URL`; if it is a *different* Gerrit than
  the one you meant, you have a stale `GERRIT_URL` exported in that shell.
- **`Unsupported operator attention:self` after pressing `r`** — your Gerrit predates 3.3 and has
  no attention set. The unfiltered list still works; toggle back with `r`.
- **"Could not find <base>..<head> in the local checkout."** — the patch-set fetch failed. Run
  `git fetch origin refs/changes/…` (the ref is named in the error) by hand and retry; that
  usually surfaces the real credential or network problem.
- **`applying label Code-Review: +2 is restricted`** — your account cannot cast `+2` on this
  project. Use Comment instead, and vote from the web UI.
