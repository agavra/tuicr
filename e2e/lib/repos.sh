# e2e/lib/repos.sh - fixture repository builders. Each takes a target dir; the harness env (core.sh) isolates git/hg config.
_fx_git() { git -C "$1" -c user.name=E2E -c user.email=e2e@example.invalid -c commit.gpgsign=false "${@:2}"; }

# fx_git_repo <dir>: history of 3 commits on main, plus staged + unstaged work.
#  c1 base files; c2 modify/add/delete/rename/binary; c3 more edits.
#  Layout: README.md, src/main.rs, src/lib.rs(no trailing newline), notes_cjk.txt,
#  tabs.txt, data.bin; then working tree changes.
fx_git_repo() {
  local d=$1; rm -rf "$d"; mkdir -p "$d/src"; git init -q -b main "$d"
  printf '# demo\n\nfirst line\n' > "$d/README.md"
  printf 'fn main() {\n    println!("hi");\n}\n' > "$d/src/main.rs"
  printf 'pub fn a() -> u32 { 1 }' > "$d/src/lib.rs"             # no trailing newline
  printf 'old name\nto be deleted\n' > "$d/old_name.txt"
  printf 'delete me\nline\n' > "$d/gone.txt"
  printf '\x00\x01\x02binary\x00' > "$d/data.bin"
  _fx_git "$d" add -A; _fx_git "$d" commit -qm "base"
  # c2: modify, add, delete, rename, binary change
  printf '# demo\n\nfirst line\nsecond line\n' > "$d/README.md"
  printf 'pub fn a() -> u32 { 2 }\n' > "$d/src/lib.rs"           # gains newline
  printf '日本語のテキスト\n中文内容 mixed with ascii\n' > "$d/notes_cjk.txt"
  printf 'a\tb\tc\n\tindented\twith\ttabs\n' > "$d/tabs.txt"
  _fx_git "$d" rm -q gone.txt
  _fx_git "$d" mv old_name.txt new_name.txt
  printf '\x00\x01\x03changed\x00' > "$d/data.bin"
  _fx_git "$d" add -A; _fx_git "$d" commit -qm "feat: touch every file kind"
  # c3
  printf 'fn main() {\n    println!("hello");\n}\n' > "$d/src/main.rs"
  _fx_git "$d" commit -qam "fix: greeting"
  # working tree: one staged, one unstaged, one untracked
  printf 'staged addition\n' > "$d/staged.txt"; _fx_git "$d" add staged.txt
  printf '# demo\n\nfirst line\nsecond line\nunstaged line\n' > "$d/README.md"
  printf 'untracked\n' > "$d/untracked.txt"
}

# fx_git_merge_base <dir>: for `-r A...B`. Branch topic forks from main, both advance.
# Sets FX_BASE (fork point), FX_MAIN, FX_TOPIC (shas).
fx_git_merge_base() {
  local d=$1; rm -rf "$d"; mkdir -p "$d"; git init -q -b main "$d"
  printf 'base\n' > "$d/shared.txt"; _fx_git "$d" add -A; _fx_git "$d" commit -qm base
  FX_BASE=$(_fx_git "$d" rev-parse HEAD)
  _fx_git "$d" checkout -q -b topic
  printf 'topic change\n' > "$d/topic.txt"; _fx_git "$d" add -A; _fx_git "$d" commit -qm "topic work"
  FX_TOPIC=$(_fx_git "$d" rev-parse HEAD)
  _fx_git "$d" checkout -q main
  printf 'main change\n' > "$d/mainonly.txt"; _fx_git "$d" add -A; _fx_git "$d" commit -qm "main moves on"
  FX_MAIN=$(_fx_git "$d" rev-parse HEAD)
}

# fx_pr_repo <dir>: small git repo whose origin is github.com/octo/demo, for
# `tuicr pr 1` against the fake gh (canned PR #1 touches README.md, src/greet.rs).
fx_pr_repo() {
  local d=$1; rm -rf "$d"; mkdir -p "$d"; git init -q -b main "$d"
  printf '# demo\nold line\n' > "$d/README.md"
  _fx_git "$d" add -A; _fx_git "$d" commit -qm base
  _fx_git "$d" remote add origin https://github.com/octo/demo.git
}

# e2e_use_hg: put real Mercurial first on PATH (E2E_HG_DIR, else the `hg` on PATH). Sapling's `hg` alias is rejected.
# Without it the scenario is SKIPPED, unless E2E_REQUIRE_HG=1 (CI), where it FAILS.
e2e_use_hg() {
  local dir
  for dir in ${E2E_HG_DIR:-} "$(dirname "$(command -v hg 2>/dev/null || echo /nonexistent/hg)")"; do
    if PATH="$dir:$PATH" hg --version 2>/dev/null | grep -qi 'mercurial distributed'; then
      export PATH="$dir:$PATH" E2E_PRE_PATH="$dir"; return 0
    fi
  done
  [ -z "${E2E_REQUIRE_HG:-}" ] || { echo "e2e: real Mercurial required (E2E_REQUIRE_HG=1) but not found; set E2E_HG_DIR" >&2; exit 1; }
  e2e_skip "real Mercurial not found (set E2E_HG_DIR; Sapling's hg does not count)"
}

