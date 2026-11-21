"""Fake-forge fixtures: PR #1 through the fake CLIs, the Gerrit REST server, the editor snapshot check."""

from __future__ import annotations

import shutil
import subprocess
import sys
from pathlib import Path
from typing import TYPE_CHECKING, Any, NamedTuple

from . import repos
from .core import E2E_DIR, TIMEOUT, UNEXPECTED_MARK, Failure, grep
from .tui import Tui
from .waits import wait_file

if TYPE_CHECKING:
    from .core import Scenario


def forge_open(
    s: Scenario, url: str, variant: str | None = None, env: dict[str, str] | None = None, **start_kw: Any
) -> Tui:
    """Fixture repo whose origin is `url` (the URL picks the fake: github.com -> gh, gitlab.com -> glab,
    gitea.example.com -> tea, bitbucket.org -> bkt), `tuicr pr 1`, wait for the PR to load.
    variant = data/<forge>/<variant>/ overlay (E2E_VARIANT); env: extra child variables."""
    repo = repos.pr_repo(s.dir / "repo", origin=url)
    t = s.start("pr", "1", cwd=repo, env={**({"E2E_VARIANT": variant} if variant else {}), **(env or {})}, **start_kw)
    return t.wait_for("PR Description|Merge Request|!1", 2 * TIMEOUT)


def pr_open(s: Scenario, variant: str = "threads_and_commits", **kw: Any) -> Tui:
    """GitHub PR #1 via the fake gh with the fixture variant data/github/<variant>/."""
    return forge_open(s, "https://github.com/octo/demo.git", variant, **kw)


def pr_comment(t: Tui, text: str) -> Tui:
    """In the opened PR, add a line comment on src/greet.rs line 1 and wait until it is saved (see Tui.comment)."""
    t.keys("}").cursor_on("1[╭ ]▌ pub fn greet")
    return t.comment(text)


def pr_editor_snapshot(s: Scenario, url: str = "https://github.com/octo/demo.git") -> Tui:
    """`e` on a PR file the checkout lacks makes tuicr write a read-only snapshot and open $EDITOR on it. A fake editor
    (data/fake_editor.py) records the snapshot's path and the permissions of every directory above it into
    <scratch>/editor.argv (waited for); then check what a private snapshot must look like: keyed by the full head sha,
    base directory 0700 and owned by us, file read-only. Returns the Tui."""
    d = s.dir
    d.mkdir(parents=True, exist_ok=True)
    argv = d / "editor.argv"
    if argv.exists():
        argv.unlink()
    shutil.copy(E2E_DIR / "data" / "fake_editor.py", d / "editor.sh")
    (d / "editor.sh").chmod(0o755)
    t = forge_open(s, url, "threads_and_commits", env={"EDITOR": str(d / "editor.sh")})
    t.keys("}").cursor_on("1[╭ ]▌ pub fn greet")
    t.keys("e")
    try:
        wait_file(argv, TIMEOUT)
    except Failure as e:
        raise Failure("fake editor was not run", t.screen()) from e
    text = argv.read_text()
    print(text, end="")
    file = next((ln[len("FILE ") :] for ln in text.splitlines() if ln.startswith("FILE ")), "")
    if not grep("pr1-2{40}/", file):
        raise Failure(f"snapshot dir is not keyed by the full head sha: {file}")
    first_dir = next((ln for ln in text.splitlines() if ln.startswith("DIR ")), None)
    if first_dir is not None:  # the base directory: `DIR <path> <mode> uid=<n> me=<n>`
        _, path, mode, uid, me = first_dir.split()
        if not (mode == "700" and uid[len("uid=") :] == me[len("me=") :]):
            raise Failure(f"snapshot base {path} is not private (mode {mode} {uid} {me})")
    if not any(ln.startswith("FILEMODE 4") for ln in text.splitlines()):
        raise Failure("snapshot file is not read-only")
    return t


def gerrit_open(
    s: Scenario, kind: str, host: str = "gerrit.example.test", project: str = "proj", target: str = "1"
) -> Tui:
    """Change 1 of `project` served by fakeforge/gerrit.py (anonymous, GERRIT_URL on 127.0.0.1); origin is a Gerrit SSH
    remote on `host`, both commits exist locally so no fetch is needed. kind:
      root:    a single parentless commit (a.txt); the change has no parents, so the base is the empty tree.
      threads: base + head; a.txt line 1 thread resolved by a reply, line 2 thread reopened by a reply.
    target: the `tuicr pr` argument, "@PORT@" replaced by the fake's port. The server is stopped by a cleanup
    registered here (it also dies with the scenario's process group); gerrit_stop() stops it earlier."""
    r, url = gerrit_serve(s, kind, host=host, project=project)
    port = url.rsplit(":", 1)[1]
    return s.start("pr", target.replace("@PORT@", port), cwd=r, env={"GERRIT_URL": url})


