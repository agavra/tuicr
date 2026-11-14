"""Fixture repository builders (git, hg, jj). Each takes a target dir and rebuilds it from scratch; the harness
environment (core.py) isolates git/hg/jj configuration."""

from __future__ import annotations

import os
import shutil
import subprocess
from collections.abc import Callable
from pathlib import Path
from typing import TYPE_CHECKING, NamedTuple

from .core import Failure, Skip, grep

if TYPE_CHECKING:
    from .core import Scenario


def git(repo: str | Path, *args: str) -> str:
    """`git -C repo` with a fixed identity and no signing; returns stripped stdout, fails on a non-zero exit."""
    p = subprocess.run(
        [
            "git",
            "-C",
            str(repo),
            "-c",
            "user.name=E2E",
            "-c",
            "user.email=e2e@example.invalid",
            "-c",
            "commit.gpgsign=false",
            *args,
        ],
        stdin=subprocess.DEVNULL,
        capture_output=True,
        text=True,
    )
    if p.returncode != 0:
        raise Failure(f"git {' '.join(args)} failed in {repo}", p.stdout + p.stderr)
    return p.stdout.strip()


def run(cwd: str | Path, *argv: str) -> str:
    """Run a fixture command (hg, jj, ...) in cwd, output captured; fails on a non-zero exit."""
    p = subprocess.run(list(argv), cwd=str(cwd), stdin=subprocess.DEVNULL, capture_output=True, text=True)
    if p.returncode != 0:
        raise Failure(f"{' '.join(argv)} failed in {cwd}", p.stdout + p.stderr)
    return p.stdout.strip()


def write(path: str | Path, content: str | bytes) -> Path:
    """Write a file (parents created); str is UTF-8 with no newline translation."""
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(content if isinstance(content, bytes) else content.encode("utf-8"))
    return path


def fresh(d: str | Path) -> Path:
    """An empty directory `d` (removed first)."""
    d = Path(d)
    shutil.rmtree(d, ignore_errors=True)
    d.mkdir(parents=True)
    return d


def _fresh_git(d: str | Path) -> Path:
    d = fresh(d)
    subprocess.run(["git", "init", "-q", "-b", "main", str(d)], check=True)
    return d


def git_repo(d: str | Path) -> Path:
    """History of 3 commits on main, plus staged + unstaged work.

    c1 base files; c2 modify/add/delete/rename/binary; c3 more edits. Layout: README.md, src/main.rs,
    src/lib.rs (no trailing newline), notes_cjk.txt, tabs.txt, data.bin; then a staged staged.txt, an unstaged
    README.md edit and an untracked untracked.txt. `-r HEAD~2..HEAD` shows 8 files."""
    d = _fresh_git(d)
    write(d / "README.md", "# demo\n\nfirst line\n")
    write(d / "src/main.rs", 'fn main() {\n    println!("hi");\n}\n')
    write(d / "src/lib.rs", "pub fn a() -> u32 { 1 }")  # no trailing newline
    write(d / "old_name.txt", "old name\nto be deleted\n")
    write(d / "gone.txt", "delete me\nline\n")
    write(d / "data.bin", b"\x00\x01\x02binary\x00")
    git(d, "add", "-A")
    git(d, "commit", "-qm", "base")  # noqa: E702
    # c2: modify, add, delete, rename, binary change
    write(d / "README.md", "# demo\n\nfirst line\nsecond line\n")
    write(d / "src/lib.rs", "pub fn a() -> u32 { 2 }\n")  # gains newline
    write(d / "notes_cjk.txt", "日本語のテキスト\n中文内容 mixed with ascii\n")
    write(d / "tabs.txt", "a\tb\tc\n\tindented\twith\ttabs\n")
    git(d, "rm", "-q", "gone.txt")
    git(d, "mv", "old_name.txt", "new_name.txt")
    write(d / "data.bin", b"\x00\x01\x03changed\x00")
    git(d, "add", "-A")
    git(d, "commit", "-qm", "feat: touch every file kind")  # noqa: E702
    # c3
    write(d / "src/main.rs", 'fn main() {\n    println!("hello");\n}\n')
    git(d, "commit", "-qam", "fix: greeting")
    # working tree: one staged, one unstaged, one untracked
    write(d / "staged.txt", "staged addition\n")
    git(d, "add", "staged.txt")  # noqa: E702
    write(d / "README.md", "# demo\n\nfirst line\nsecond line\nunstaged line\n")
    write(d / "untracked.txt", "untracked\n")
    return d


