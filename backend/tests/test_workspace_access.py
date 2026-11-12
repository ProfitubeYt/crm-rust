"""Workspace access: one account, one session, the workspaces an administrator granted.

Every test goes through the real application with a real token and a registered
session; the database is an isolated in-memory SQLite and Time is a stand-in
that records what the gateway sends (see hub_support.TimeStub).
"""
from __future__ import annotations

import asyncio
from types import SimpleNamespace

import pytest
from fastapi import HTTPException

from api import dependencies
from database.models import User
from services import workspace_access as ws
from services.ai_agent.authorization import AgentAuthorizationError, current_agent_authorization
from services.user_service import build_permissions_snapshot, create_user
from hub_support import PASSWORD, app_client, bearer, error_code, install_time_stub, seed

CHECKS_READS = ["/api/transactions/years", "/api/transactions/?page=1&page_size=5"]
TIME_READ = "/api/time/month/2026-11"


@pytest.fixture
def client(db_session, monkeypatch, tmp_path):
    with app_client(db_session, monkeypatch, tmp_path) as test_client:
        test_client.time = install_time_stub(monkeypatch)
        yield test_client


def _workspace_refusal(response, workspace: str) -> bool:
    return response.status_code == 403 and response.json()["detail"] == {
        "error": "workspace_forbidden",
        "workspace": workspace,
    }


# ---------------------------------------------------------------- the four combinations


@pytest.mark.parametrize(
    "checks,time_role,listed",
    [
        (True, None, ["checks"]),
        (False, "manager", ["time"]),
        (False, "viewer", ["time"]),
        (True, "manager", ["checks", "time"]),
        (False, None, []),
        # An employee marks attendance through the bot and opens nothing on screen.
        (False, "employee", []),
        (True, "employee", ["checks"]),
    ],
)
def test_an_account_reaches_exactly_the_workspaces_it_was_granted(client, db_session, checks, time_role, listed):
    user = seed(db_session, "anna", checks=checks, time_role=time_role)
    headers = bearer(user)

    me = client.get("/api/auth/me", headers=headers)
    assert me.status_code == 200, me.text
    assert me.json()["workspaces"] == listed
    expected_roles = {"checks": "operator"} if checks else {}
    if time_role:
        expected_roles["time"] = time_role
    assert me.json()["workspace_roles"] == expected_roles
    # No workspace, in any combination, makes a CRM administrator.
    assert me.json()["global_admin"] is False

    for path in CHECKS_READS:
        response = client.get(path, headers=headers)
        if checks:
            assert response.status_code == 200, (path, response.text)
        else:
            assert _workspace_refusal(response, "checks"), (path, response.text)

    response = client.get(TIME_READ, headers=headers)
    if "time" in listed:
        assert response.status_code == 200, response.text
        assert response.json()["title"] == "Ноябрь 2026"
    else:
        assert _workspace_refusal(response, "time"), response.text
        assert client.time.requests == [], "a refused request never reaches Time"


def test_one_sign_in_serves_both_workspaces_and_says_which_ones(client, db_session):
    seed(db_session, "both", checks=True, time_role="viewer")
    login = client.post("/api/auth/login", json={"username": "both", "password": PASSWORD})
    assert login.status_code == 200, login.text
    body = login.json()
    assert body["ok"] is True
    # The Hub reads the list off the sign-in answer; nothing else is asked.
    assert body["user"]["workspaces"] == ["checks", "time"]
    assert body["user"]["workspace_roles"] == {"checks": "operator", "time": "viewer"}
    assert body["user"]["global_admin"] is False

    headers = {"Authorization": f"Bearer {body['token']}"}
    assert client.get(CHECKS_READS[0], headers=headers).status_code == 200
    assert client.get(TIME_READ, headers=headers).status_code == 200

    refreshed = client.post("/api/auth/refresh", json={"refresh_token": body["refresh_token"]})
    assert refreshed.status_code == 200, refreshed.text
    assert refreshed.json()["user"]["workspaces"] == ["checks", "time"]


def test_an_account_with_nothing_granted_still_signs_in_and_is_told_so(client, db_session):
    seed(db_session, "nobody", checks=False, time_role=None)
    login = client.post("/api/auth/login", json={"username": "nobody", "password": PASSWORD})
    assert login.status_code == 200, login.text
    assert login.json()["user"]["workspaces"] == []
    headers = {"Authorization": f"Bearer {login.json()['token']}"}
    # The account itself works: it can ask who it is and how long it is entitled.
    assert client.get("/api/auth/me", headers=headers).json()["workspaces"] == []
    entitlement = client.get("/api/auth/entitlement", headers=headers)
    assert entitlement.status_code == 200
    assert entitlement.json()["workspaces"] == []


