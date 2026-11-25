# e2e coverage

Workflow areas (`e2e/scenarios/<area>/<behaviour>.py`; each file's header comment says what it checks and the PR/issue it covers). The scenarios themselves are not listed here: run `uv run --python 3.12 e2e/run.py --list` for the current set. `e2e/run.py` fails if a scenario directory has no row below.

| Area | Workflow |
|---|---|
| `local` | Local reviews: git/hg/jj targets, empty working-tree rows, commit selector, sessions tab, `-A`/`-p`/`--file --line`, `.tuicrignore`, editor handoff |
| `tree` | File tree: filters, reviewed-file hiding, hit areas, horizontal scroll, width |
| `diff` | Diff view: wrap, side-by-side, scroll, goto/expand context, relative numbers, `:focus` |
| `comments` | Comment input, types, side-by-side anchors, `:clear*` |
| `review` | Reviewed marks, session persistence/merge, quit guards, diff watch |
| `theme` | `:theme` picker, apply/rollback, `--theme` |
| `input` | Prompts, mouse, paste, visual selection, goto guards |
| `help` | Help popup |
| `ui` | Chrome: header/chips, config layout, leader and quit keys, minimal UI, how frames reach the terminal |
| `export` | Markdown export of local and remote threads |
| `syntax` | Syntax colouring, user grammars |
| `cli` | Non-interactive CLI: `review` subcommands, flag validation, update detection, install script |
| `pr` | GitHub PR flows: open, reload, commit selector, threads, submit, editor snapshots, selector |
| `gerrit` | Gerrit: URL, threads, root change, HTTPS trust (reads only) |
| `gitlab` | GitLab MR: URL/host, threads, draft read-back, submit, partial submit |
| `gitea` | Gitea PR: pagination, votes, draft read-back, submit |
| `bitbucket` | Bitbucket Cloud: threads, submit |
| `azure` | Azure DevOps: URL parsing and auth-error path only |

## Known gaps

* **Gerrit and Azure DevOps submit:** Gerrit refuses credentials over `http://`, so a fake needs HTTPS with a certificate tuicr trusts. `e2e.forge.tls_files` + `gerrit_serve(tls=)` provide that on Linux (`SSL_CERT_FILE` feeds the platform verifier); macOS would need a Keychain edit, so a submit scenario would be Linux-only. Azure needs a fake for iterations/threads/PAT or `az rest`. Both forges' reads are covered where a fake exists.
* **Gitea range paths and Azure iteration anchors:** need a two-commit clone plus compare/fetch fakes; unit-tested only.
* **Not driven end to end yet:** Sessions tab `a` toggle, `:messages`/`:version`, mouse wheel/drag, non-Homebrew update install flows (Homebrew `tuicr update` is covered), the rest of `[export]` config keys (`code_context` is covered), `scroll_offset`/`cursor_line`/`search_highlight`/`ignore_whitespace`/`initial_commit_selection`, comment-input editing keys and the rest of `comment_vim` (Insert-mode `Ctrl-W` is covered), PR `:prs`/`:copy-url`.
* **Environment-dependent:** `local/hg_*` need real Mercurial and `local/jj_*` need Jujutsu; both SKIP without them (`E2E_REQUIRE_HG=1`/`E2E_REQUIRE_JJ=1` fail instead). `cli/install_script_without_tty` needs `install.sh` from the `site` branch.
