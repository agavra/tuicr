# e2e/lib/forge.sh - fake-forge fixtures and payload readers.
# sc_forge_open <name> <remote-url> [variant]: fixture repo whose origin is <remote-url> (the URL picks the fake forge:
# github.com -> gh, gitlab.com -> glab, gitea.example.com -> tea, bitbucket.org -> bkt), `tuicr pr 1`, wait for the PR to load.
# Extra env for the fake / child: E2E_ENV. variant = data/github/<variant>/ overlay (default none).
sc_forge_open() {
  n=$1; d=$(e2e_dir "$n"); mkdir -p "$d"; fx_pr_repo "$d/repo"; git -C "$d/repo" remote set-url origin "$2"
  E2E_ENV="${3:+E2E_VARIANT=$3 }${E2E_ENV:-}" E2E_CWD="$d/repo" e2e_start "$n" "$TUICR_BIN" --no-update-check pr 1
  e2e_wait_for "$n" 'PR Description|Merge Request|!1' 20
}
# sc_pr_open <name> [variant]: GitHub PR #1 via fake gh with fixture variant (data/github/<variant>/, default: threads_and_commits).
sc_pr_open() { sc_forge_open "$1" https://github.com/octo/demo.git "${2:-threads_and_commits}"; }
# sc_posts <name>: print the stdin payload of every review POST in the fake-gh log, one JSON per line.
sc_posts() {
  python3 - "$E2E_FORGE_LOG" <<'PY'
import json, sys
for l in open(sys.argv[1]):
    j = json.loads(l)
    a = j["argv"]
    if j["tool"] == "gh" and "--method" in a and a[a.index("--method") + 1] == "POST":
        print(json.dumps({"path": [x for x in a if x.startswith("repos/")][0], "payload": json.loads(j["stdin"] or "null")}))
PY
}
# sc_forge_posts <tool>: one JSON per mutating call in the fake's log: {"argv": [...], "payload": <stdin json or null>}.
# A call mutates when its argv has a POST method (`--method POST`, `-X POST`, `--method=POST`) or is a `bkt pr approve`.
sc_forge_posts() {
  python3 - "$E2E_FORGE_LOG" "$1" <<'PY'
import json, sys
for l in open(sys.argv[1]):
    j = json.loads(l)
    a = j["argv"]
    post = any(a[i] in ("--method", "-X") and a[i + 1] == "POST" for i in range(len(a) - 1)) or "--method=POST" in a
    if j["tool"] == sys.argv[2] and (post or a[:2] == ["pr", "approve"]):
        body = j.get("stdin") or (a[a.index("--input") + 1] if "--input" in a else None)   # bkt passes the body as an argument
        print(json.dumps({"argv": a, "payload": json.loads(body) if body else None}))
PY
}
# sc_pr_comment <name> <text>: in the opened PR, add a line comment on src/greet.rs line 1.
sc_pr_comment() {
  e2e_keys "$1" '}'; sc_cursor_on "$1" '1[╭ ]▌ pub fn greet'
  e2e_keys "$1" c; e2e_wait_for "$1" COMMENT; e2e_keys "$1" "$2"; e2e_keys "$1" Enter; e2e_wait_for "$1" "$2"
}
# sc_pr_editor_snapshot <name> [remote-url]: `e` on a PR file the checkout lacks makes tuicr write a read-only snapshot and
# open $EDITOR on it. A fake editor (data/fake_editor.py) records the snapshot's path and the permissions of every directory
# above it into $d/editor.argv (waited for); then check what a private snapshot must look like: keyed by the full head sha,
# base directory 0700 and owned by us, file read-only.
sc_pr_editor_snapshot() {
  local n=$1 url=${2:-https://github.com/octo/demo.git}; d=$(e2e_dir "$n"); mkdir -p "$d"; rm -f "$d/editor.argv"
  cp "$E2E_DIR/data/fake_editor.py" "$d/editor.sh"; chmod +x "$d/editor.sh"
  E2E_ENV="EDITOR=$d/editor.sh" sc_forge_open "$n" "$url" threads_and_commits
  e2e_keys "$n" '}'; sc_cursor_on "$n" '1[╭ ]▌ pub fn greet'
  e2e_keys "$n" e
  e2e_wait_file "$d/editor.argv" 10 || { echo "fake editor was not run"; e2e_screen "$n"; return 1; }
  cat "$d/editor.argv"
  local file; file=$(sed -n 's/^FILE //p' "$d/editor.argv")
  echo "$file" | grep -Eq 'pr1-2{40}/' || { echo "snapshot dir is not keyed by the full head sha: $file"; return 1; }
  grep -m1 '^DIR ' "$d/editor.argv" | while read -r _ path mode uid me; do
    [ "$mode" = 700 ] && [ "${uid#uid=}" = "${me#me=}" ] || { echo "snapshot base $path is not private (mode $mode $uid $me)"; exit 1; }
  done
  grep -q '^FILEMODE 4' "$d/editor.argv" || { echo "snapshot file is not read-only"; return 1; }
}
# sc_gerrit_open <name> <root|threads>: change 1 of project `proj` served by fakeforge/gerrit.py (anonymous, GERRIT_URL
# on 127.0.0.1); origin is a Gerrit SSH remote, both commits exist locally so no fetch is needed.
#  root:    single parentless commit (a.txt), change has no parents -> base is the empty tree.
#  threads: base + head; a.txt line 1 thread resolved by a reply, line 2 thread reopened by a reply.
# Env: E2E_GERRIT_HOST, E2E_GERRIT_PROJECT, E2E_GERRIT_TARGET (PR target, @PORT@ = the fake's port).
# The server is stopped by a cleanup registered here (it also dies with the scenario's process group).
sc_gerrit_open() {
  n=$1; local kind=$2; d=$(e2e_dir "$n"); mkdir -p "$d"; local r=$d/repo gd=$d/gerrit
  local project=${E2E_GERRIT_PROJECT:-proj}
  rm -rf "$r" "$gd"; mkdir -p "$r" "$gd"; git init -q -b main "$r"
  local base=""
  if [ "$kind" = threads ]; then
    printf 'base one\nbase two\n' > "$r/a.txt"; _fx_git "$r" add -A; _fx_git "$r" commit -qm base; base=$(_fx_git "$r" rev-parse HEAD)
  fi
  printf 'line one\nline two\nline three\n' > "$r/a.txt"
  _fx_git "$r" add -A; _fx_git "$r" commit -qm "Add a.txt"; local head; head=$(_fx_git "$r" rev-parse HEAD)
  _fx_git "$r" remote add origin "ssh://${E2E_GERRIT_HOST:-gerrit.example.test}:29418/$project"
  local parents='[]'; [ -z "$base" ] || parents="[{\"commit\": \"$base\"}]"
  sed -e "s|@PROJECT@|$project|" -e "s|@HEAD@|$head|g" -e "s|@PARENTS@|$parents|" "$E2E_DIR/data/gerrit/change.json.tpl" > "$gd/change.json"
  cp "$E2E_DIR/data/gerrit/comments.json" "$gd/comments.json"
  python3 "$E2E_DIR/fakeforge/gerrit.py" "$gd" "$gd/port" & echo $! > "$d/gerrit.pid"
  e2e_on_exit "sc_gerrit_stop $n"
  e2e_wait_file "$gd/port" 10 || return 1
  local port; port=$(cat "$gd/port"); local target=${E2E_GERRIT_TARGET:-1}; target=${target//@PORT@/$port}
  E2E_ENV="GERRIT_URL=http://127.0.0.1:$port ${E2E_ENV:-}" E2E_CWD="$r" e2e_start "$n" "$TUICR_BIN" --no-update-check pr "$target"
}
sc_gerrit_stop() { kill "$(cat "$(e2e_dir "$1")/gerrit.pid")" 2>/dev/null || true; }