class MergeBase(NamedTuple):
    repo: Path
    base: str  # the fork point (sha)
    main: str
    topic: str


def git_merge_base(d: str | Path) -> MergeBase:
    """For `-r A...B`: branch topic forks from main, both advance."""
    d = _fresh_git(d)
    write(d / "shared.txt", "base\n")
    git(d, "add", "-A")
    git(d, "commit", "-qm", "base")  # noqa: E702
    base = git(d, "rev-parse", "HEAD")
    git(d, "checkout", "-q", "-b", "topic")
    write(d / "topic.txt", "topic change\n")
    git(d, "add", "-A")
    git(d, "commit", "-qm", "topic work")  # noqa: E702
    topic = git(d, "rev-parse", "HEAD")
    git(d, "checkout", "-q", "main")
    write(d / "mainonly.txt", "main change\n")
    git(d, "add", "-A")
    git(d, "commit", "-qm", "main moves on")  # noqa: E702
    return MergeBase(d, base, git(d, "rev-parse", "HEAD"), topic)


def pr_repo(d: str | Path, origin: str = "https://github.com/octo/demo.git") -> Path:
    """Small git repo whose origin picks the fake forge (default github.com/octo/demo: fake gh, canned PR #1 touching
    README.md and src/greet.rs)."""
    d = _fresh_git(d)
    write(d / "README.md", "# demo\nold line\n")
    git(d, "add", "-A")
    git(d, "commit", "-qm", "base")  # noqa: E702
    git(d, "remote", "add", "origin", origin)
    return d


def wide_repo(d: str | Path) -> Path:
    """Two commits. cjk.txt gains long CJK / VS16-emoji lines (to overflow a side-by-side pane), plus a deeply nested
    long-named file and a long directory name (to overflow the file tree). Review with `-r HEAD~1..HEAD`."""
    d = _fresh_git(d)
    write(d / "cjk.txt", "ascii one\nascii two\nascii three\n")
    write(d / "deep/dir/an_extremely_long_file_name_for_the_tree_pane.txt", "x\n")
    write(d / "zzzz_a_rather_long_directory_name_here/b.txt", "x\n")
    git(d, "add", "-A")
    git(d, "commit", "-qm", "base")  # noqa: E702
    write(
        d / "cjk.txt",
        "ascii one\n"
        + "日本語のテキスト変更" * 8
        + " wide-tail-W\n"
        + "emoji "
        + "❤️✔️" * 20
        + " tail-E vs16\n"
        + "ascii three changed "
        + "x" * 80
        + "\n"
        + "a"
        + "日本語のテキスト変更" * 8
        + "\n",
    )
    write(d / "deep/dir/an_extremely_long_file_name_for_the_tree_pane.txt", "y\n")
    write(d / "zzzz_a_rather_long_directory_name_here/b.txt", "y\n")
    git(d, "add", "-A")
    git(d, "commit", "-qm", "change")  # noqa: E702
    return d


def long_file_repo(d: str | Path) -> Path:
    """git_repo plus a committed, unchanged 120-line long.txt ("line N"), for --file/--line."""
    d = git_repo(d)
    write(d / "long.txt", "".join(f"line {i}\n" for i in range(1, 121)))
    git(d, "add", "long.txt")
    git(d, "commit", "-qm", "add long.txt")  # noqa: E702
    return d


