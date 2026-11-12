#!/usr/bin/env python3
"""End-to-end checks of the Hub on the disposable stack of compose.yml: the
real backend on PostgreSQL and Redis, the real Time server behind its gateway,
Caddy with the server's routes, and fake_telegram.py where Telegram would be.
Run through the `tool` service, on the stack's internal network:

    checks.py            the stack as it would be deployed (run.sh start)
    checks.py legacy     release 2.0.3's code on whatever schema is there: reads only
    checks.py legacy-writes   the same, and two accounts created through the 2.0.3 API

Requests go two ways on purpose: through Caddy, as a client on the internet
reaches the server, and straight to the backend, as only a neighbour container
can. Every account, Telegram ID and group here is made up.
"""

import base64
import json
import os
import sys
import time
from datetime import datetime, timedelta, timezone

import httpx

EDGE = "http://caddy"
BACKEND = "http://backend:8000"
TIME = "http://timelist:8080"
TELEGRAM = "http://faketg:8081"

PASSWORD = os.environ["E2E_PASSWORD"]
SYSTEM_TOKEN = os.environ["SYSTEM_ACCESS_TOKEN"]
HUB_KEY = os.environ["TIME_HUB_KEY"]
TIME_TOKEN = os.environ["TIMELIST_API_TOKEN"]
WORK_GROUP = int(os.environ["TIME_BOT_CHAT_ID"])
OTHER_GROUP = WORK_GROUP - 998
TG_EMPLOYEE = 700000001
TG_STRANGER = 700000099

WF = "workspace_forbidden"
ROLE = "time_role_forbidden"
NOT_ADMIN = "global_admin_required"
XLSX = "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"


def client(base, **headers):
    made = httpx.Client(base_url=base, headers=headers, timeout=30.0, trust_env=False, follow_redirects=False)
    # One client object stands for many people here. Without this it would
    # carry the refresh cookie of whoever signed in last into everyone's calls.
    made.event_hooks["response"].append(lambda _response: made.cookies.clear())
    return made


edge = client(EDGE)
backend = client(BACKEND)
timelist = client(TIME)
telegram = client(TELEGRAM)
limited = []


class Report:
    def __init__(self):
        self.count = 0
        self.failed = []

    def section(self, title):
        print(f"\n== {title}", flush=True)

    def check(self, name, ok, detail=""):
        self.count += 1
        if ok:
            print(f"  ok    {name}", flush=True)
        else:
            self.failed.append(name)
            print(f"  FAIL  {name}" + (f" -- {detail}" if detail else ""), flush=True)
        return bool(ok)

    def answer(self, name, response, status, code=None):
        ok = response.status_code == status and (code is None or error_code(response) == code)
        return self.check(name, ok, f"{response.status_code} {response.text[:200]!r}, expected {status} {code or ''}")

    def finish(self):
        print(f"\n{self.count} checks, {len(self.failed)} failed", flush=True)
        for name in self.failed:
            print(f"  failed: {name}")
        sys.exit(1 if self.failed else 0)


report = Report()


def error_code(response):
    """The machine-readable part of an error answer, whatever shape it took."""
    try:
        body = response.json()
    except ValueError:
        return None
    detail = body.get("detail", body) if isinstance(body, dict) else body
    if isinstance(detail, dict):
        return detail.get("error") or detail
    return detail


def bearer(token):
    return {"Authorization": f"Bearer {token}"}


def sign_in(username, password=None, via=None):
    """One attempt at the sign-in form. The server allows ten a minute from one
    address; when it says so, this waits as a client has to."""
    while True:
        response = (via or edge).post("/api/auth/login", json={"username": username, "password": password or PASSWORD})
        if response.status_code != 429:
            return response
        found = [int(part) for part in response.text.replace('"', " ").replace(":", " ").replace("}", " ").split() if part.isdigit()]
        wait = (found[-1] if found else 60) + 1
        limited.append(wait)
        print(f"  ...   the sign-in limit answered 429; waiting {wait} s", flush=True)
        time.sleep(wait)


def token_of(username, password=None):
    response = sign_in(username, password)
    body = response.json() if response.status_code == 200 else {}
    if not body.get("ok") or not body.get("token"):
        report.check(f"{username} signs in", False, f"{response.status_code} {response.text[:200]!r}")
        return None
    return body["token"]


def me(token):
    response = edge.get("/api/auth/me", headers=bearer(token))
    return response.json() if response.status_code == 200 else {"_status": response.status_code, "_code": error_code(response)}


def hub_fields(token):
    body = me(token)
    return body.get("workspaces"), body.get("workspace_roles"), body.get("global_admin")


def patch_user(admin, user_id, **change):
    return edge.patch(f"/api/admin/users/{user_id}", headers=bearer(admin), json=change)


def with_expiry(token, moment):
    """The same token, signed with the stack's own key, with another expiry."""
    from jose import jwt

    part = token.split(".")[1]
    claims = json.loads(base64.urlsafe_b64decode(part + "=" * (-len(part) % 4)))
    claims["exp"] = int(moment.timestamp())
    claims["iat"] = min(int(claims.get("iat") or claims["exp"]), claims["exp"] - 60)
    return jwt.encode(claims, os.environ["JWT_SECRET"], algorithm=os.environ.get("JWT_ALGORITHM", "HS256"))


# ---------------------------------------------------------------- release 2.0.3


