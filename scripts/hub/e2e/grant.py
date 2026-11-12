#!/usr/bin/env python3
"""Changes one account of the disposable stack from outside the client, the way
an administrator would in the panel. For the window run, where access is taken
away while a page is open. Goes through Caddy as e2e_admin; run it through the
`tool` service:

    run.sh compose run --rm -T tool /e2e/grant.py USER time=manager checks=- admin=yes active=no

`-` takes a workspace away; whatever is not named stays as it is.
"""

import os
import sys

import httpx


def main():
    if len(sys.argv) < 3:
        print(__doc__)
        sys.exit(2)
    username, change = sys.argv[1], {}
    for item in sys.argv[2:]:
        name, _, value = item.partition("=")
        if name in ("checks", "time"):
            change.setdefault("workspaces", {})[name] = None if value == "-" else value
        elif name == "admin":
            change["global_admin"] = value == "yes"
        elif name == "active":
            change["is_active"] = value == "yes"
        else:
            sys.exit(f"unknown change: {item}")
    with httpx.Client(base_url="http://caddy", timeout=30.0, trust_env=False) as edge:
        signed = edge.post("/api/auth/login", json={"username": "e2e_admin", "password": os.environ["E2E_PASSWORD"]}).json()
        if not signed.get("token"):
            sys.exit(f"e2e_admin did not sign in: {signed.get('error')}")
        auth = {"Authorization": f"Bearer {signed['token']}"}
        rows = {row["username"]: row for row in edge.get("/api/admin/users", headers=auth).json()}
        if username not in rows:
            sys.exit(f"no account {username}; there are {sorted(rows)}")
        answer = edge.patch(f"/api/admin/users/{rows[username]['id']}", headers=auth, json=change)
        row = answer.json()
        shown = {key: row.get(key) for key in ("username", "is_active", "global_admin", "workspace_roles")} if answer.status_code == 200 else row
        print(answer.status_code, shown)
        edge.post("/api/auth/logout", headers=auth)
        sys.exit(0 if answer.status_code == 200 else 1)


if __name__ == "__main__":
    main()
