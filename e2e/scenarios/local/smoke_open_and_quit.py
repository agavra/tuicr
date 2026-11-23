#!/usr/bin/env python3
# Open the git fixture, see the file tree + a diff, quit with :q. The bare binary (no --no-update-check) starts: its
# update check goes through a local proxy that records the request and drops it, so no real network is used.
import socket
import threading

from e2e import TUICR_BIN, Scenario, repos, scenario, wait_cmd


@scenario
def main(s: Scenario) -> None:
    repo = repos.git_repo(s.dir / "repo")
    proxy = socket.create_server(("127.0.0.1", 0))
    s.on_exit(proxy.close)
    requests: list[str] = []

    def serve() -> None:  # read each request's head, record its first line, close the connection unanswered
        while True:
            try:
                conn, _ = proxy.accept()
            except OSError:
                return
            with conn:
                # ureq sends the CONNECT head in many small writes (TCP_NODELAY), so one recv can return a fragment
                # such as "CONNECT ": read up to the blank line that ends the head
                conn.settimeout(5)
                head = b""
                try:
                    while b"\r\n\r\n" not in head and (chunk := conn.recv(4096)):
                        head += chunk
                except OSError:
                    pass
                requests.append(head.decode(errors="replace").split("\r\n")[0])

    threading.Thread(target=serve, daemon=True).start()
    url = f"http://127.0.0.1:{proxy.getsockname()[1]}"
    env = {"HTTPS_PROXY": url, "https_proxy": url, "ALL_PROXY": url}
    tui = s.start_cmd([TUICR_BIN], cwd=repo, env=env)  # no --no-update-check: the bare binary starts
    tui.wait_for("Staged changes|Unstaged changes")  # commit selector
    wait_cmd(10, lambda: "CONNECT crates.io:443 HTTP/1.1" in requests)  # the update check ran, through the proxy
    tui.keys("Enter")
    tui.expect(r"README\.md")  # file tree
    tui.expect("unstaged line")  # diff content
    tui.keys(":q", "Enter")  # `q` alone is only a hint
    tui.wait_exit()