def legacy(writes):
    system = client(BACKEND, **{"X-System-Access": SYSTEM_TOKEN})
    report.section("release 2.0.3 code, straight to the backend with the system token")
    health = backend.get("/health")
    report.check("/health answers 200", health.status_code == 200, health.text[:200])
    signed = sign_in("e2e_admin", via=system)
    body = signed.json() if signed.status_code == 200 else {}
    if not report.check("the administrator signs in", bool(body.get("ok") and body.get("token")), signed.text[:200]):
        return
    auth = bearer(body["token"])
    account = system.get("/api/auth/me", headers=auth)
    fields = account.json() if account.status_code == 200 else {}
    report.check("/api/auth/me: role admin", fields.get("role") == "admin", account.text[:200])
    absent = [name for name in ("workspaces", "workspace_roles", "global_admin") if name not in fields]
    report.check("/api/auth/me carries none of workspaces, workspace_roles, global_admin", len(absent) == 3, str(sorted(fields)))
    report.answer("Checks: the list of receipts", system.get("/api/transactions/?page_size=1", headers=auth), 200)
    users = system.get("/api/admin/users", headers=auth)
    names = sorted(item["username"] for item in users.json()) if users.status_code == 200 else []
    report.check("account administration lists the accounts", {"e2e_admin", "e2e_admin2", "e2e_operator", "e2e_old_admin"} <= set(names), users.text[:200])
    operator = sign_in("e2e_operator", via=system)
    operator_token = (operator.json() or {}).get("token") if operator.status_code == 200 else None
    if report.check("the operator signs in", bool(operator_token), operator.text[:200]):
        report.answer("the operator reads receipts", system.get("/api/transactions/?page_size=1", headers=bearer(operator_token)), 200)
        refused = system.get("/api/admin/users", headers=bearer(operator_token))
        report.check("the operator is refused account administration", refused.status_code == 403, f"{refused.status_code} {refused.text[:120]}")
    if writes:
        for username, role in (("e2e_old_code_operator", "operator"), ("e2e_old_code_admin", "admin")):
            if username in names:
                report.check(f"{username} is already there", True)
                continue
            made = system.post("/api/admin/users", headers=auth, json={"username": username, "password": PASSWORD, "role": role})
            report.check(f"2.0.3 creates {username} ({role}) on this schema", made.status_code == 200, f"{made.status_code} {made.text[:200]}")


# ---------------------------------------------------------------- the edge


def edge_and_gates():
    report.section("Caddy and the gates in front of the backend")
    health = edge.get("/health")
    report.check("/health through Caddy: 200, database connected", health.status_code == 200 and "connected" in health.text, health.text[:200])
    report.answer("an API call straight to the backend without the system token is refused", backend.get("/api/auth/me"), 403)
    report.answer(
        "with the system token the same call reaches authentication (401)",
        backend.get("/api/auth/me", headers={"X-System-Access": SYSTEM_TOKEN}), 401,
    )
    report.answer("through Caddy, which adds the system token, it reaches authentication (401)", edge.get("/api/auth/me"), 401)

    redirect = edge.get("/admin")
    report.check("/admin: 308 to /admin/", redirect.status_code == 308 and redirect.headers.get("location") == "/admin/", f"{redirect.status_code} {dict(redirect.headers)}")
    page = edge.get("/admin/")
    policy = page.headers.get("content-security-policy", "")
    report.check("/admin/: the panel's page", page.status_code == 200 and "text/html" in page.headers.get("content-type", ""), f"{page.status_code}")
    report.check(
        "/admin/: policy forbids foreign scripts, framing and form posts",
        all(part in policy for part in ("default-src 'none'", "script-src 'self'", "frame-ancestors 'none'", "form-action 'none'")), policy,
    )
    for name, kind in (("admin.js", "javascript"), ("admin.css", "text/css")):
        asset = edge.get(f"/admin/{name}")
        report.check(f"/admin/{name}: 200", asset.status_code == 200 and kind in asset.headers.get("content-type", ""), f"{asset.status_code}")
    report.answer("/admin/ serves nothing but its three files", edge.get("/admin/root-access.server.json"), 404)

    report.section("the bot's question, /internal/time/telegram/{id}")
    for label, headers in (("without a key", {}), ("with the right key", {"X-Time-Hub-Key": HUB_KEY})):
        outside = edge.get(f"/internal/time/telegram/{TG_EMPLOYEE}", headers=headers)
        from_backend = "allowed" in outside.text or "time_hub_key" in outside.text
        report.check(f"through Caddy {label}: not routed to the backend", not from_backend and outside.status_code != 200, f"{outside.status_code} {outside.text[:120]!r}")
    report.answer("straight to the backend without the key: refused", backend.get(f"/internal/time/telegram/{TG_EMPLOYEE}"), 403, "time_hub_key_refused")
    report.answer(
        "with a wrong key: refused",
        backend.get(f"/internal/time/telegram/{TG_EMPLOYEE}", headers={"X-Time-Hub-Key": HUB_KEY[::-1]}), 403, "time_hub_key_refused",
    )
    report.answer(
        "with the right key and a proxy's header: refused",
        backend.get(f"/internal/time/telegram/{TG_EMPLOYEE}", headers={"X-Time-Hub-Key": HUB_KEY, "X-Forwarded-For": "203.0.113.7"}),
        403, "time_hub_key_refused",
    )
    answer = backend.get(f"/internal/time/telegram/{TG_STRANGER}", headers={"X-Time-Hub-Key": HUB_KEY})
    report.check(
        "with the right key, from inside: answered (an unknown ID is not linked)",
        answer.status_code == 200 and answer.json() == {"allowed": False, "reason": "not_linked"}, f"{answer.status_code} {answer.text[:200]}",
    )

    report.section("Time itself, from inside the network")
    meta = timelist.get("/api/meta")
    body = meta.json() if meta.status_code == 200 else {}
    report.check("its /api/meta (the container's health check) answers without a token", meta.status_code == 200 and body.get("authed") is False, meta.text[:200])
    report.check("it runs with the shared accounts", body.get("hub") is True, meta.text[:200])
    report.answer("its data without the gateway's token: 401", timelist.get("/api/month/2026-11"), 401)
    report.answer("its own password sign-in is switched off", timelist.post("/api/login", json={"password": "x"}), 404)
    report.answer("with the gateway's token it answers", timelist.get("/api/month/2026-11", headers=bearer(TIME_TOKEN)), 200)
    report.answer("Time is not behind Caddy under its own paths", edge.get("/api/meta"), 404)


