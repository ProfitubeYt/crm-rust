"""Shared pieces of the workspace, admin and Time gateway tests.

The real application, an isolated in-memory SQLite session, the fake session
store the other auth tests use, and a system-access token as Caddy supplies it.
Nothing here talks to a network service.
"""
from __future__ import annotations

import asyncio
import json
from contextlib import asynccontextmanager, contextmanager
from typing import Any, Dict, Iterable, Optional

import httpx
from fastapi.testclient import TestClient

from api.main import app
from database.connection import get_db_session
from database.models import AccessAuditLog, User
from services.access_control_service import hash_password
from services.root_access_config_service import hash_password_pbkdf2, reset_root_access_cache
from fake_auth_redis import install_fake_auth_redis, issue_active_app_token

SYSTEM_TOKEN = "test-system-token"
PASSWORD = "Strong!123"
ADMIN_TABS = '["dashboard","reference","automation","userbot","logs","admin"]'


@contextmanager
def app_client(db_session, monkeypatch, tmp_path):
    """The application as a first-party client reaches it: system token on every call."""

    @asynccontextmanager
    async def noop_lifespan(_app):
        yield

    token_hash, token_salt = hash_password_pbkdf2(SYSTEM_TOKEN, "testsalt")
    config_path = tmp_path / "root-access.server.json"
    config_path.write_text(
        json.dumps(
            {
                "version": 1,
                "system_access": {
                    "enforced": True,
                    "tokens": [
                        {"id": "tests", "kind": "pbkdf2_sha256", "hash": token_hash, "salt": token_salt, "active": True}
                    ],
                },
                "scopes": [],
            }
        ),
        encoding="utf-8",
    )
    monkeypatch.setenv("SYSTEM_ACCESS_ENFORCED", "true")
    monkeypatch.setenv("SCOPES_MANAGED_BY_CONFIG", "false")
    monkeypatch.setenv("ROOT_ACCESS_SERVER_CONFIG_PATH", str(config_path))
    monkeypatch.setenv("LAUNCH_GATE_ENABLED", "false")
    fake_redis, _ = install_fake_auth_redis(monkeypatch)
    reset_root_access_cache()

    original_lifespan = app.router.lifespan_context
    app.router.lifespan_context = noop_lifespan

    def override_db():
        yield db_session

    app.dependency_overrides[get_db_session] = override_db
    try:
        with TestClient(app) as client:
            client.headers.update({"X-System-Access": SYSTEM_TOKEN})
            client.fake_redis = fake_redis
            yield client
    finally:
        app.dependency_overrides.pop(get_db_session, None)
        app.router.lifespan_context = original_lifespan
        reset_root_access_cache()


def seed(
    db_session,
    username: str,
    *,
    role: str = "operator",
    checks: bool = True,
    time_role: Optional[str] = None,
    global_admin: Optional[bool] = None,
    tabs: Optional[Iterable[str]] = None,
    telegram_id: Optional[int] = None,
    active: bool = True,
    password: str = PASSWORD,
) -> User:
    """An account as it sits in the database.

    ``role`` is the Checks role. Left unsaid, ``global_admin`` is what the
    migration makes of an account from before the split: a Checks
    administrator was the one who administered accounts, so it still is.
    Tests about the split say it by name.
    """
    password_hash, salt = hash_password(password)
    if tabs is None:
        allowed_tabs = ADMIN_TABS if role == "admin" else '["dashboard"]'
    else:
        allowed_tabs = json.dumps(list(tabs))
    user = User(
        username=username,
        password_hash=password_hash,
        salt=salt,
        role=role,
        display_name=username.title(),
        allowed_tabs=allowed_tabs,
        allowed_folders="[]",
        forbidden_periods="[]",
        allowed_sources="[]",
        can_toggle_sources=(role == "admin"),
        permissions_version=1,
        is_active=active,
        checks_access=checks,
        time_role=time_role,
        global_admin=(role == "admin") if global_admin is None else global_admin,
        telegram_id=telegram_id,
    )
    db_session.add(user)
    db_session.commit()
    db_session.refresh(user)
    return user


def bearer(user: User) -> Dict[str, str]:
    """Headers of a signed-in client: a real token with a registered session."""
    return {"Authorization": f"Bearer {issue_active_app_token(user)}"}


def error_code(response) -> Any:
    """The machine-readable part of an error answer, whatever shape it took."""
    try:
        body = response.json()
    except ValueError:
        return None
    detail = body.get("detail", body)
    if isinstance(detail, dict):
        return detail.get("error") or detail
    return detail


def audit_rows(db_session, action: Optional[str] = None):
    query = db_session.query(AccessAuditLog)
    if action:
        query = query.filter(AccessAuditLog.action == action)
    rows = query.order_by(AccessAuditLog.id.asc()).all()
    return [(row.action, json.loads(row.details_json) if row.details_json else {}, row) for row in rows]


def all_audit_text(db_session) -> str:
    return "\n".join(
        f"{row.action} {row.details_json or ''}" for row in db_session.query(AccessAuditLog).all()
    )


def call_from(host: str, method: str, path: str, *, headers: Optional[Dict[str, str]] = None) -> httpx.Response:
    """One request to the application as if it came from `host`.

    The ordinary test client always calls from the name "testclient"; whether a
    caller sits on the internal network is exactly what some tests are about.
    """

    async def run() -> httpx.Response:
        transport = httpx.ASGITransport(app=app, client=(host, 40000))
        async with httpx.AsyncClient(transport=transport, base_url="http://backend:8000") as client:
            return await client.request(method, path, headers=headers or {})

    return asyncio.run(run())


TIME_TOKEN = "stub-time-token-0123456789abcdef"
TIME_URL = "http://timelist.internal:8080"


class TimeStub:
    """Stands in for the Time server behind the gateway. It is not Time: it
    records what the gateway sent and answers with fixed data."""

    def __init__(self) -> None:
        self.requests: list[httpx.Request] = []
        self.down = False
        self.status: Optional[int] = None

    def handler(self, request: httpx.Request) -> httpx.Response:
        if self.down:
            raise httpx.ConnectError("connection refused", request=request)
        self.requests.append(request)
        if request.headers.get("authorization") != f"Bearer {TIME_TOKEN}":
            return httpx.Response(401, json={"error": "требуется вход"})
        if self.status is not None:
            return httpx.Response(self.status, json={"error": "отказ"})
        path = request.url.path
        if path == "/api/meta":
            return httpx.Response(200, json={"version": "0.1.0", "authed": True, "revision": 7, "bot": {"enabled": True}})
        if path.startswith("/api/export/"):
            return httpx.Response(
                200,
                content=b"PK\x03\x04 workbook",
                headers={
                    "content-type": "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
                    "content-disposition": 'attachment; filename="TimeList-2026-11.xlsx"',
                    "set-cookie": "tl_session=must-not-leak",
                },
            )
        if path.startswith("/api/month/"):
            return httpx.Response(200, json={"title": "Ноябрь 2026", "employees": []})
        return httpx.Response(200, json={"ok": True, "method": request.method, "path": path})

    @property
    def last(self) -> httpx.Request:
        return self.requests[-1]


def install_time_stub(monkeypatch) -> TimeStub:
    from api.routes import time_gateway

    stub = TimeStub()
    client = httpx.AsyncClient(transport=httpx.MockTransport(stub.handler))
    monkeypatch.setenv("TIMELIST_INTERNAL_URL", TIME_URL)
    monkeypatch.setenv("TIMELIST_API_TOKEN", TIME_TOKEN)
    monkeypatch.setattr(time_gateway, "_get_client", lambda: client)
    return stub