# ---- Mercurial and Jujutsu ---------------------------------------------------------------------------------------
def _candidate_dirs(env_dir: str, tool: str) -> list[str]:
    found = shutil.which(tool)
    return [x for x in (os.environ.get(env_dir), os.path.dirname(found) if found else None) if x]


def _use_tool(s: Scenario, tool: str, env_dir: str, env_require: str, works: Callable[[str], bool], hint: str) -> str:
    for d in _candidate_dirs(env_dir, tool):
        env = dict(os.environ, PATH=f"{d}:{os.environ['PATH']}")
        p = subprocess.run(
            [tool, "--version"],
            env=env,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            text=True,
        )
        if p.returncode == 0 and works(p.stdout):
            os.environ["PATH"] = f"{d}:{os.environ['PATH']}"  # the harness's fixture commands
            s.pre_path.insert(0, d)  # and the child's PATH
            return d
    if os.environ.get(env_require):
        raise Failure(f"real {hint} required ({env_require}=1) but not found; set {env_dir}")
    raise Skip(f"{hint} not found (set {env_dir}" + ("; Sapling's hg does not count)" if tool == "hg" else ")"))


def use_hg(s: Scenario) -> None:
    """Put real Mercurial first on PATH (E2E_HG_DIR, else the `hg` on PATH); Sapling's `hg` alias is rejected.
    Without it the scenario is SKIPPED, unless E2E_REQUIRE_HG=1 (CI), where it FAILS."""
    _use_tool(s, "hg", "E2E_HG_DIR", "E2E_REQUIRE_HG", lambda out: grep("(?i)mercurial distributed", out), "Mercurial")


def use_jj(s: Scenario) -> None:
    """Put the real `jj` first on PATH (E2E_JJ_DIR, else the one on PATH) and give it an identity (also in the child's
    environment). Without jj the scenario is SKIPPED, unless E2E_REQUIRE_JJ=1 (CI), where it FAILS."""
    _use_tool(s, "jj", "E2E_JJ_DIR", "E2E_REQUIRE_JJ", lambda out: True, "jj")
    ident = dict(JJ_USER="E2E", JJ_EMAIL="e2e@example.invalid", JJ_CONFIG="/dev/null")
    os.environ.update(ident)
    s.env.update(ident)


def hg_repo(d: str | Path) -> Path:
    """Needs use_hg first. Modified z, added a, removed m, untouched k."""
    d = fresh(d)
    write(d / "z.txt", "z_old\n")
    write(d / "m.txt", "m_old\n")
    write(d / "k.txt", "k\n")  # noqa: E702
    run(d, "hg", "init", "-q", ".")
    run(d, "hg", "add", "-q", ".")  # noqa: E702
    run(d, "hg", "commit", "-q", "-u", "e2e", "-m", "base")
    write(d / "z.txt", "z_new\n")
    write(d / "a.txt", "a_added\n")  # noqa: E702
    run(d, "hg", "add", "-q", "a.txt")
    run(d, "hg", "rm", "-qf", "m.txt")  # noqa: E702
    return d


def jj_repo(d: str | Path) -> Path:
    """Needs use_jj first. Pure jj repo: "base" (a.txt, b.txt), "second" (a.txt edited, b.txt deleted, c.txt added),
    then a working-copy change (a.txt edited again, d.txt added)."""
    d = fresh(d)
    run(d, "jj", "--quiet", "git", "init", ".")
    write(d / "a.txt", "a one\n")
    write(d / "b.txt", "b one\n")
    run(d, "jj", "--quiet", "describe", "-m", "base")  # noqa: E702
    run(d, "jj", "--quiet", "new", "-m", "second")
    write(d / "a.txt", "a one\na two\n")
    (d / "b.txt").unlink()
    write(d / "c.txt", "c one\n")  # noqa: E702
    run(d, "jj", "--quiet", "new", "-m", "wip")
    write(d / "a.txt", "a one\na two\na wip\n")
    write(d / "d.txt", "d wip\n")  # noqa: E702
    return d
