#!/usr/bin/env python3
"""Fake Gerrit REST server (anonymous, 127.0.0.1). usage: fakeforge/gerrit.py <dir> <portfile> [<cert.pem> <key.pem>]
Plain HTTP, or HTTPS when a certificate chain and key are given.
Serves $dir/change.json for /changes/N/ and $dir/comments.json for /changes/N/comments, '[]'-style magic prefix
included; logs every request path to $dir/gerrit.log. Every request is also logged to $dir/forge.log in the fake-CLI
format ({"tool": "gerrit", "argv": [METHOD, path], "stdin": body}); an unknown route, an unsupported method or a
handler crash adds an UNEXPECTED note there (404/405/500 answered), which fails the scenario like the CLI fakes."""

from __future__ import annotations

import http.server
import json
import os
import socketserver
import ssl
import sys
import threading
import traceback
from typing import Any

d, portfile = sys.argv[1], sys.argv[2]
_lock = threading.Lock()


def record(method: str, path: str, body: str | None, note: str | None = None) -> None:
    entry: dict[str, Any] = {"tool": "gerrit", "argv": [method, path], "stdin": body}
    if note:
        entry["note"] = note
    with _lock:
        fd = os.open(os.path.join(d, "forge.log"), os.O_WRONLY | os.O_APPEND | os.O_CREAT, 0o644)
        try:
            os.write(fd, (json.dumps(entry) + "\n").encode())
        finally:
            os.close(fd)


class H(http.server.BaseHTTPRequestHandler):
    def log_message(self, format: str, *args: Any) -> None:
        pass

    def _reply(self, status: int, body: bytes, ctype: str = "text/plain") -> None:
        self.send_response(status)
        self.send_header("Content-Type", ctype)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def _body(self) -> str | None:
        n = int(self.headers.get("Content-Length") or 0)
        return self.rfile.read(n).decode(errors="replace") if n else None

    def do_GET(self) -> None:
        p = self.path
        with _lock:
            open(os.path.join(d, "gerrit.log"), "a").write(p + "\n")
        record("GET", p, None)
        try:
            route = p.split("?")[0].rstrip("/")
            f = None
            if route.startswith("/changes/") and route.count("/") == 2:
                f = "change.json"
            elif route.endswith("/comments"):
                f = "comments.json"
            if f is None:
                record("GET", p, None, "UNEXPECTED: no route for GET " + p)
                self._reply(404, b"no")
                return
            body = (")]}'\n" + open(os.path.join(d, f)).read()).encode()
        except Exception:  # noqa: BLE001 - a crash of the fake must fail the scenario, not look like a 500 from Gerrit
            record("GET", p, None, "UNEXPECTED: fake crashed: " + traceback.format_exc().replace("\n", " | "))
            self._reply(500, b"fake crashed")
            return
        self._reply(200, body, "application/json")

    def _unsupported(self) -> None:
        body = self._body()
        record(self.command, self.path, body)
        record(self.command, self.path, body, f"UNEXPECTED: unsupported method {self.command} {self.path}")
        self._reply(405, b"unsupported")

    do_POST = do_PUT = do_PATCH = do_DELETE = do_HEAD = _unsupported


class S(http.server.ThreadingHTTPServer):
    # HTTPServer.server_bind does a reverse DNS lookup (getfqdn), which can take many seconds on CI runners
    def server_bind(self) -> None:
        socketserver.TCPServer.server_bind(self)
        self.server_name, self.server_port = "127.0.0.1", self.server_address[1]


s = S(("127.0.0.1", 0), H)
if len(sys.argv) == 5:
    ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    ctx.load_cert_chain(sys.argv[3], sys.argv[4])
    # A client that rejects the certificate fails the handshake inside accept(); socketserver drops that OSError.
    s.socket = ctx.wrap_socket(s.socket, server_side=True)
open(portfile, "w").write(str(s.server_address[1]))
s.serve_forever()