# ---------------------------------------------------------------- accounts


def migrated_accounts():
    report.section("accounts from before the migration")
    admin = token_of("e2e_admin")
    if not admin:
        return None
    report.check("an administrator of 2.0.3 signs in and is a CRM administrator with Checks", hub_fields(admin) == (["checks"], {"checks": "admin"}, True), str(hub_fields(admin)))
    meta = edge.get("/api/admin/meta", headers=bearer(admin))
    spaces = {item["id"]: item for item in meta.json().get("workspaces", [])} if meta.status_code == 200 else {}
    report.check("the panel's reference data lists Checks and Time", set(spaces) == {"checks", "time"}, meta.text[:200])
    report.check("Time is available to grant", bool(spaces.get("time", {}).get("available")), str(spaces.get("time")))
    listed = edge.get("/api/admin/users", headers=bearer(admin))
    rows = {item["username"]: item for item in listed.json()} if listed.status_code == 200 else {}
    old = rows.get("e2e_old_admin", {})
    report.check(
        "the disabled administrator kept the right on paper and cannot use it",
        old.get("global_admin") is True and old.get("effective_access", {}).get("global_admin") is False and old.get("effective_access", {}).get("can_sign_in") is False,
        str(old.get("effective_access")),
    )
    operator_row = rows.get("e2e_operator", {})
    report.check(
        "the operator: Checks only, no Time, not an administrator",
        operator_row.get("global_admin") is False and operator_row.get("workspace_roles") == {"checks": "operator"}, str(operator_row.get("workspace_roles")),
    )
    report.check("nobody got Time from the migration", all("time" not in (item.get("workspace_roles") or {}) for item in rows.values()), str({k: v.get("workspace_roles") for k, v in rows.items()}))
    operator = token_of("e2e_operator")
    if operator:
        report.check("the operator's own view: Checks, not an administrator", hub_fields(operator) == (["checks"], {"checks": "operator"}, False), str(hub_fields(operator)))
        report.answer("the operator is refused CRM administration", edge.get("/api/admin/meta", headers=bearer(operator)), 403, NOT_ADMIN)
        report.answer("the operator is refused Time", edge.get("/api/time/meta", headers=bearer(operator)), 403, WF)
    refused = sign_in("e2e_old_admin")
    report.check("the disabled administrator cannot sign in", refused.status_code == 403 and refused.json().get("error") == "inactive_user", f"{refused.status_code} {refused.text[:120]}")
    return admin, rows


ACCOUNTS = (
    # name, what the administrator asks for, what /api/auth/me must say
    ("e2e_checks", {"workspaces": {"checks": "operator"}}, (["checks"], {"checks": "operator"}, False)),
    ("e2e_viewer", {"workspaces": {"time": "viewer"}}, (["time"], {"time": "viewer"}, False)),
    ("e2e_manager", {"workspaces": {"time": "manager"}}, (["time"], {"time": "manager"}, False)),
    ("e2e_both", {"workspaces": {"checks": "operator", "time": "manager"}}, (["checks", "time"], {"checks": "operator", "time": "manager"}, False)),
    ("e2e_none", {"workspaces": {}}, ([], {}, False)),
    ("e2e_crm", {"workspaces": {}, "global_admin": True}, ([], {}, True)),
    ("e2e_employee", {"workspaces": {"time": "employee"}, "telegram_id": TG_EMPLOYEE}, ([], {"time": "employee"}, False)),
)

# What each account gets from: the Checks list, the Checks export, the Excel
# grants, Time's page data, a Time edit, the Time workbook, CRM administration.
PROBES = ("checks", "checks export", "excel grants", "time read", "time write", "time export", "crm administration")
NO_CHECKS = ((403, WF),) * 3
NO_TIME = ((403, WF),) * 3
EXPECTED = {
    "e2e_checks": ((200, None),) * 3 + NO_TIME + ((403, NOT_ADMIN),),
    "e2e_viewer": NO_CHECKS + ((200, None), (403, ROLE), (200, None), (403, NOT_ADMIN)),
    "e2e_manager": NO_CHECKS + ((200, None),) * 3 + ((403, NOT_ADMIN),),
    "e2e_both": ((200, None),) * 6 + ((403, NOT_ADMIN),),
    "e2e_none": NO_CHECKS + NO_TIME + ((403, NOT_ADMIN),),
    "e2e_crm": NO_CHECKS + NO_TIME + ((200, None),),
    "e2e_employee": NO_CHECKS + NO_TIME + ((403, NOT_ADMIN),),
}


def probe(token, employee):
    auth = bearer(token)
    return (
        edge.get("/api/transactions/?page_size=1", headers=auth),
        edge.get("/api/transactions/export", headers=auth),
        edge.get("/api/excel/grants", headers=auth),
        edge.get("/api/time/meta", headers=auth),
        edge.put(f"/api/time/marks/{employee}/2026-11-10", headers=auth, json={"note": "e2e"}),
        edge.get("/api/time/export/2026-11", headers=auth),
        edge.get("/api/admin/meta", headers=auth),
    )