# ---------------------------------------------------------------- hiding is not enough


_USER_DEPENDENCIES = {
    "get_current_user", "get_current_app_user", "require_admin_user", "require_global_admin",
    "get_current_user_optional", "_dep",
}
# These let anyone ask and answer the same to everyone: the user is optional
# there, and an account without Checks is treated as no user at all.
_ANSWERS_ANONYMOUS = {
    ("GET", "/api/security/status"),
    ("POST", "/api/security/unlock"),
    ("POST", "/api/security/scope/{scope_id}/request-code"),
    ("POST", "/api/security/scope/{scope_id}/verify-code"),
    ("POST", "/api/security/otp/telegram-access"),
    ("POST", "/api/security/otp/telegram-access/verify"),
}


def _dependency_names(dependant, seen=None):
    seen = seen if seen is not None else set()
    for dependency in dependant.dependencies:
        name = getattr(dependency.call, "__name__", "")
        if name not in seen:
            seen.add(name)
            _dependency_names(dependency, seen)
    return seen


def _checks_routes_that_name_a_user():
    """Every Checks route whose dependency graph resolves a user, read off the application itself."""
    import re

    from fastapi.routing import APIRoute

    from api.main import app

    found = []
    for route in app.routes:
        if not isinstance(route, APIRoute) or ws.required_workspace_for_path(route.path) != "checks":
            continue
        if not route.path.startswith("/api/") or not (_dependency_names(route.dependant) & _USER_DEPENDENCIES):
            continue
        for method in sorted(route.methods - {"HEAD", "OPTIONS"}):
            found.append((method, route.path, re.sub(r"\{[^}]+\}", "1", route.path)))
    return sorted(found)


def test_no_checks_route_serves_an_account_without_the_checks_workspace(client, db_session):
    """Hiding the workspace is not the protection: every Checks route refuses by itself.

    The list is not written by hand. It is every route of the running
    application that resolves a user and belongs to Checks, exports and the
    Excel management routes included.
    """
    # An administrator with every tab, so nothing but the workspace can refuse.
    admin = seed(db_session, "timeonly", role="admin", checks=False, time_role="manager")
    headers = bearer(admin)
    routes = _checks_routes_that_name_a_user()
    assert len(routes) > 150, "the route table was not read"

    wrong = []
    for method, template, path in routes:
        body = None if method in {"GET", "DELETE"} else {}
        response = client.request(method, path, headers=headers, json=body)
        if (method, template) in _ANSWERS_ANONYMOUS:
            # Reachable, but as nobody: the answer must not depend on the account.
            anonymous = client.request(method, path, json=body)
            if response.status_code != anonymous.status_code:
                wrong.append((method, template, response.status_code, "differs from anonymous"))
            continue
        if not _workspace_refusal(response, "checks"):
            wrong.append((method, template, response.status_code, response.text[:120]))
    assert wrong == []


def test_every_route_that_names_a_user_belongs_to_a_workspace_or_to_the_account():
    """The classification has no third answer, and a new Checks route is Checks by default."""
    from fastapi.routing import APIRoute

    from api.main import app

    seen = {None: 0, "checks": 0, "time": 0}
    for route in app.routes:
        if isinstance(route, APIRoute) and route.path.startswith("/api/"):
            seen[ws.required_workspace_for_path(route.path)] += 1
    assert seen["time"] == 1 and seen["checks"] > 150 and seen[None] > 20

    assert ws.required_workspace_for_path("/api/brand-new-checks-feature") == "checks"
    assert ws.required_workspace_for_path("/api/time") == "time"
    assert ws.required_workspace_for_path("/api/time/month/2026-11") == "time"
    # A prefix is a path segment, not a string prefix.
    assert ws.required_workspace_for_path("/api/timesheets") == "checks"
    assert ws.required_workspace_for_path("/api/authx") == "checks"
    assert ws.required_workspace_for_path("/api/admin-tools") == "checks"
    for path in ("/api/auth/me", "/api/2fa/status", "/api/admin/users", "/api/security/app/launch-status",
                 "/api/security/sessions", "/api/security/system-status", "/api/system-settings"):
        assert ws.required_workspace_for_path(path) is None, path
    # Data-facing security routes stay with Checks.
    for path in ("/api/security/scopes", "/api/security/unlock", "/api/security/audit", "/api/security/status"):
        assert ws.required_workspace_for_path(path) == "checks", path


