"""The Time gateway and the bot's question.

Time itself is not started here: `hub_support.TimeStub` stands in for it and
records exactly what the gateway forwarded. What these tests establish is the
gateway's own behaviour — who gets through, what is sent on, what never is.
The real Time server behind the real gateway is exercised by the local
end-to-end run described in docs/handoff/TIME_HUB_ADMIN_PROGRESS.md.
"""
from __future__ import annotations

from datetime import datetime, timedelta, timezone
from urllib.parse import unquote

import pytest

from api.routes import time_gateway
from database.models import AccessAuditLog
from services.workspace_access import time_bot_decision
from hub_support import (
    TIME_TOKEN,
    TIME_URL,
    app_client,
    bearer,
    call_from,
    error_code,
    install_time_stub,
    seed,
)

HUB_KEY = "time-hub-key-0123456789abcdef-test"


@pytest.fixture
def client(db_session, monkeypatch, tmp_path):
    with app_client(db_session, monkeypatch, tmp_path) as test_client:
        test_client.time = install_time_stub(monkeypatch)
        monkeypatch.setenv("TIME_HUB_KEY", HUB_KEY)
        yield test_client


# ---------------------------------------------------------------- who gets through


READS = [
    ("GET", "meta"), ("GET", "month/2026-11"), ("GET", "day/3/2026-11-02"), ("GET", "messages?ym=2026-11"),
    ("POST", "parse"), ("GET", "employees"), ("GET", "codes"), ("GET", "settings"), ("GET", "calendar/2026"),
    ("GET", "periods"), ("GET", "tg/users"), ("GET", "audit?limit=5"), ("GET", "export/2026-11"),
    ("GET", "export/2026-11.xlsx"),
]
WRITES = [
    ("PUT", "marks/3/2026-11-02"), ("POST", "months/2026-11/lock"), ("DELETE", "months/2026-11/lock"),
    ("PUT", "months/2026-11/limit"), ("POST", "messages"), ("POST", "messages/import"), ("PATCH", "messages/5"),
    ("DELETE", "messages/5"), ("POST", "employees"), ("PUT", "employees/3"), ("DELETE", "employees/3"),
    ("PUT", "codes/%D0%91"), ("PUT", "codes/%D0%9A1"), ("PUT", "settings"), ("PUT", "calendar/2026/norms/11"),
    ("DELETE", "calendar/2026/norms/11"), ("PUT", "calendar/days/2026-11-03"),
    ("DELETE", "calendar/days/2026-11-03"), ("POST", "periods"), ("DELETE", "periods/4"),
    ("POST", "tg/users/111222333/link"), ("POST", "import/xlsx"),
]


def _call(client, headers, method, path):
    body = None if method in {"GET", "DELETE"} else {}
    return client.request(method, f"/api/time/{path}", headers=headers, json=body)


def test_the_route_table_covers_time_and_only_time():
    """Every route Time serves is either offered with a role or deliberately left out."""
    import re
    from pathlib import Path

    source = (Path(__file__).resolve().parents[2] / "time/crates/server/src/api.rs").read_text(encoding="utf-8")
    router = source[source.index("pub fn router("):source.index("// ---------------------------------------------------------------- интерфейс")]
    served = set(re.findall(r'"(/[a-z{}/_]+)"', router)) - {"/api", "/", "/assets/{file}"}
    assert len(served) >= 25

    def sample(route: str) -> str:
        fill = {"{ym}": "2026-11", "{date}": "2026-11-02", "{employee}": "3", "{id}": "5", "{code}": "Б",
                "{year}": "2026", "{month}": "11"}
        for name, value in fill.items():
            route = route.replace(name, value)
        return route.lstrip("/")

    offered = {route for route in served
               if any(time_gateway.required_role(method, sample(route)) for method in ("GET", "POST", "PUT", "PATCH", "DELETE"))}
    # Time's password login and its event stream are not part of the workspace.
    assert served - offered == {"/login", "/logout", "/events"}