def matrix(admin):
    report.section("accounts created through /api/admin/users")
    present = {item["username"] for item in edge.get("/api/admin/users", headers=bearer(admin)).json()}
    if present & {name for name, _, _ in ACCOUNTS}:
        sys.exit("these checks create their accounts and need the stack fresh: run.sh down, then run.sh")
    ids, tokens = {}, {}
    for username, fields, expected in ACCOUNTS:
        made = edge.post("/api/admin/users", headers=bearer(admin), json={"username": username, "password": PASSWORD, **fields})
        if not report.check(f"{username} is created", made.status_code == 200, f"{made.status_code} {made.text[:200]}"):
            continue
        ids[username] = made.json()["id"]
        tokens[username] = token_of(username)
        if tokens[username]:
            report.check(f"{username}: workspaces {expected[0]}, roles {expected[1]}, administrator {expected[2]}", hub_fields(tokens[username]) == expected, str(hub_fields(tokens[username])))
    disabled = edge.post("/api/admin/users", headers=bearer(admin), json={"username": "e2e_disabled", "password": PASSWORD, "is_active": False, "workspaces": {"checks": "operator", "time": "manager"}})
    report.check("e2e_disabled is created disabled", disabled.status_code == 200 and disabled.json().get("is_active") is False, disabled.text[:200])
    refused = sign_in("e2e_disabled")
    report.check("e2e_disabled cannot sign in", refused.status_code == 403 and refused.json().get("error") == "inactive_user", f"{refused.status_code} {refused.text[:120]}")
    for change in ({"workspaces": {"time": "owner"}}, {"workspaces": {"payroll": "manager"}}):
        bad = edge.post("/api/admin/users", headers=bearer(admin), json={"username": "e2e_bad", "password": PASSWORD, **change})
        report.check(f"a grant that does not exist is refused: {change['workspaces']}", bad.status_code in (400, 422), f"{bad.status_code} {bad.text[:160]}")

    report.section("Time through the gateway")
    manager = tokens.get("e2e_manager")
    employee = 0
    if manager:
        made = edge.post("/api/time/employees", headers=bearer(manager), json={"full_name": "Synthetic Employee", "nick": "synth"})
        if report.check("the manager adds an employee", made.status_code == 200 and made.json().get("id"), f"{made.status_code} {made.text[:200]}"):
            employee = made.json()["id"]
        about = edge.get("/api/time/meta", headers=bearer(manager)).json()
        report.check("the page is told who it works as: manager, may edit", about.get("hub", {}).get("role") == "manager" and about.get("hub", {}).get("can_edit") is True, str(about.get("hub")))
        report.check("the bot is running", bool((about.get("bot") or {}).get("enabled")), str(about.get("bot")))
    viewer = tokens.get("e2e_viewer")
    if viewer:
        about = edge.get("/api/time/meta", headers=bearer(viewer)).json()
        report.check("the viewer's page: viewer, may not edit", about.get("hub", {}).get("role") == "viewer" and about.get("hub", {}).get("can_edit") is False, str(about.get("hub")))
        report.answer("the viewer cannot add an employee", edge.post("/api/time/employees", headers=bearer(viewer), json={"full_name": "X", "nick": "x"}), 403, ROLE)
    if manager:
        report.answer("a route Time has but the gateway does not offer (its sign-in): 404", edge.post("/api/time/login", headers=bearer(manager), json={"password": "x"}), 404, "time_route_not_found")
        report.answer("a path that climbs out: 404", edge.get("/api/time/..%2F..%2Fapi%2Fmeta", headers=bearer(manager)), 404)
    report.answer("Time without signing in: 401", edge.get("/api/time/meta"), 401)

    report.section("who gets what: " + ", ".join(PROBES))
    for username, _, _ in ACCOUNTS:
        token = tokens.get(username)
        if not token:
            continue
        got = [(response.status_code, error_code(response) if response.status_code != 200 else None) for response in probe(token, employee)]
        wanted = list(EXPECTED[username])
        got = [(status, code if isinstance(code, str) or code is None else str(code)) for status, code in got]
        report.check(f"{username}: " + " ".join(str(status) for status, _ in wanted), got == wanted, f"got {got}")
    for username in ("e2e_viewer", "e2e_both"):
        token = tokens.get(username)
        if token:
            book = edge.get("/api/time/export/2026-11", headers=bearer(token))
            report.check(
                f"{username}'s Time workbook is a workbook, not cached, with no cookie",
                book.content[:2] == b"PK" and book.headers.get("content-type") == XLSX and book.headers.get("cache-control") == "no-store" and "set-cookie" not in book.headers,
                f"{book.status_code} {dict(book.headers)}",
            )
    return ids, tokens, employee


def month_cells(token, month, employee):
    """One employee's days of a month as Time shows them, or None."""
    answer = edge.get(f"/api/time/month/{month}", headers=bearer(token))
    if answer.status_code != 200:
        return None
    for row in answer.json().get("employees", []):
        if row.get("id") == employee:
            return json.dumps(row.get("cells"), sort_keys=True, ensure_ascii=False)
    return None


