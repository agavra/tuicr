#!/usr/bin/env python3
"""Fake Gerrit REST server (anonymous, http://127.0.0.1). usage: fakeforge/gerrit.py <dir> <portfile>
Serves $dir/change.json for /changes/N/ and $dir/comments.json for /changes/N/comments, '[]'-style magic prefix
included; logs every request path to $dir/gerrit.log; anything else is 404."""

from __future__ import annotations

import http.server
import os
import socketserver
import sys
from typing import Any

d, portfile = sys.argv[1], sys.argv[2]


class H(http.server.BaseHTTPRequestHandler):
    def log_message(self, format: str, *args: Any) -> None:
        pass

    def do_GET(self) -> None:
        p = self.path
        open(os.path.join(d, "gerrit.log"), "a").write(p + "\n")
        route = p.split("?")[0].rstrip("/")
        f = None
        if route.startswith("/changes/") and route.count("/") == 2:
            f = "change.json"
        elif route.endswith("/comments"):
            f = "comments.json"
        if f is None:
            self.send_response(404)
            self.end_headers()
            self.wfile.write(b"no")
            return
        body = (")]}'\n" + open(os.path.join(d, f)).read()).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.end_headers()
        self.wfile.write(body)


class S(http.server.ThreadingHTTPServer):
    # HTTPServer.server_bind does a reverse DNS lookup (getfqdn), which can take many seconds on CI runners
    def server_bind(self) -> None:
        socketserver.TCPServer.server_bind(self)
        self.server_name, self.server_port = "127.0.0.1", self.server_address[1]


s = S(("127.0.0.1", 0), H)
open(portfile, "w").write(str(s.server_address[1]))
s.serve_forever()