def gerrit_serve(
    s: Scenario, kind: str, host: str = "gerrit.example.test", project: str = "proj", tls: TlsFiles | None = None
) -> tuple[Path, str]:
    """gerrit_open's fixture without starting tuicr: returns (clone, GERRIT_URL). tls: serve HTTPS with that
    certificate (see tls_files) instead of plain HTTP."""
    d = s.dir
    r, gd = d / "repo", d / "gerrit"
    log = gd / "forge.log"  # the fake's calls, UNEXPECTED notes included: checked at exit like the CLI fakes'
    if log.exists():  # a second serve wipes the dir: keep what the first one flagged
        s.forge_unexpected += [ln for ln in log.read_text().splitlines() if UNEXPECTED_MARK in ln]
    shutil.rmtree(gd, ignore_errors=True)
    r = repos._fresh_git(r)
    gd.mkdir(parents=True)
    base = ""
    if kind == "threads":
        repos.write(r / "a.txt", "base one\nbase two\n")
        repos.git(r, "add", "-A")
        repos.git(r, "commit", "-qm", "base")  # noqa: E702
        base = repos.git(r, "rev-parse", "HEAD")
    repos.write(r / "a.txt", "line one\nline two\nline three\n")
    repos.git(r, "add", "-A")
    repos.git(r, "commit", "-qm", "Add a.txt")  # noqa: E702
    head = repos.git(r, "rev-parse", "HEAD")
    repos.git(r, "remote", "add", "origin", f"ssh://{host}:29418/{project}")
    parents = f'[{{"commit": "{base}"}}]' if base else "[]"
    tpl = (E2E_DIR / "data/gerrit/change.json.tpl").read_text()
    (gd / "change.json").write_text(
        tpl.replace("@PROJECT@", project).replace("@HEAD@", head).replace("@PARENTS@", parents)
    )
    shutil.copy(E2E_DIR / "data/gerrit/comments.json", gd / "comments.json")
    argv = [sys.executable, str(E2E_DIR / "fakeforge/gerrit.py"), str(gd), str(gd / "port")]
    if tls is not None:
        argv += [str(tls.cert), str(tls.key)]
    s.forge_logs.append(log)
    s.gerrit_proc = subprocess.Popen(argv)
    s.on_exit(gerrit_stop, s)
    wait_file(gd / "port", TIMEOUT)
    port = (gd / "port").read_text().strip()
    return r, f"{'https' if tls else 'http'}://127.0.0.1:{port}"


class TlsFiles(NamedTuple):
    ca: Path  # PEM root certificate, trusted by nothing until a scenario says so
    cert: Path  # PEM leaf for IP 127.0.0.1, signed by `ca`
    key: Path  # the leaf's private key


def tls_files(d: Path) -> TlsFiles:
    """A throwaway CA and a 127.0.0.1 server certificate it signs, made with the `openssl` CLI under `d`."""
    d.mkdir(parents=True, exist_ok=True)
    ca_conf = repos.write(
        d / "ca.cnf",
        "[req]\ndistinguished_name = dn\nprompt = no\nx509_extensions = v3_ca\n[dn]\nCN = tuicr e2e CA\n"
        "[v3_ca]\nbasicConstraints = critical,CA:TRUE\nkeyUsage = critical,keyCertSign,cRLSign\n"
        "subjectKeyIdentifier = hash\n",
    )
    leaf_ext = repos.write(
        d / "leaf.ext",
        "basicConstraints = CA:FALSE\nkeyUsage = critical,digitalSignature,keyEncipherment\n"
        "extendedKeyUsage = serverAuth\nsubjectAltName = IP:127.0.0.1\n",
    )
    ca, ca_key, cert, key, csr = (d / n for n in ("ca.pem", "ca.key", "leaf.pem", "leaf.key", "leaf.csr"))
    for argv in (
        ["req", "-x509", "-newkey", "rsa:2048", "-nodes", "-days", "2", "-config", str(ca_conf)]
        + ["-keyout", str(ca_key), "-out", str(ca)],
        ["req", "-newkey", "rsa:2048", "-nodes", "-subj", "/CN=127.0.0.1", "-keyout", str(key), "-out", str(csr)],
        ["x509", "-req", "-in", str(csr), "-CA", str(ca), "-CAkey", str(ca_key), "-set_serial", "2", "-days", "2"]
        + ["-extfile", str(leaf_ext), "-out", str(cert)],
    ):
        p = subprocess.run(["openssl", *argv], capture_output=True, text=True)
        if p.returncode != 0:
            raise Failure(f"openssl {argv[0]} failed", p.stderr)
    return TlsFiles(ca, cert, key)


def gerrit_stop(s: Scenario) -> None:
    """Stop the fake Gerrit server started by gerrit_open (idempotent)."""
    p = s.gerrit_proc
    if p is not None and p.poll() is None:
        p.kill()
        p.wait()
    s.gerrit_proc = None