def journal_and_lock(tokens, employee):
    report.section("Time: the change journal and a closed month")
    manager, viewer = tokens.get("e2e_manager"), tokens.get("e2e_viewer")
    if not (manager and viewer and employee):
        return report.check("needs the manager, the viewer and an employee", False)
    edit = edge.put(f"/api/time/marks/{employee}/2026-11-11", headers={**bearer(manager), "X-Timelist-User": "forged-name"}, json={"set": {"start": "9:30"}})
    report.answer("the manager edits a mark", edit, 200)
    journal = edge.get("/api/time/audit?limit=20", headers=bearer(viewer))
    report.check("the journal names the account, not what the client claimed", journal.status_code == 200 and "e2e_manager" in journal.text and "forged-name" not in journal.text, journal.text[:300])
    report.answer("the viewer cannot close a month", edge.post("/api/time/months/2026-10/lock", headers=bearer(viewer)), 403, ROLE)
    report.answer("the manager closes October", edge.post("/api/time/months/2026-10/lock", headers=bearer(manager)), 200)
    report.answer("an edit in the closed month: 409", edge.put(f"/api/time/marks/{employee}/2026-10-06", headers=bearer(manager), json={"set": {"start": "9:00"}}), 409)
    # Time freezes a closed month rather than refusing everything that touches
    # its dates: an absence entered later waits until the month is opened.
    frozen = month_cells(manager, "2026-10", employee)
    absence = edge.post("/api/time/periods", headers=bearer(manager), json={"employee_ids": [employee], "code": "О", "date_from": "2026-10-05", "date_to": "2026-10-07"})
    report.check("an absence entered over the closed month is taken", absence.status_code == 200 and absence.json().get("ids"), f"{absence.status_code} {absence.text[:160]}")
    report.check("and the closed month stays as it was", frozen is not None and month_cells(manager, "2026-10", employee) == frozen, str(frozen)[:200])
    report.answer("the open month still takes edits", edge.put(f"/api/time/marks/{employee}/2026-11-12", headers=bearer(manager), json={"note": "open"}), 200)
    report.answer("the manager opens October again", edge.delete("/api/time/months/2026-10/lock", headers=bearer(manager)), 200)
    report.check("now the absence shows", frozen is not None and month_cells(manager, "2026-10", employee) != frozen)
    for period in (absence.json().get("ids") or []) if absence.status_code == 200 else []:
        report.answer("the absence is removed", edge.delete(f"/api/time/periods/{period}", headers=bearer(manager)), 200)
    report.answer("and the edit goes through", edge.put(f"/api/time/marks/{employee}/2026-10-06", headers=bearer(manager), json={"set": {"start": "9:00"}}), 200)


def revocation(admin, ids, tokens, employee):
    report.section("revocation and grant while signed in (the same token throughout)")
    both, user_id = tokens.get("e2e_both"), ids.get("e2e_both")
    if not both:
        return report.check("needs e2e_both", False)
    auth = bearer(both)
    report.answer("Time is taken away", patch_user(admin, user_id, workspaces={"time": None}), 200)
    report.answer("the next Time call is refused", edge.get("/api/time/meta", headers=auth), 403, WF)
    report.answer("so is the workbook", edge.get("/api/time/export/2026-11", headers=auth), 403, WF)
    report.check("the Hub is told: Checks only", hub_fields(both) == (["checks"], {"checks": "operator"}, False), str(hub_fields(both)))
    report.answer("Checks goes on working", edge.get("/api/transactions/?page_size=1", headers=auth), 200)
    report.answer("Time is granted again, as a viewer", patch_user(admin, user_id, workspaces={"time": "viewer"}), 200)
    report.answer("reading works at once", edge.get("/api/time/meta", headers=auth), 200)
    report.answer("editing does not", edge.put(f"/api/time/marks/{employee}/2026-11-10", headers=auth, json={"note": "x"}), 403, ROLE)
    report.answer("Checks is taken away", patch_user(admin, user_id, workspaces={"checks": None}), 200)
    report.answer("the next Checks call is refused", edge.get("/api/transactions/?page_size=1", headers=auth), 403, WF)
    report.answer("so is the Checks export", edge.get("/api/transactions/export", headers=auth), 403, WF)
    report.check("the Hub is told: Time only", hub_fields(both) == (["time"], {"time": "viewer"}, False), str(hub_fields(both)))
    report.answer("both are given back", patch_user(admin, user_id, workspaces={"checks": "operator", "time": "manager"}), 200)
    report.check("the Hub is told: both", hub_fields(both) == (["checks", "time"], {"checks": "operator", "time": "manager"}, False), str(hub_fields(both)))

    nobody, nobody_id = tokens.get("e2e_none"), ids.get("e2e_none")
    if nobody:
        report.answer("CRM administration is given to an account with no workspace", patch_user(admin, nobody_id, global_admin=True), 200)
        report.answer("it administers at once", edge.get("/api/admin/users", headers=bearer(nobody)), 200)
        report.check("and still has no workspace", hub_fields(nobody) == ([], {}, True), str(hub_fields(nobody)))
        report.answer("it is taken away", patch_user(admin, nobody_id, global_admin=False), 200)
        report.answer("the next call is refused", edge.get("/api/admin/users", headers=bearer(nobody)), 403, NOT_ADMIN)
    crm, crm_id = tokens.get("e2e_crm"), ids.get("e2e_crm")
    if crm:
        report.answer("an administrator cannot take the right from themselves", patch_user(crm, crm_id, global_admin=False), 409, "cannot_demote_self")
        report.answer("nor disable themselves", patch_user(crm, crm_id, is_active=False), 409, "cannot_disable_self")
        report.answer("a Time manager cannot change accounts", patch_user(tokens.get("e2e_manager") or "", crm_id, global_admin=False), 403, NOT_ADMIN)
        report.answer("a Checks operator cannot create accounts", edge.post("/api/admin/users", headers=bearer(tokens.get("e2e_checks") or ""), json={"username": "x", "password": PASSWORD}), 403, NOT_ADMIN)


def session_count(admin, user_id):
    listed = edge.get(f"/api/admin/users/{user_id}/sessions", headers=bearer(admin))
    return len(listed.json().get("sessions", [])) if listed.status_code == 200 else f"{listed.status_code} {listed.text[:120]}"


