#!/usr/bin/env python3
"""Serves the Time page to a browser the way the desktop program serves it to
its own web view, for work on the page's layout: the three files from the
working tree, the program's greeting before them, the program's content policy,
and /api/time forwarded to the stack on the session of one made-up account per
port. The account never reaches the page, as in the program.

It is a stand-in for the program's window in an ordinary browser. What it
shows says how the page lays out, not how the program's window behaves.

    run.sh page     starts it on the stack's network: http://pagehost:8101 a manager,
                    http://pagehost:8102 a viewer; ?theme=dark for the dark theme
"""

import os
import sys
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import parse_qs, urlsplit

import httpx

ASSETS = Path(os.environ.get("PAGE_ASSETS", "/time-assets"))
UPSTREAM = os.environ.get("PAGE_UPSTREAM", "http://caddy")
PASSWORD = os.environ["E2E_PASSWORD"]
# port -> (account, Time role); the accounts are made here if the stack has none.
ACCOUNTS = {8101: ("ui_manager", "manager"), 8102: ("ui_viewer", "viewer")}
# The policy the program gives the page: desktop/crates/tbs-app/src/screens/hosted/assets.rs, Kind::policy.
POLICY = (
    "default-src 'none'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; "
    "font-src 'self'; connect-src 'self'; form-action 'none'; base-uri 'none'; frame-src 'none'; object-src 'none'"
)
TYPES = {".html": "text/html; charset=utf-8", ".css": "text/css; charset=utf-8", ".js": "text/javascript; charset=utf-8"}

upstream = httpx.Client(base_url=UPSTREAM, timeout=60.0, trust_env=False)
tokens = {}


def sign_in(username):
    answer = upstream.post("/api/auth/login", json={"username": username, "password": PASSWORD})
    upstream.cookies.clear()
    body = answer.json() if answer.status_code == 200 else {}
    return body.get("token")


def prepare():
    admin = sign_in("e2e_admin")
    if not admin:
        sys.exit("e2e_admin did not sign in: is the stack up and migrated (run.sh up, rehearse, start)?")
    auth = {"Authorization": f"Bearer {admin}"}
    known = {row["username"] for row in upstream.get("/api/admin/users", headers=auth).json()}
    for port, (username, role) in ACCOUNTS.items():
        if username not in known:
            made = upstream.post("/api/admin/users", headers=auth, json={"username": username, "password": PASSWORD, "display_name": username, "workspaces": {"time": role}})
            if made.status_code != 200:
                sys.exit(f"{username} was not created: {made.status_code} {made.text[:200]}")
        tokens[port] = sign_in(username)
        if not tokens[port]:
            sys.exit(f"{username} did not sign in")
    upstream.post("/api/auth/logout", headers=auth)


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, *_):
        pass

    def reply(self, status, body, kind, **headers):
        self.send_response(status)
        self.send_header("Content-Type", kind)
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Cache-Control", "no-store")
        for name, value in headers.items():
            self.send_header(name.replace("_", "-"), value)
        self.end_headers()
        self.wfile.write(body)

    def page_file(self, name):
        path = ASSETS / name
        if not path.is_file():
            return self.reply(404, b"not found", "text/plain")
        body = path.read_bytes()
        if name == "index.html":
            theme = "dark" if parse_qs(urlsplit(self.path).query).get("theme") == ["dark"] else "light"
            hook = f'<script src="/__shell.js?theme={theme}"></script>\n  <script src="/assets/app.js">'.encode()
            body = body.replace(b'<script src="/assets/app.js">', hook)
        self.reply(200, body, TYPES[path.suffix], Content_Security_Policy=POLICY, X_Content_Type_Options="nosniff")

    def forward(self):
        size = int(self.headers.get("Content-Length") or 0)
        body = self.rfile.read(size) if size else None
        # As in the program: the page's own headers stop here, but for the body's type.
        headers = {"Authorization": f"Bearer {tokens[self.server.server_port]}"}
        if self.headers.get("Content-Type"):
            headers["Content-Type"] = self.headers["Content-Type"]
        answer = upstream.request(self.command, self.path, content=body, headers=headers)
        upstream.cookies.clear()
        more = {"Content_Disposition": answer.headers["content-disposition"]} if "content-disposition" in answer.headers else {}
        self.reply(answer.status_code, answer.content, answer.headers.get("content-type", "application/octet-stream"), **more)

    def do_GET(self):
        path = urlsplit(self.path).path
        if path.startswith("/api/time/"):
            return self.forward()
        if path == "/__shell.js":
            theme = "dark" if "theme=dark" in self.path else "light"
            script = f"window.TIMELIST_HUB = Object.freeze({{ apiBase: '/api/time', native: true, theme: '{theme}' }});"
            return self.reply(200, script.encode(), TYPES[".js"])
        if path in ("/", "/index.html"):
            return self.page_file("index.html")
        if path in ("/assets/app.css", "/assets/app.js"):
            return self.page_file(path.rsplit("/", 1)[1])
        self.reply(404, b"not found", "text/plain")

    def do_POST(self):
        if urlsplit(self.path).path.startswith("/api/time/"):
            return self.forward()
        self.reply(403, b"closed", "text/plain")

    do_PUT = do_PATCH = do_DELETE = do_POST


if __name__ == "__main__":
    prepare()
    servers = [ThreadingHTTPServer(("0.0.0.0", port), Handler) for port in ACCOUNTS]
    for server in servers[1:]:
        threading.Thread(target=server.serve_forever, daemon=True).start()
    print("page host: " + ", ".join(f"{port} {name} ({role})" for port, (name, role) in ACCOUNTS.items()), flush=True)
    servers[0].serve_forever()