def test_an_optional_user_without_the_workspace_is_nobody_there(db_session, monkeypatch):
    from fake_auth_redis import install_fake_auth_redis, issue_active_app_token

    install_fake_auth_redis(monkeypatch)
    user = seed(db_session, "timeonly", checks=False, time_role="viewer")
    token = issue_active_app_token(user)

    def conn(path):
        return SimpleNamespace(headers={"authorization": f"Bearer {token}"}, url=SimpleNamespace(path=path))

    # Anonymous on a Checks path, itself on an account path.
    assert asyncio.run(dependencies.get_current_user_optional(conn("/api/security/status"), db_session)) is None
    found = asyncio.run(dependencies.get_current_user_optional(conn("/api/2fa/status"), db_session))
    assert found["username"] == "timeonly"
    with pytest.raises(HTTPException) as refused:
        asyncio.run(dependencies.get_current_user(conn("/api/transactions/years"), db_session))
    assert refused.value.status_code == 403
    assert refused.value.detail == {"error": "workspace_forbidden", "workspace": "checks"}


# ---------------------------------------------------------------- changes reach a running client


def test_revoking_a_workspace_stops_the_session_that_is_already_open(client, db_session):
    admin = seed(db_session, "root", role="admin")
    user = seed(db_session, "anna", checks=True, time_role="manager")
    admin_headers, headers = bearer(admin), bearer(user)
    assert client.get(CHECKS_READS[0], headers=headers).status_code == 200
    assert client.get(TIME_READ, headers=headers).status_code == 200

    # Time goes; the same token keeps Checks and learns the new list.
    patch = client.patch(f"/api/admin/users/{user.id}", headers=admin_headers, json={"workspaces": {"time": None}})
    assert patch.status_code == 200, patch.text
    assert patch.json()["workspaces"] == ["checks"]
    assert _workspace_refusal(client.get(TIME_READ, headers=headers), "time")
    assert client.get(CHECKS_READS[0], headers=headers).status_code == 200
    assert client.get("/api/auth/me", headers=headers).json()["workspaces"] == ["checks"]

    # Checks goes too: the session is still a session, it just opens nothing.
    patch = client.patch(f"/api/admin/users/{user.id}", headers=admin_headers, json={"workspaces": {"checks": None}})
    assert patch.status_code == 200, patch.text
    assert _workspace_refusal(client.get(CHECKS_READS[0], headers=headers), "checks")
    me = client.get("/api/auth/me", headers=headers)
    assert me.status_code == 200 and me.json()["workspaces"] == []

    # Granted back, it works again without signing in: the version never moved.
    patch = client.patch(f"/api/admin/users/{user.id}", headers=admin_headers,
                         json={"workspaces": {"checks": "operator", "time": "viewer"}})
    assert patch.json()["permissions_version"] == 1
    assert client.get(CHECKS_READS[0], headers=headers).status_code == 200
    assert client.get(TIME_READ, headers=headers).status_code == 200


def test_a_disabled_account_is_refused_in_both_workspaces_and_cannot_refresh(client, db_session):
    admin = seed(db_session, "root", role="admin")
    seed(db_session, "anna", checks=True, time_role="manager")
    login = client.post("/api/auth/login", json={"username": "anna", "password": PASSWORD}).json()
    headers = {"Authorization": f"Bearer {login['token']}"}
    assert client.get(CHECKS_READS[0], headers=headers).status_code == 200

    user_id = login["user"]["id"]
    assert client.delete(f"/api/admin/users/{user_id}", headers=bearer(admin)).status_code == 200

    for path in (CHECKS_READS[0], TIME_READ, "/api/auth/me"):
        response = client.get(path, headers=headers)
        assert response.status_code in (401, 403), (path, response.text)
        assert response.status_code != 200
    refresh = client.post("/api/auth/refresh", json={"refresh_token": login["refresh_token"]})
    assert refresh.status_code in (401, 403)
    again = client.post("/api/auth/login", json={"username": "anna", "password": PASSWORD})
    assert again.status_code == 403 and again.json()["error"] == "inactive_user"