def sessions(admin, ids, tokens):
    report.section("sessions")
    user_id = ids.get("e2e_checks")
    first, second = sign_in("e2e_checks").json(), sign_in("e2e_checks").json()
    if not (first.get("token") and second.get("token")):
        return report.check("e2e_checks signs in twice more", False, str(first)[:200])
    report.check("the administrator sees the account's three sessions", session_count(admin, user_id) == 3, str(session_count(admin, user_id)))

    rotated = edge.post("/api/auth/refresh", json={"refresh_token": first["refresh_token"]})
    renewed = rotated.json() if rotated.status_code == 200 else {}
    report.check("a refresh token is exchanged for a new pair", bool(renewed.get("token") and renewed.get("refresh_token") and renewed["refresh_token"] != first["refresh_token"]), rotated.text[:200])
    if renewed.get("token"):
        report.check("the new access token works", me(renewed["token"]).get("username") == "e2e_checks", str(me(renewed["token"])))
    report.check("the exchange continues the session, it does not add one", session_count(admin, user_id) == 3, str(session_count(admin, user_id)))
    report.answer("the used refresh token is refused the second time", edge.post("/api/auth/refresh", json={"refresh_token": first["refresh_token"]}), 401)
    if renewed.get("token"):
        report.check("the session it was exchanged in goes on", me(renewed["token"]).get("username") == "e2e_checks", str(me(renewed["token"])))

    expired = with_expiry(second["token"], datetime.now(timezone.utc) - timedelta(minutes=5))
    report.answer("an access token past its expiry: 401", edge.get("/api/auth/me", headers=bearer(expired)), 401)
    report.answer("so is it at the Time gateway", edge.get("/api/time/meta", headers=bearer(expired)), 401)
    forged = second["token"][:-6] + ("AAAAAA" if not second["token"].endswith("AAAAAA") else "BBBBBB")
    report.answer("a token with a broken signature: 401", edge.get("/api/auth/me", headers=bearer(forged)), 401)

    report.answer("signing out", edge.post("/api/auth/logout", headers=bearer(second["token"])), 200)
    report.answer("the signed-out token: 401", edge.get("/api/auth/me", headers=bearer(second["token"])), 401)
    report.check("the other sessions of the account are untouched", me(tokens["e2e_checks"]).get("username") == "e2e_checks", str(me(tokens["e2e_checks"])))
    left = session_count(admin, user_id)
    report.check("two sessions are left", left == 2, str(left))

    ended = edge.post(f"/api/admin/users/{user_id}/revoke-sessions", headers=bearer(admin))
    report.check("the administrator ends them", ended.status_code == 200 and ended.json().get("revoked") == left, ended.text[:200])
    report.answer("the first token: 401", edge.get("/api/auth/me", headers=bearer(tokens["e2e_checks"])), 401)
    if renewed.get("token"):
        report.answer("the renewed token: 401", edge.get("/api/auth/me", headers=bearer(renewed["token"])), 401)
        report.answer("its refresh token: 401", edge.post("/api/auth/refresh", json={"refresh_token": renewed["refresh_token"]}), 401)
    report.check("the administrator's own session is untouched", me(admin).get("username") == "e2e_admin", str(me(admin)))
    again = token_of("e2e_checks")
    if again:
        tokens["e2e_checks"] = again
        report.check("the account signs in again with the access it had", hub_fields(again) == (["checks"], {"checks": "operator"}, False), str(hub_fields(again)))

    report.section("deactivation while signed in")
    manager_id, manager = ids.get("e2e_manager"), tokens.get("e2e_manager")
    report.answer("the administrator disables the Time manager", patch_user(admin, manager_id, is_active=False), 200)
    report.answer("the manager's next Time call is refused: the session is over", edge.get("/api/time/meta", headers=bearer(manager)), 401, "session_inactive")
    report.answer("so is the account's own data", edge.get("/api/auth/me", headers=bearer(manager)), 401, "session_inactive")
    refused = sign_in("e2e_manager")
    report.check("and signing in", refused.status_code == 403 and refused.json().get("error") == "inactive_user", f"{refused.status_code} {refused.text[:120]}")
    report.answer("the account is enabled again", patch_user(admin, manager_id, is_active=True), 200)
    back = token_of("e2e_manager")
    if back:
        tokens["e2e_manager"] = back
        report.check("it signs in with the access it had", hub_fields(back) == (["time"], {"time": "manager"}, False), str(hub_fields(back)))