def test_a_manager_reaches_everything_and_a_viewer_only_reads(client, db_session):
    manager = bearer(seed(db_session, "boss", checks=False, time_role="manager"))
    viewer = bearer(seed(db_session, "reader", checks=False, time_role="viewer"))

    for method, path in READS + WRITES:
        assert _call(client, manager, method, path).status_code == 200, (method, path)
    for method, path in READS:
        assert _call(client, viewer, method, path).status_code == 200, (method, path)

    seen_before = len(client.time.requests)
    for method, path in WRITES:
        refused = _call(client, viewer, method, path)
        assert refused.status_code == 403, (method, path, refused.text)
        assert refused.json()["detail"] == {"error": "time_role_forbidden", "required": "manager"}
    assert len(client.time.requests) == seen_before, "a refused write never reaches Time"


@pytest.mark.parametrize("checks,time_role", [(True, None), (False, None), (True, "employee"), (False, "employee")])
def test_without_a_screen_role_nothing_of_time_is_reachable(client, db_session, checks, time_role):
    headers = bearer(seed(db_session, "anna", role="admin", checks=checks, time_role=time_role))
    for method, path in READS + WRITES:
        refused = _call(client, headers, method, path)
        assert refused.status_code == 403 and error_code(refused) == "workspace_forbidden", (method, path)
    assert client.time.requests == []
    assert _call(client, {}, "GET", "month/2026-11").status_code == 401


@pytest.mark.parametrize(
    "method,path",
    [
        ("POST", "login"), ("POST", "logout"), ("GET", "events"), ("GET", ""), ("GET", "month"),
        ("GET", "month/2026-11/extra"), ("GET", "month/..%2F..%2Fapi%2Flogin"), ("GET", "codes/..%2Fsettings"),
        ("PUT", "codes/.."), ("PUT", "codes/%D0%91/x"), ("GET", "export/2026-11.csv"), ("DELETE", "employees"),
        ("PUT", "marks/3/2026-11-02/../../settings"), ("GET", "tg/users/1/link"), ("POST", "tg/users/-100/link"),
        ("GET", "assets/app.js"), ("GET", "day/3;drop/2026-11-02"), ("PATCH", "settings"),
    ],
)
def test_a_route_that_is_not_offered_does_not_exist_even_for_a_manager(client, db_session, method, path):
    manager = bearer(seed(db_session, "boss", checks=False, time_role="manager"))
    response = _call(client, manager, method, path)
    assert response.status_code in (404, 405), (method, path, response.status_code, response.text)
    assert client.time.requests == []


# ---------------------------------------------------------------- what is sent on, and what never is


def test_time_is_called_with_the_servers_token_and_the_accounts_own_name(client, db_session):
    user = seed(db_session, "boss", checks=False, time_role="manager")
    user.display_name = "Анна Петрова"
    db_session.commit()
    headers = {
        **bearer(user),
        # What a client might try to slip through.
        "X-Timelist-User": "%D0%94%D0%B8%D1%80%D0%B5%D0%BA%D1%82%D0%BE%D1%80",
        "Cookie": "tl_session=stolen; refresh_token=abc",
        "X-Access-Token": "scope-token",
    }
    response = client.put("/api/time/marks/3/2026-11-02?note=1", headers=headers, json={"set": {"start": "9:30"}})
    assert response.status_code == 200, response.text
    assert response.headers["cache-control"] == "no-store"

    sent = client.time.last
    assert str(sent.url) == f"{TIME_URL}/api/marks/3/2026-11-02?note=1"
    assert sent.headers["authorization"] == f"Bearer {TIME_TOKEN}"
    assert unquote(sent.headers["x-timelist-user"]) == "Анна Петрова (boss)"
    assert sent.content == b'{"set": {"start": "9:30"}}'
    # The user's token, cookies and the system token stay on this side.
    forwarded = {name.lower() for name in sent.headers}
    assert not forwarded & {"cookie", "x-system-access", "x-access-token", "x-launch-session", "x-refresh-token"}
    assert headers["Authorization"].split()[1] not in str(sent.headers)


def test_a_cyrillic_code_survives_the_way_to_time(client, db_session):
    manager = bearer(seed(db_session, "boss", checks=False, time_role="manager"))
    assert client.put("/api/time/codes/%D0%9A1", headers=manager, json={"coef": 1}).status_code == 200
    assert client.time.last.url.raw_path == b"/api/codes/%D0%9A1"


