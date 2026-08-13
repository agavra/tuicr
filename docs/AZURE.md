# Azure DevOps

tuicr can review **Azure DevOps Git pull requests** the same way it reviews GitHub PRs and
GitLab MRs: open a PR, read its diff, leave inline comments, and push the review back.

tuicr talks to the
[Azure DevOps REST API](https://learn.microsoft.com/en-us/rest/api/azure/devops/git/pull-requests)
and picks its transport automatically:

- **Personal Access Token (preferred).** If `AZURE_DEVOPS_EXT_PAT` (or `AZURE_DEVOPS_PAT`) is
  set, tuicr calls the REST API directly over HTTPS with the PAT (HTTP Basic auth).
- **Azure CLI fallback.** Otherwise it shells out to `az rest`, reusing your `az login` AAD
  session — like the GitHub backend reuses `gh` and GitLab reuses `glab`.

**Prefer the PAT.** Many enterprise tenants don't provision the Azure DevOps AAD application,
so `az rest` fails there with `AADSTS500011: The resource principal … was not found in the
tenant`. A PAT sidesteps that entirely.

## Setup

Create a PAT in Azure DevOps (User settings → Personal access tokens) with **Code: Read &
Write** scope for the organization, then set it in your environment:

```bash
# bash/zsh
export AZURE_DEVOPS_EXT_PAT=xxxxxxxx...

# PowerShell (persist for your user)
setx AZURE_DEVOPS_EXT_PAT "xxxxxxxx..."
```

`AZURE_DEVOPS_EXT_PAT` is the same variable the `azure-devops` CLI extension uses, so if
you already use `az repos` it may already be set.

Alternatively, if your tenant allows AAD access, just install the
[Azure CLI](https://learn.microsoft.com/en-us/cli/azure/install-azure-cli) and run
`az login` — no PAT needed.

## Reviewing a PR

Run tuicr from inside a **local clone** of the Azure repo (see the diff note below), then:

```bash
tuicr pr 125                                                  # by PR id
tuicr pr https://dev.azure.com/org/project/_git/repo/pullrequest/125   # by URL
```

`*.visualstudio.com` URLs work too and are normalized to `dev.azure.com` internally.

Inside tuicr, review as usual (`j`/`k` to move, `c` to comment, `v` for a range comment),
then `:submit` to push:

- **Comment** — posts your inline comments as Azure PR comment threads and your review-level
  comment as a general PR comment.
- **Approve** — posts comments and casts an *Approved* vote (10).
- **Request changes** — posts comments and casts a *Rejected* vote (-10).

Existing comment threads (yours and other reviewers') are fetched when you open the PR and
render inline on their lines; PR-level (conversation) comments show in the review summary.
After you `:submit`, press `:e` to re-fetch and see your just-posted comments come back as
threads. `az`/system threads (pushes, policy, votes) are filtered out.

## The local-clone requirement

Azure DevOps exposes no single "unified diff" endpoint, so tuicr builds the PR diff from a
local clone with `git diff <base>...<head>`. Run tuicr from a checkout of the repo, and make
sure the PR's base and source commits are fetched (`git fetch origin`). If they aren't
present locally, tuicr will tell you to fetch and retry.

File content for context expansion is read from the local clone first, falling back to the
REST API when a revision isn't present locally.

## MVP limitations

This is the first Azure DevOps slice. Not yet supported:

- **Commit-subrange selection** requires both commits to be present in the local clone.
- **Draft reviews** — Azure DevOps has no draft-review primitive, so `:submit draft` behaves
  like Comment and posts immediately.
- **Azure DevOps Server (on-prem)** — only the cloud (`dev.azure.com` / `*.visualstudio.com`)
  is targeted.

## Troubleshooting

- **"…needs either a PAT (set AZURE_DEVOPS_EXT_PAT) or the Azure CLI (`az`)."** — set a PAT
  (recommended) or install the Azure CLI and run `az login`.
- **PAT rejected** — the token is expired or lacks **Code: Read & Write** scope for the org;
  create a new one.
- **`AADSTS500011: The resource principal … was not found in the tenant`** — your tenant
  hasn't provisioned the Azure DevOps AAD app, so `az rest` can't get a token. Use a PAT
  (`AZURE_DEVOPS_EXT_PAT`) instead.
- **"Could not diff …in the local checkout."** — `git fetch origin` so the PR's base and
  source commits exist locally, then retry.