# fx_hg_repo <dir>: needs e2e_use_hg first. Modified z, added a, removed m, untouched k.
fx_hg_repo() {
  local d=$1; rm -rf "$d"; mkdir -p "$d"
  ( cd "$d" && hg init -q . && printf 'z_old\n' > z.txt && printf 'm_old\n' > m.txt && printf 'k\n' > k.txt \
      && hg add -q . && hg commit -q -u e2e -m base \
      && printf 'z_new\n' > z.txt && printf 'a_added\n' > a.txt && hg add -q a.txt && hg rm -qf m.txt )
}

# fx_wide_repo <dir>: two commits. cjk.txt gains long CJK / VS16-emoji lines (to overflow a side-by-side pane), plus a deeply nested
# long-named file and a long directory name (to overflow the file tree). Review with `-r HEAD~1..HEAD`.
fx_wide_repo() {
  local r=$1; rm -rf "$r"; mkdir -p "$r/deep/dir" "$r/zzzz_a_rather_long_directory_name_here"; git init -q -b main "$r"
  printf 'ascii one\nascii two\nascii three\n' > "$r/cjk.txt"
  printf 'x\n' > "$r/deep/dir/an_extremely_long_file_name_for_the_tree_pane.txt"
  printf 'x\n' > "$r/zzzz_a_rather_long_directory_name_here/b.txt"
  _fx_git "$r" add -A; _fx_git "$r" commit -qm base
  python3 - "$r/cjk.txt" <<'PY'
import sys
open(sys.argv[1], 'w').write('ascii one\n' + '日本語のテキスト変更' * 8 + ' wide-tail-W\n' + 'emoji ' + '❤️✔️' * 20 + ' tail-E vs16\n'
    + 'ascii three changed ' + 'x' * 80 + '\n' + 'a' + '日本語のテキスト変更' * 8 + '\n')
PY
  printf 'y\n' > "$r/deep/dir/an_extremely_long_file_name_for_the_tree_pane.txt"
  printf 'y\n' > "$r/zzzz_a_rather_long_directory_name_here/b.txt"
  _fx_git "$r" add -A; _fx_git "$r" commit -qm change
}

# fx_long_file_repo <dir>: fx_git_repo plus a committed, unchanged 120-line long.txt ("line N"), for --file/--line.
fx_long_file_repo() {
  local d=$1; fx_git_repo "$d"
  seq 1 120 | sed 's/^/line /' > "$d/long.txt"
  _fx_git "$d" add long.txt; _fx_git "$d" commit -qm "add long.txt"
}

# e2e_use_jj: put the real `jj` first on PATH (E2E_JJ_DIR, else the one on PATH) and give it an identity.
# Without jj the scenario is SKIPPED, unless E2E_REQUIRE_JJ=1 (CI), where it FAILS.
e2e_use_jj() {
  local dir
  for dir in ${E2E_JJ_DIR:-} "$(dirname "$(command -v jj 2>/dev/null || echo /nonexistent/jj)")"; do
    if PATH="$dir:$PATH" jj --version >/dev/null 2>&1; then
      export PATH="$dir:$PATH" JJ_USER=E2E JJ_EMAIL=e2e@example.invalid JJ_CONFIG=/dev/null
      E2E_PRE_PATH="$dir${E2E_PRE_PATH:+:$E2E_PRE_PATH}"; E2E_ENV="${E2E_ENV:-} JJ_USER=E2E JJ_EMAIL=e2e@example.invalid JJ_CONFIG=/dev/null"; return 0
    fi
  done
  [ -z "${E2E_REQUIRE_JJ:-}" ] || { echo "e2e: jj required (E2E_REQUIRE_JJ=1) but not found; set E2E_JJ_DIR" >&2; exit 1; }
  e2e_skip "jj not found (set E2E_JJ_DIR)"
}

# fx_jj_repo <dir>: needs e2e_use_jj. Pure jj repo: "base" (a.txt, b.txt), "second" (a.txt edited, b.txt deleted, c.txt added),
# then a working-copy change (a.txt edited again, d.txt added).
fx_jj_repo() {
  local d=$1; rm -rf "$d"; mkdir -p "$d"
  ( cd "$d" && jj --quiet git init . \
      && printf 'a one\n' > a.txt && printf 'b one\n' > b.txt && jj --quiet describe -m base \
      && jj --quiet new -m second && printf 'a one\na two\n' > a.txt && rm b.txt && printf 'c one\n' > c.txt \
      && jj --quiet new -m wip && printf 'a one\na two\na wip\n' > a.txt && printf 'd wip\n' > d.txt ) >/dev/null
}