def test_an_export_keeps_its_file_name_and_nothing_else_of_times_headers(client, db_session):
    viewer = bearer(seed(db_session, "reader", checks=False, time_role="viewer"))
    response = client.get("/api/time/export/2026-11", headers=viewer)
    assert response.status_code == 200
    assert response.content == b"PK\x03\x04 workbook"
    assert response.headers["content-disposition"] == 'attachment; filename="TimeList-2026-11.xlsx"'
    assert response.headers["content-type"].startswith("application/vnd.openxmlformats")
    assert "set-cookie" not in response.headers


def test_the_page_is_told_who_it_works_as(client, db_session):
    viewer = seed(db_session, "reader", checks=False, time_role="viewer")
    meta = client.get("/api/time/meta", headers=bearer(viewer)).json()
    assert meta["revision"] == 7 and meta["bot"] == {"enabled": True}
    assert meta["hub"] == {"username": "reader", "display_name": "Reader", "role": "viewer", "can_edit": False}
    manager = seed(db_session, "boss", checks=False, time_role="manager")
    assert client.get("/api/time/meta", headers=bearer(manager)).json()["hub"]["can_edit"] is True


# ---------------------------------------------------------------- failures that must not look like something else


def test_a_refusal_by_time_is_never_passed_on_as_a_lost_session(client, db_session, monkeypatch):
    manager = bearer(seed(db_session, "boss", checks=False, time_role="manager"))
    # The server's own token is wrong: a deployment fault, not the user's session.
    monkeypatch.setenv("TIMELIST_API_TOKEN", "not-the-token-time-expects")
    response = client.get("/api/time/month/2026-11", headers=manager)
    assert response.status_code == 502 and error_code(response) == "time_upstream_refused"
    # The session is intact.
    assert client.get("/api/auth/me", headers=manager).status_code == 200


def test_time_being_down_or_absent_is_said_plainly(client, db_session, monkeypatch):
    manager = bearer(seed(db_session, "boss", checks=False, time_role="manager"))
    client.time.down = True
    down = client.get("/api/time/month/2026-11", headers=manager)
    assert down.status_code == 502 and error_code(down) == "time_upstream_unavailable"

    monkeypatch.delenv("TIMELIST_INTERNAL_URL")
    absent = client.get("/api/time/month/2026-11", headers=manager)
    assert absent.status_code == 503 and error_code(absent) == "time_workspace_unavailable"
    assert time_gateway.is_configured() is False
    # Access is still judged first: an account without Time learns nothing about the deployment.
    outsider = bearer(seed(db_session, "anna"))
    assert error_code(client.get("/api/time/month/2026-11", headers=outsider)) == "workspace_forbidden"


def test_times_own_answers_reach_the_page_as_they_are(client, db_session):
    manager = bearer(seed(db_session, "boss", checks=False, time_role="manager"))
    client.time.status = 409
    locked = client.put("/api/time/marks/3/2026-11-02", headers=manager, json={})
    assert locked.status_code == 409 and locked.json() == {"error": "отказ"}


def test_a_request_body_has_a_ceiling(client, db_session):
    manager = bearer(seed(db_session, "boss", checks=False, time_role="manager"))
    too_much = client.post(
        "/api/time/periods", headers={**manager, "Content-Type": "application/json"}, content=b"x" * (300 * 1024))
    assert too_much.status_code == 413 and error_code(too_much) == "time_request_too_large"
    # A workbook may be larger than a form.
    workbook = client.post("/api/time/import/xlsx", headers={**manager, "Content-Type": "application/octet-stream"},
                           content=b"x" * (300 * 1024))
    assert workbook.status_code == 200
    assert client.time.last.headers["content-type"] == "application/octet-stream"
    assert len(client.time.last.content) == 300 * 1024


# ---------------------------------------------------------------- the bot's question


def _ask(telegram_id, *, host="172.18.0.7", key=HUB_KEY, extra=None):
    headers = {"X-Time-Hub-Key": key} if key else {}
    headers.update(extra or {})
    return call_from(host, "GET", f"/internal/time/telegram/{telegram_id}", headers=headers)