def test_a_revoked_session_is_refused_in_both_workspaces(client, db_session):
    admin = seed(db_session, "root", role="admin")
    user = seed(db_session, "anna", checks=True, time_role="manager")
    headers = bearer(user)
    other = bearer(user)
    assert client.get(TIME_READ, headers=headers).status_code == 200

    ended = client.post(f"/api/admin/users/{user.id}/revoke-sessions", headers=bearer(admin))
    assert ended.status_code == 200 and ended.json()["revoked"] == 2

    for token_headers in (headers, other):
        for path in (CHECKS_READS[0], TIME_READ, "/api/auth/me"):
            response = client.get(path, headers=token_headers)
            assert response.status_code == 401, (path, response.text)
            assert error_code(response) == "session_inactive"
    # The account is intact: a new sign-in works and nothing about its access moved.
    login = client.post("/api/auth/login", json={"username": "anna", "password": PASSWORD})
    assert login.status_code == 200 and login.json()["user"]["workspaces"] == ["checks", "time"]


# ---------------------------------------------------------------- doors that are not a session


def test_an_excel_connection_does_not_outlive_the_checks_workspace(db_session):
    from api.routes import excel_sync

    user = seed(db_session, "anna", role="admin", checks=True)
    assert excel_sync._check_user(user)["role"] == "admin"
    user.checks_access = False
    db_session.commit()
    with pytest.raises(HTTPException) as refused:
        excel_sync._check_user(user)
    assert (refused.value.status_code, refused.value.detail) == (403, "workspace_forbidden:checks")


def test_background_agent_work_stops_with_the_checks_workspace(db_session):
    user = seed(db_session, "anna", role="admin", checks=True, time_role="manager")
    assert current_agent_authorization(db_session, user.id).is_admin
    user.checks_access = False
    db_session.commit()
    with pytest.raises(AgentAuthorizationError, match="workspace_forbidden:checks"):
        current_agent_authorization(db_session, user.id)


# ---------------------------------------------------------------- nothing changes for who was already here


def test_an_account_created_without_a_word_about_workspaces_is_a_checks_account(db_session):
    legacy = User(username="old", password_hash="x", salt="y", role="operator")
    db_session.add(legacy)
    db_session.commit()
    db_session.refresh(legacy)
    assert legacy.checks_access is True and legacy.time_role is None
    # And not a CRM administrator, whatever inserted it.
    assert legacy.global_admin is False

    created = create_user(db_session, username="fromservice", password=PASSWORD)
    snapshot = build_permissions_snapshot(created)
    assert snapshot["workspaces"] == ["checks"]
    assert snapshot["workspace_roles"] == {"checks": "operator"}
    assert snapshot["global_admin"] is False

    # The same for an administrator made the old way, by the role alone: the
    # Checks role is what was asked for, and nothing more came with it.
    old_way = create_user(db_session, username="oldadmin", password=PASSWORD, role="admin")
    snapshot = build_permissions_snapshot(old_way)
    assert snapshot["workspace_roles"] == {"checks": "admin"}
    assert snapshot["global_admin"] is False


def test_a_snapshot_from_before_workspaces_reads_as_checks_only():
    assert ws.snapshot_workspaces({"role": "admin"}) == frozenset({"checks"})
    assert ws.snapshot_workspaces({"workspaces": []}) == frozenset()
    assert ws.snapshot_workspaces({"workspaces": ["time"]}) == frozenset({"time"})
    assert ws.snapshot_workspaces(None) == frozenset()
    assert ws.snapshot_workspaces({"workspaces": "checks"}) == frozenset()


def test_time_roles_are_ordered_and_unknown_ones_grant_nothing():
    assert ws.time_role_allows("manager", "viewer") and ws.time_role_allows("viewer", "viewer")
    assert not ws.time_role_allows("viewer", "manager")
    assert not ws.time_role_allows("employee", "viewer")
    assert not ws.time_role_allows(None, "viewer") and not ws.time_role_allows("owner", "viewer")
    assert ws.normalize_workspace_role("time", "") is None and ws.normalize_workspace_role("time", "None") is None
    assert ws.normalize_workspace_role("time", " Manager ") == "manager"
    with pytest.raises(ws.WorkspaceError, match="invalid_workspace_role"):
        ws.normalize_workspace_role("time", "owner")
    # A role of one workspace is not a role of another.
    with pytest.raises(ws.WorkspaceError, match="invalid_workspace_role"):
        ws.normalize_workspace_role("checks", "manager")
    with pytest.raises(ws.WorkspaceError, match="unknown_workspace"):
        ws.normalize_workspace_role("payroll", "manager")
    # A value the code does not know, should one ever be in the column, is no access.
    assert ws.time_role_of(SimpleNamespace(time_role="owner")) is None
    assert ws.workspaces_of(SimpleNamespace(checks_access=False, time_role="owner")) == []