def lockout_and_two_factor(admin):
    report.section("sign-in protection")
    made = edge.post("/api/admin/users", headers=bearer(admin), json={"username": "e2e_lock", "password": PASSWORD, "workspaces": {"time": "viewer"}})
    lock_id = made.json().get("id") if made.status_code == 200 else None
    left = []
    for _ in range(5):
        wrong = sign_in("e2e_lock", "wrong-password")
        left.append((wrong.status_code, wrong.json().get("attempts_left")))
    report.check("five wrong passwords are refused, the attempts counted down", [status for status, _ in left] == [401] * 4 + [423] or [status for status, _ in left] == [401] * 5, str(left))
    locked = sign_in("e2e_lock")
    report.check("then the right password is refused too: locked", locked.status_code == 423 and locked.json().get("error") == "locked", f"{locked.status_code} {locked.text[:160]}")
    report.answer("the administrator unlocks the account", edge.post(f"/api/admin/users/{lock_id}/unlock", headers=bearer(admin)), 200)
    report.check("and it signs in", bool(token_of("e2e_lock")))

    import pyotp

    made = edge.post("/api/admin/users", headers=bearer(admin), json={"username": "e2e_2fa", "password": PASSWORD, "force_2fa": True, "workspaces": {"time": "viewer"}})
    report.check("an account that must use a second factor is created", made.status_code == 200, made.text[:200])
    first = sign_in("e2e_2fa").json()
    report.check("its password alone gives no session, and asks to set the factor up", not first.get("token") and first.get("requires_2fa") and first.get("requires_2fa_setup") and first.get("temp_token"), str({k: v for k, v in first.items() if k != "temp_token"}))
    setup = edge.post("/api/2fa/setup", json={"temp_token": first.get("temp_token")})
    secret = setup.json().get("secret") if setup.status_code == 200 else None
    report.check("the set-up hands out a secret", bool(secret), f"{setup.status_code} {setup.text[:120]}")
    if not secret:
        return
    totp = pyotp.TOTP(secret)
    report.answer("a wrong code does not confirm it", edge.post("/api/2fa/confirm", json={"temp_token": first["temp_token"], "code": "000000" if totp.now() != "000000" else "111111"}), 401, "invalid_code")
    report.answer("the right code confirms it", edge.post("/api/2fa/confirm", json={"temp_token": first["temp_token"], "code": totp.now()}), 200)
    second = sign_in("e2e_2fa").json()
    report.check("the password now asks for the code, and gives no session", not second.get("token") and second.get("requires_2fa") and not second.get("requires_2fa_setup"), str({k: v for k, v in second.items() if k != "temp_token"}))
    report.answer("a wrong code is refused", edge.post("/api/auth/login/2fa", json={"temp_token": second.get("temp_token"), "code": "000000" if totp.now() != "000000" else "111111"}), 401, "invalid_code")
    done = edge.post("/api/auth/login/2fa", json={"temp_token": second.get("temp_token"), "code": totp.now()})
    if done.status_code == 401:
        # A code is good once; the confirmation above may have used this half-minute's.
        time.sleep(31 - int(time.time()) % 30)
        done = edge.post("/api/auth/login/2fa", json={"temp_token": second.get("temp_token"), "code": totp.now()})
    token = done.json().get("token") if done.status_code == 200 else None
    report.check("the right code signs in", bool(token), f"{done.status_code} {done.text[:160]}")
    if token:
        report.answer("and Time opens", edge.get("/api/time/meta", headers=bearer(token)), 200)

    policy = edge.patch("/api/system-settings", headers=bearer(admin), json={"totp_2fa_enabled": True})
    if report.check("the administrator turns the second factor on for administrators", policy.status_code == 200 and policy.json().get("totp_2fa_enabled") is True, f"{policy.status_code} {policy.text[:200]}"):
        crm = sign_in("e2e_crm").json()
        report.check("a CRM administrator with no workspace is then asked for it", not crm.get("token") and crm.get("requires_2fa") is True, str({k: v for k, v in crm.items() if k != "temp_token"}))
        plain = sign_in("e2e_viewer").json()
        report.check("an account that administers nothing is not", bool(plain.get("token")), str({k: v for k, v in plain.items() if k not in ("token", "refresh_token", "scope_token")})[:200])
        report.check("the administrator's open session goes on", me(admin).get("username") == "e2e_admin", str(me(admin)))
        back = edge.patch("/api/system-settings", headers=bearer(admin), json={"totp_2fa_enabled": False})
        report.check("the setting is put back", back.status_code == 200 and back.json().get("totp_2fa_enabled") is False, back.text[:200])


# ---------------------------------------------------------------- the bot


def post_update(chat, sender, text, **more):
    return telegram.post("/_fake/update", json={"chat_id": chat, "from_id": sender, "text": text, **more}).json()


def bot_read(update, patience=40.0):
    """True once the bot has confirmed the update, which it does after handling it."""
    deadline = time.monotonic() + patience
    while time.monotonic() < deadline:
        if telegram.get("/_fake/state").json().get("read_up_to", 0) >= update["update_id"]:
            return True
        time.sleep(0.3)
    return False


def bot_calls(message_id=None, method=None):
    calls = telegram.get("/_fake/calls").json()
    return [
        call for call in calls
        if (method is None or call.get("method") == method) and (message_id is None or message_id in (call.get("message_id"), (call.get("reply_parameters") or {}).get("message_id"), call.get("reply_to_message_id")))
    ]


def reaction(update):
    marks = [json.dumps(call.get("reaction"), ensure_ascii=False) for call in bot_calls(update["message_id"], "setMessageReaction")]
    for emoji in ("✍", "🤷", "🤔"):
        if any(emoji in mark for mark in marks):
            return emoji
    return None


def stored(manager, month, text):
    listed = edge.get(f"/api/time/messages?ym={month}", headers=bearer(manager))
    return [item for item in listed.json() if item.get("text") == text] if listed.status_code == 200 else []


def said(chat, sender, text, **more):
    update = post_update(chat, sender, text, **more)
    return update, bot_read(update)