def test_the_bot_is_answered_from_the_account_as_it_is_right_now(client, db_session):
    admin = seed(db_session, "root", role="admin")
    anna = seed(db_session, "anna", checks=False, time_role="employee", telegram_id=111)
    admin_headers = bearer(admin)

    answer = _ask(111)
    assert answer.status_code == 200
    assert answer.json() == {"allowed": True, "reason": "ok", "time_role": "employee", "user_id": anna.id,
                             "username": "anna", "display_name": "Anna"}
    assert _ask(999).json() == {"allowed": False, "reason": "not_linked"}

    # Each administrative change is the bot's very next answer: nothing is cached.
    client.patch(f"/api/admin/users/{anna.id}", headers=admin_headers, json={"workspaces": {"time": None}})
    assert (_ask(111).json()["allowed"], _ask(111).json()["reason"]) == (False, "no_time_access")
    client.patch(f"/api/admin/users/{anna.id}", headers=admin_headers, json={"workspaces": {"time": "manager"}})
    assert _ask(111).json()["allowed"] is True

    client.delete(f"/api/admin/users/{anna.id}", headers=admin_headers)
    assert (_ask(111).json()["allowed"], _ask(111).json()["reason"]) == (False, "inactive")
    client.patch(f"/api/admin/users/{anna.id}", headers=admin_headers, json={"is_active": True})
    assert _ask(111).json()["allowed"] is True

    client.post(f"/api/admin/users/{anna.id}/unlink-telegram", headers=admin_headers)
    assert _ask(111).json() == {"allowed": False, "reason": "not_linked"}


def test_an_expired_entitlement_stops_the_bot_as_it_stops_the_screens(db_session):
    past = datetime.now(timezone.utc) - timedelta(days=1)
    user = seed(db_session, "anna", time_role="employee", telegram_id=111)
    user.access_until = past
    assert time_bot_decision(user)["reason"] == "access_expired"
    user.access_until = datetime.now(timezone.utc) + timedelta(days=1)
    assert time_bot_decision(user)["allowed"] is True
    # An administrator is never locked out by a date, here as everywhere else.
    admin = seed(db_session, "root", role="admin", time_role="manager", telegram_id=222)
    admin.access_until = past
    assert time_bot_decision(admin)["allowed"] is True
    # A group ID is nobody.
    assert time_bot_decision(None) == {"allowed": False, "reason": "not_linked"}


@pytest.mark.parametrize(
    "host,key,extra",
    [
        ("172.18.0.7", None, None),                                   # no key
        ("172.18.0.7", "wrong-key-0123456789abcdef-wrong", None),      # wrong key
        ("203.0.113.9", HUB_KEY, None),                               # right key, from outside
        ("172.18.0.7", HUB_KEY, {"X-Forwarded-For": "203.0.113.9"}),  # right key, through the public proxy
        ("172.18.0.7", HUB_KEY, {"X-Real-IP": "203.0.113.9"}),
    ],
)
def test_only_time_itself_may_ask(client, db_session, host, key, extra):
    seed(db_session, "anna", time_role="employee", telegram_id=111)
    refused = _ask(111, host=host, key=key, extra=extra)
    assert refused.status_code == 403 and refused.json()["detail"] == {"error": "time_hub_key_refused"}
    assert "anna" not in refused.text
    assert db_session.query(AccessAuditLog).filter(AccessAuditLog.action == "time_hub_key_refused").count() == 1


def test_no_other_credential_opens_the_bots_question(client, db_session, monkeypatch):
    monkeypatch.setenv("INTERNAL_API_KEY", "internal-secret-0123456789abcdef")
    admin = seed(db_session, "root", role="admin", time_role="manager", telegram_id=111)
    for headers in ({"X-Internal-API-Key": "internal-secret-0123456789abcdef"}, bearer(admin),
                    {"X-System-Access": "test-system-token"}, {"Authorization": f"Bearer {TIME_TOKEN}"}):
        assert call_from("172.18.0.7", "GET", "/internal/time/telegram/111", headers=headers).status_code == 403


def test_without_a_configured_key_the_question_is_closed(client, db_session, monkeypatch):
    for value in ("", "short"):
        monkeypatch.setenv("TIME_HUB_KEY", value)
        closed = _ask(111, key=value or "anything")
        assert closed.status_code == 503 and closed.json()["detail"] == {"error": "time_hub_key_not_configured"}


def test_the_bots_question_is_not_under_the_public_api_prefix():
    """Caddy forwards /api/*, /mini-app* and /health to the backend and nothing else."""
    from pathlib import Path

    assert time_gateway.internal_router.prefix == "/internal/time"
    caddyfile = (Path(__file__).resolve().parents[2] / "deploy/Caddyfile").read_text(encoding="utf-8")
    assert "/internal" not in caddyfile