def bot(admin, ids, tokens, employee):
    report.section("the Time bot (a stand-in for Telegram; synthetic senders and groups)")
    manager = tokens.get("e2e_manager")
    if not (manager and employee):
        return report.check("needs the manager and an employee", False)
    month = (datetime.now(timezone.utc) + timedelta(hours=5)).strftime("%Y-%m")
    state = telegram.get("/_fake/state").json()
    report.check("the bot polls the stand-in", state.get("polls", 0) > 0, str(state))
    person = {"first_name": "Synthetic", "last_name": "Employee"}
    identity = backend.get(f"/internal/time/telegram/{TG_EMPLOYEE}", headers={"X-Time-Hub-Key": HUB_KEY})
    report.check("the backend knows the employee's Telegram ID", identity.status_code == 200 and identity.json().get("allowed") is True and identity.json().get("time_role") == "employee", identity.text[:200])

    update, read = said(WORK_GROUP, TG_EMPLOYEE, "офис 9:10", **person)
    report.check("an employee account's message in the work group is read", read)
    kept = stored(manager, month, "офис 9:10")
    report.check("and kept, with nobody to count it for yet", len(kept) == 1 and kept[0].get("author_id") is None, str(kept)[:200])
    report.check("the bot answers 🤷 until the sender is bound to an employee", reaction(update) == "🤷", str(bot_calls(update["message_id"])))
    people = edge.get("/api/time/tg/users", headers=bearer(manager))
    report.check("the sender is offered for binding", str(TG_EMPLOYEE) in people.text, people.text[:200])
    report.answer("the viewer cannot bind", edge.post(f"/api/time/tg/users/{TG_EMPLOYEE}/link", headers=bearer(tokens.get("e2e_viewer") or ""), json={"employee_id": employee}), 403, ROLE)
    report.answer("the manager binds the sender to the employee", edge.post(f"/api/time/tg/users/{TG_EMPLOYEE}/link", headers=bearer(manager), json={"employee_id": employee}), 200)
    update, read = said(WORK_GROUP, TG_EMPLOYEE, "уход 18:05", **person)
    kept = stored(manager, month, "уход 18:05")
    report.check("the next message is counted for the employee", read and len(kept) == 1 and kept[0].get("author_id") == employee and kept[0].get("recognized") is True, str(kept)[:300])
    report.check("and answered ✍", reaction(update) == "✍", str(bot_calls(update["message_id"])))
    earlier = stored(manager, month, "офис 9:10")
    report.check("the earlier message now counts too", len(earlier) == 1 and earlier[0].get("author_id") == employee, str(earlier)[:200])

    update, read = said(WORK_GROUP, TG_STRANGER, "офис 8:00", first_name="Unknown", last_name="Sender")
    report.check("a sender no account knows: read, answered 🤷, nothing kept", read and reaction(update) == "🤷" and not stored(manager, month, "офис 8:00"), f"read={read} {reaction(update)} {stored(manager, month, 'офис 8:00')}")

    report.answer("the employee's Time access is taken away", patch_user(admin, ids["e2e_employee"], workspaces={"time": None}), 200)
    update, read = said(WORK_GROUP, TG_EMPLOYEE, "офис 8:01", **person)
    report.check("the next message is refused: 🤷, nothing kept", read and reaction(update) == "🤷" and not stored(manager, month, "офис 8:01"), f"read={read} {reaction(update)}")
    report.answer("it is given back", patch_user(admin, ids["e2e_employee"], workspaces={"time": "employee"}), 200)
    report.answer("the account is disabled", patch_user(admin, ids["e2e_employee"], is_active=False), 200)
    update, read = said(WORK_GROUP, TG_EMPLOYEE, "офис 8:02", **person)
    report.check("a disabled account's message is refused: 🤷, nothing kept", read and reaction(update) == "🤷" and not stored(manager, month, "офис 8:02"), f"read={read} {reaction(update)}")
    private = post_update(TG_EMPLOYEE, TG_EMPLOYEE, "/me", **person)
    answered = bot_read(private) and [call.get("text", "") for call in bot_calls(method="sendMessage") if call.get("chat_id") == TG_EMPLOYEE]
    report.check("asked /me, the bot tells a disabled account why", bool(answered) and "отключена" in answered[-1], str(answered)[:200])
    report.answer("the account is enabled", patch_user(admin, ids["e2e_employee"], is_active=True), 200)
    update, read = said(WORK_GROUP, TG_EMPLOYEE, "уход 18:30", **person)
    report.check("and marks again: kept, ✍", read and reaction(update) == "✍" and len(stored(manager, month, "уход 18:30")) == 1, f"read={read} {reaction(update)}")

    before = len(telegram.get("/_fake/calls").json())
    update, read = said(OTHER_GROUP, TG_EMPLOYEE, "офис 7:00", chat_title="another group", **person)
    after = len(telegram.get("/_fake/calls").json())
    report.check("the same person in another group: read, not answered, nothing kept", read and after == before and not stored(manager, month, "офис 7:00"), f"read={read} calls {before}->{after}")

    asked = post_update(TG_STRANGER, TG_STRANGER, "/id", first_name="Unknown")
    replies = bot_read(asked) and [call.get("text", "") for call in bot_calls(method="sendMessage") if call.get("chat_id") == TG_STRANGER]
    report.check("/id in private answers with the sender's Telegram ID", bool(replies) and str(TG_STRANGER) in replies[-1], str(replies)[:200])
    asked = post_update(TG_STRANGER, TG_STRANGER, "/me", first_name="Unknown")
    replies = bot_read(asked) and [call.get("text", "") for call in bot_calls(method="sendMessage") if call.get("chat_id") == TG_STRANGER]
    report.check("/me from an unknown sender says the ID is linked to no account", bool(replies) and "не привязан" in replies[-1], str(replies)[:200])
    passwords = [call for call in telegram.get("/_fake/calls").json() if PASSWORD in json.dumps(call, ensure_ascii=False)]
    report.check("nothing the bot sent carries a password", not passwords)
    day = edge.get(f"/api/time/day/{employee}/{(datetime.now(timezone.utc) + timedelta(hours=5)).strftime('%Y-%m-%d')}", headers=bearer(manager))
    report.check("the employee's day shows the messages that were accepted", day.status_code == 200 and "уход 18:30" in day.text and "офис 8:02" not in day.text, day.text[:200])


def main():
    mode = sys.argv[1] if len(sys.argv) > 1 else "hub"
    if mode in ("legacy", "legacy-writes"):
        legacy(mode == "legacy-writes")
        report.finish()
    edge_and_gates()
    found = migrated_accounts()
    if found:
        admin, _ = found
        ids, tokens, employee = matrix(admin)
        journal_and_lock(tokens, employee)
        revocation(admin, ids, tokens, employee)
        sessions(admin, ids, tokens)
        bot(admin, ids, tokens, employee)
        lockout_and_two_factor(admin)
    if limited:
        print(f"\nthe sign-in limit (ten a minute from one address) answered 429 {len(limited)} time(s); each was waited out")
    report.finish()


if __name__ == "__main__":
    main()
