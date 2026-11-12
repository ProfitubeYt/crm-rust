"""CRM administration: who may do it, what it cannot break, what it leaves behind.

The account directory and the grants belong to the CRM as a whole. The tests
here hold the line between the three things decided about an account: whether
it administers the CRM, what it may open, and what it may do inside.

Real application, real tokens with registered sessions, isolated SQLite.
"""
from __future__ import annotations

import asyncio
import importlib.util
import json
from pathlib import Path
from types import SimpleNamespace

import pytest
from fastapi import HTTPException

from api import dependencies
from database.models import AccessAuditLog, AccessScope, User
from services import workspace_access as ws
from hub_support import PASSWORD, all_audit_text, app_client, audit_rows, bearer, error_code, seed

NEW_PASSWORD = "Fresh#Pass9"
ALL_OPERATOR_TABS = ["dashboard", "reference", "automation", "userbot", "logs"]


@pytest.fixture
def client(db_session, monkeypatch, tmp_path):
    with app_client(db_session, monkeypatch, tmp_path) as test_client:
        yield test_client


# ---------------------------------------------------------------- who may administer

_ADMINISTRATION_PREFIXES = ("/api/admin", "/api/security/sessions", "/api/system-settings")


def _administration_routes():
    """Every CRM administration route of the running application, with a usable path."""
    import re

    from fastapi.routing import APIRoute

    from api.main import app

    routes = []
    for route in app.routes:
        if isinstance(route, APIRoute) and route.path.startswith(_ADMINISTRATION_PREFIXES):
            for method in sorted(route.methods - {"HEAD", "OPTIONS"}):
                routes.append((method, re.sub(r"\{[^}]+\}", "1", route.path)))
    return sorted(routes)


@pytest.mark.parametrize(
    "who",
    [
        # Every right an operator can hold, in both workspaces.
        {"role": "operator", "time_role": "manager", "tabs": ALL_OPERATOR_TABS},
        # The most senior role of each workspace at once.
        {"role": "admin", "time_role": "manager", "global_admin": False},
        # Time alone, as its manager.
        {"role": "operator", "checks": False, "time_role": "manager"},
    ],
    ids=["operator-everything", "checks-admin-and-time-manager", "time-manager-only"],
)
def test_no_workspace_role_opens_crm_administration(client, db_session, who):
    seed(db_session, "root", role="admin")
    account = seed(db_session, "member", **who)
    headers = bearer(account)
    routes = _administration_routes()
    assert len(routes) >= 19, "the route table was not read"

    for method, path in routes:
        body = None if method in {"GET", "DELETE"} else {}
        signed_in = client.request(method, path, headers=headers, json=body)
        assert signed_in.status_code == 403, (method, path, signed_in.text)
        assert error_code(signed_in) == "global_admin_required", (method, path, signed_in.text)
        anonymous = client.request(method, path, json=body)
        assert anonymous.status_code == 401, (method, path, anonymous.text)

    assert db_session.query(User).count() == 2
    db_session.refresh(account)
    assert account.global_admin is False and account.is_active is True
    assert client.get("/api/auth/me", headers=headers).json()["global_admin"] is False


def test_the_checks_administrator_keeps_checks_and_gets_nothing_of_the_crm(client, db_session):
    folder = AccessScope(name="2025", password_hash="unused", salt="unused", years_json="[2025]")
    db_session.add(folder)
    db_session.commit()
    checks_admin = seed(db_session, "chief", role="admin", global_admin=False)
    headers = bearer(checks_admin)

    # Everything the role always did inside Checks is still there.
    assert client.get("/api/security/scopes", headers=headers).status_code == 200
    assert client.get("/api/audit", headers=headers).status_code == 200
    assert client.get("/api/transactions/years", headers=headers).status_code == 200
    me = client.get("/api/auth/me", headers=headers).json()
    assert me["role"] == "admin" and me["workspace_roles"] == {"checks": "admin"}

    # Accounts, everybody's sessions and the system-wide security switches are not Checks.
    for method, path in (("GET", "/api/admin/users"), ("GET", "/api/admin/meta"), ("GET", "/api/admin/audit"),
                         ("GET", "/api/security/sessions"), ("GET", "/api/system-settings")):
        refused = client.request(method, path, headers=headers)
        assert refused.status_code == 403 and error_code(refused) == "global_admin_required", (path, refused.text)
    switched = client.patch("/api/system-settings", headers=headers, json={"launch_gate_enabled": True})
    assert switched.status_code == 403 and error_code(switched) == "global_admin_required"

    # The Checks journal stays readable, without the account administration in it.
    root = seed(db_session, "root", role="admin")
    client.post("/api/admin/users", headers=bearer(root), json={"username": "newbie", "password": NEW_PASSWORD})
    db_session.add(AccessAuditLog(action="scope_unlock_ok", success=True, details_json="{}"))
    db_session.commit()
    for path in ("/api/audit", "/api/security/audit"):
        as_checks_admin = {item["action"] for item in client.get(path, headers=headers).json()}
        assert "scope_unlock_ok" in as_checks_admin, path
        assert not any(action.startswith(("admin_", "authbot_user_")) for action in as_checks_admin), path
        as_crm_admin = {item["action"] for item in client.get(path, headers=bearer(root)).json()}
        assert {"admin_user_create", "scope_unlock_ok"} <= as_crm_admin, path


def test_a_crm_administrator_needs_no_workspace_and_gets_none(client, db_session):
    admin = seed(db_session, "owner", role="operator", checks=False, time_role=None, global_admin=True)
    headers = bearer(admin)

    me = client.get("/api/auth/me", headers=headers).json()
    assert me["global_admin"] is True and me["workspaces"] == [] and me["workspace_roles"] == {}

    for path in ("/api/admin/users", "/api/admin/meta", "/api/admin/audit", "/api/security/sessions",
                 "/api/system-settings"):
        assert client.get(path, headers=headers).status_code == 200, path

    # Administering the CRM opens no workspace: like anyone else, only once granted.
    for path, workspace in (("/api/transactions/years", "checks"), ("/api/security/scopes", "checks"),
                            ("/api/time/month/2026-11", "time")):
        refused = client.get(path, headers=headers)
        assert refused.status_code == 403, (path, refused.text)
        assert refused.json()["detail"] == {"error": "workspace_forbidden", "workspace": workspace}

    # Granting oneself a workspace is an administrator's right, and it is on record.
    granted = client.patch(f"/api/admin/users/{admin.id}", headers=headers, json={"workspaces": {"checks": "operator"}})
    assert granted.status_code == 200, granted.text
    assert client.get("/api/transactions/years", headers=headers).status_code == 200
    change = audit_rows(db_session, "admin_user_access_change")[-1][1]
    assert change["before"]["workspace_roles"] == {} and change["after"]["workspace_roles"] == {"checks": "operator"}
    # An operator in Checks all the same: the administrator's right did not become a Checks role.
    assert client.get("/api/security/scopes", headers=headers).status_code == 403


def test_crm_administration_is_given_and_taken_by_name_and_reaches_a_running_client(client, db_session):
    admin = seed(db_session, "root", role="admin")
    user = seed(db_session, "anna", checks=True, time_role="viewer")
    admin_headers, headers = bearer(admin), bearer(user)
    assert client.get("/api/admin/users", headers=headers).status_code == 403

    given = client.patch(f"/api/admin/users/{user.id}", headers=admin_headers, json={"global_admin": True})
    assert given.status_code == 200, given.text
    body = given.json()
    assert body["global_admin"] is True
    # Nothing else about the account moved, and nobody was signed out for it.
    assert (body["role"], body["workspace_roles"], body["permissions_version"]) == (
        "operator", {"checks": "operator", "time": "viewer"}, 1,
    )
    assert body["access_summary"][0] == "Администратор CRM: учётные записи и доступ"
    # The token issued before the change administers now...
    assert client.get("/api/admin/users", headers=headers).status_code == 200
    assert client.get("/api/auth/me", headers=headers).json()["global_admin"] is True
    # ...and is still an operator inside Checks.
    assert client.get("/api/security/scopes", headers=headers).status_code == 403

    taken = client.patch(f"/api/admin/users/{user.id}", headers=admin_headers, json={"global_admin": False})
    assert taken.status_code == 200 and taken.json()["global_admin"] is False
    refused = client.get("/api/admin/users", headers=headers)
    assert refused.status_code == 403 and error_code(refused) == "global_admin_required"
    assert client.get("/api/transactions/years", headers=headers).status_code == 200

    changes = [details for _, details, _ in audit_rows(db_session, "admin_user_access_change")]
    assert [(item["before"]["global_admin"], item["after"]["global_admin"]) for item in changes] == [
        (False, True), (True, False),
    ]


def test_an_account_cannot_grant_itself_anything(client, db_session):
    seed(db_session, "root", role="admin")
    user = seed(db_session, "anna", checks=True, time_role=None)
    headers = bearer(user)
    for change in ({"workspaces": {"time": "manager"}}, {"role": "admin"}, {"global_admin": True},
                   {"workspaces": {"checks": "admin"}}, {"allowed_tabs": ["logs"]}):
        response = client.patch(f"/api/admin/users/{user.id}", headers=headers, json=change)
        assert response.status_code == 403, response.text
    db_session.refresh(user)
    assert (user.role, user.time_role, user.global_admin) == ("operator", None, False)


# ---------------------------------------------------------------- creating and granting


@pytest.mark.parametrize(
    "extra,workspaces,roles,summary",
    [
        ({}, ["checks"], {"checks": "operator"}, ["Чеки: оператор"]),
        ({"workspaces": {"time": "manager"}}, ["time"], {"time": "manager"}, ["Учёт времени: руководитель"]),
        ({"workspaces": {"checks": "operator", "time": "viewer"}}, ["checks", "time"],
         {"checks": "operator", "time": "viewer"}, ["Чеки: оператор", "Учёт времени: просмотр"]),
        ({"workspaces": {}}, [], {}, ["Нет доступа к рабочим пространствам"]),
        # An employee marks attendance through the bot and opens nothing on screen.
        ({"workspaces": {"time": "employee"}}, [], {"time": "employee"}, ["Учёт времени: сотрудник"]),
        ({"workspaces": {"checks": "admin"}}, ["checks"], {"checks": "admin"}, ["Чеки: администратор Чеков"]),
        ({"global_admin": True, "workspaces": {}}, [], {},
         ["Администратор CRM: учётные записи и доступ", "Нет доступа к рабочим пространствам"]),
    ],
)
def test_an_account_is_created_with_exactly_the_access_asked_for(
    client, db_session, extra, workspaces, roles, summary
):
    admin = seed(db_session, "root", role="admin")
    response = client.post(
        "/api/admin/users", headers=bearer(admin),
        json={"username": "newbie", "password": NEW_PASSWORD, "display_name": "Новый", **extra},
    )
    assert response.status_code == 200, response.text
    body = response.json()
    assert body["workspaces"] == workspaces
    assert body["workspace_roles"] == roles
    assert body["access_summary"] == summary
    assert body["global_admin"] is extra.get("global_admin", False)

    effective = body["effective_access"]
    assert effective["state"] == "active" and effective["can_sign_in"] is True
    assert {item["id"]: item["role"] for item in effective["workspaces"] if item["granted"]} == roles
    assert [item["id"] for item in effective["workspaces"] if item["in_hub"]] == workspaces

    created = audit_rows(db_session, "admin_user_create")[-1][1]
    assert created["access"] == {
        "is_active": True, "global_admin": extra.get("global_admin", False), "workspace_roles": roles,
    }

    # The account signs in with that password and the Hub is told the same.
    login = client.post("/api/auth/login", json={"username": "newbie", "password": NEW_PASSWORD})
    assert login.status_code == 200, login.text
    assert login.json()["user"]["workspaces"] == workspaces
    assert login.json()["user"]["global_admin"] is extra.get("global_admin", False)


def test_a_client_from_before_workspaces_still_creates_and_edits_checks_accounts(client, db_session):
    admin = seed(db_session, "root", role="admin")
    headers = bearer(admin)
    # Exactly what desktop 2.0.2 sends: no workspace field anywhere.
    created = client.post(
        "/api/admin/users", headers=headers,
        json={"username": "operator1", "password": NEW_PASSWORD, "role": "operator", "allowed_tabs": ["dashboard"],
              "allowed_folders": [1], "can_toggle_sources": False, "force_2fa": False, "session_ttl_days": 7},
    )
    assert created.status_code == 200, created.text
    user_id = created.json()["id"]
    assert created.json()["workspaces"] == ["checks"]

    client.patch(f"/api/admin/users/{user_id}", headers=headers, json={"workspaces": {"time": "manager"}})
    edited = client.patch(f"/api/admin/users/{user_id}", headers=headers,
                          json={"display_name": "Оператор", "allowed_tabs": ["dashboard", "reference"]})
    assert edited.status_code == 200, edited.text
    # An edit that does not mention workspaces leaves them as they were.
    assert edited.json()["workspaces"] == ["checks", "time"]
    assert edited.json()["workspace_roles"] == {"checks": "operator", "time": "manager"}

    # Its "administrator" is the Checks role and nothing more: such a client
    # never appoints a CRM administrator, because it cannot say that it means to.
    chief = client.post("/api/admin/users", headers=headers,
                        json={"username": "chief", "password": NEW_PASSWORD, "role": "admin"}).json()
    assert (chief["role"], chief["global_admin"], chief["workspace_roles"]) == ("admin", False, {"checks": "admin"})
    promoted = client.patch(f"/api/admin/users/{user_id}", headers=headers, json={"role": "admin"}).json()
    assert (promoted["role"], promoted["global_admin"]) == ("admin", False)


def test_a_role_is_one_the_workspace_has_or_nothing(client, db_session):
    admin = seed(db_session, "root", role="admin")
    user = seed(db_session, "anna")
    headers = bearer(admin)

    def grant(workspaces):
        return client.patch(f"/api/admin/users/{user.id}", headers=headers, json={"workspaces": workspaces})

    for bad in ({"time": "owner"}, {"time": "admin"}, {"time": "viewer,manager"}, {"checks": "manager"}):
        response = grant(bad)
        assert response.status_code == 400 and error_code(response) == "invalid_workspace_role", (bad, response.text)
    unknown = grant({"payroll": "manager"})
    assert unknown.status_code == 400 and error_code(unknown) == "unknown_workspace"
    assert grant(["time"]).status_code == 422
    db_session.refresh(user)
    assert (user.time_role, user.role, user.checks_access) == (None, "operator", True)

    for role in ("employee", "viewer", "manager"):
        assert grant({"time": role}).json()["workspace_roles"]["time"] == role
    for clear in (None, "", "none"):
        grant({"time": "viewer"})
        assert "time" not in grant({"time": clear}).json()["workspace_roles"]

    bad_create = client.post("/api/admin/users", headers=headers,
                             json={"username": "x", "password": NEW_PASSWORD, "workspaces": {"time": "owner"}})
    assert bad_create.status_code == 400 and error_code(bad_create) == "invalid_workspace_role"
    assert db_session.query(User).filter(User.username == "x").count() == 0


def test_checks_restrictions_are_untouched_by_grants(client, db_session):
    admin = seed(db_session, "root", role="admin")
    folder = AccessScope(name="2025", password_hash="unused", salt="unused", years_json="[2025]")
    db_session.add(folder)
    db_session.commit()
    headers = bearer(admin)
    created = client.post(
        "/api/admin/users", headers=headers,
        json={"username": "scoped", "password": NEW_PASSWORD, "allowed_tabs": ["dashboard", "userbot"],
              "allowed_folders": [folder.id], "allowed_sources": [777],
              "forbidden_periods": [{"from": "2025-01-01", "to": "2025-01-31"}]},
    ).json()
    restrictions = {key: created[key] for key in ("role", "allowed_tabs", "allowed_folders", "allowed_sources",
                                                  "forbidden_periods", "can_toggle_sources", "permissions_version")}

    for change in ({"workspaces": {"time": "manager"}}, {"workspaces": {"checks": None}},
                   {"workspaces": {"checks": "operator"}}, {"workspaces": {"time": None}},
                   {"global_admin": True}, {"global_admin": False}):
        after = client.patch(f"/api/admin/users/{created['id']}", headers=headers, json=change).json()
        assert {key: after[key] for key in restrictions} == restrictions, change

    # The Checks role is the one grant that is a Checks permission itself.
    chief = client.patch(f"/api/admin/users/{created['id']}", headers=headers,
                         json={"workspaces": {"checks": "admin"}}).json()
    assert chief["role"] == "admin" and chief["permissions_version"] == restrictions["permissions_version"] + 1
    assert chief["global_admin"] is False


# ---------------------------------------------------------------- nobody is locked out by accident


def test_the_last_crm_administrator_cannot_be_disabled_or_stripped(client, db_session):
    admin = seed(db_session, "root", role="admin")
    second = seed(db_session, "second", role="admin")
    headers = bearer(admin)

    # Not oneself, whoever else is there.
    assert error_code(client.delete(f"/api/admin/users/{admin.id}", headers=headers)) == "cannot_disable_self"
    assert error_code(client.patch(f"/api/admin/users/{admin.id}", headers=headers, json={"is_active": False})) == "cannot_disable_self"
    assert error_code(client.patch(f"/api/admin/users/{admin.id}", headers=headers, json={"global_admin": False})) == "cannot_demote_self"
    # The way a client from before the split says the same thing.
    assert error_code(client.patch(f"/api/admin/users/{admin.id}", headers=headers, json={"role": "operator"})) == "cannot_demote_self"

    # Another administrator can be removed while one stays...
    stripped = client.patch(f"/api/admin/users/{second.id}", headers=headers, json={"global_admin": False})
    assert stripped.status_code == 200 and stripped.json()["global_admin"] is False
    # ...and keeps the Checks role: that was not what was taken.
    assert stripped.json()["role"] == "admin"
    # The one who stays cannot be removed by any path, the service included.
    from services.user_service import UserServiceError, deactivate_user, update_user

    db_session.refresh(admin)
    for attempt in (lambda: deactivate_user(db_session, admin),
                    lambda: update_user(db_session, admin, is_active=False),
                    lambda: update_user(db_session, admin, global_admin=False)):
        with pytest.raises(UserServiceError, match="last_admin"):
            attempt()
    db_session.refresh(admin)
    assert admin.global_admin is True and admin.is_active is True

    # With two administrators again, either may remove the other, never both.
    client.patch(f"/api/admin/users/{second.id}", headers=headers, json={"global_admin": True})
    second_headers = bearer(db_session.get(User, second.id))
    removed = client.delete(f"/api/admin/users/{admin.id}", headers=second_headers)
    assert removed.status_code == 200 and removed.json()["is_active"] is False
    last = client.patch(f"/api/admin/users/{second.id}", headers=second_headers, json={"is_active": False})
    assert last.status_code == 409 and error_code(last) == "cannot_disable_self"
    assert db_session.query(User).filter(User.global_admin.is_(True), User.is_active.is_(True)).count() == 1


def test_a_demotion_by_an_older_client_still_takes_the_administrators_rights(client, db_session):
    """Before the split "admin" was one thing, and demoting someone took all of it.

    A client that knows only the role must not leave a demoted person with the
    right to administer accounts that it has no way to show or to take away.
    """
    admin = seed(db_session, "root", role="admin")
    second = seed(db_session, "second", role="admin")
    headers = bearer(admin)

    demoted = client.patch(f"/api/admin/users/{second.id}", headers=headers, json={"role": "operator"})
    assert demoted.status_code == 200, demoted.text
    assert (demoted.json()["role"], demoted.json()["global_admin"]) == ("operator", False)
    change = audit_rows(db_session, "admin_user_access_change")[-1][1]
    assert change["before"]["global_admin"] is True and change["after"]["global_admin"] is False

    # Only the change does it. An administrator who is an operator in Checks on
    # purpose is not stripped by a form that sends the same role back...
    owner = seed(db_session, "owner", role="operator", global_admin=True)
    resent = client.patch(f"/api/admin/users/{owner.id}", headers=headers,
                          json={"role": "operator", "display_name": "Владелец"})
    assert resent.status_code == 200 and resent.json()["global_admin"] is True
    # ...and the panel, which says both things by name, changes exactly what it names.
    third = seed(db_session, "third", role="admin")
    named = client.patch(f"/api/admin/users/{third.id}", headers=headers,
                         json={"workspaces": {"checks": "operator"}, "global_admin": True})
    assert (named.json()["role"], named.json()["global_admin"]) == ("operator", True)
    # The last one is still refused, whichever way it is said.
    client.patch(f"/api/admin/users/{owner.id}", headers=headers, json={"global_admin": False})
    client.patch(f"/api/admin/users/{third.id}", headers=headers, json={"global_admin": False})
    last = client.patch(f"/api/admin/users/{admin.id}", headers=bearer(admin), json={"role": "operator"})
    assert last.status_code == 409


def test_an_administrator_changes_their_own_checks_role_without_losing_the_crm(client, db_session):
    admin = seed(db_session, "root", role="admin")
    changed = client.patch(f"/api/admin/users/{admin.id}", headers=bearer(admin),
                           json={"workspaces": {"checks": "operator"}})
    assert changed.status_code == 200, changed.text
    assert (changed.json()["role"], changed.json()["global_admin"]) == ("operator", True)
    # The role moved the permissions version, so the client signs in again...
    login = client.post("/api/auth/login", json={"username": "root", "password": PASSWORD}).json()
    headers = {"Authorization": f"Bearer {login['token']}"}
    # ...as an operator in Checks who still administers the CRM.
    assert client.get("/api/security/scopes", headers=headers).status_code == 403
    assert client.get("/api/admin/users", headers=headers).status_code == 200


def test_an_account_is_disabled_and_enabled_again_without_losing_its_access(client, db_session):
    admin = seed(db_session, "root", role="admin")
    user = seed(db_session, "anna", checks=True, time_role="viewer")
    headers = bearer(admin)
    user_headers = bearer(user)

    disabled = client.patch(f"/api/admin/users/{user.id}", headers=headers, json={"is_active": False})
    assert disabled.status_code == 200 and disabled.json()["is_active"] is False
    # What was granted is remembered; what it comes to is nothing.
    assert disabled.json()["workspace_roles"] == {"checks": "operator", "time": "viewer"}
    effective = disabled.json()["effective_access"]
    assert effective["state"] == "disabled" and effective["can_sign_in"] is False
    assert not any(item["effective"] or item["in_hub"] for item in effective["workspaces"])
    assert disabled.json()["access_summary"][0] == "Учётная запись отключена — войти нельзя"
    assert client.get("/api/auth/me", headers=user_headers).status_code in (401, 403)
    assert client.get(f"/api/admin/users/{user.id}/sessions", headers=headers).json()["sessions"] == []

    enabled = client.patch(f"/api/admin/users/{user.id}", headers=headers, json={"is_active": True})
    assert enabled.json()["is_active"] is True and enabled.json()["workspaces"] == ["checks", "time"]
    assert enabled.json()["effective_access"]["state"] == "active"
    # The old token stays dead; the account signs in again with its own password.
    assert client.get("/api/auth/me", headers=user_headers).status_code == 401
    assert client.post("/api/auth/login", json={"username": "anna", "password": PASSWORD}).status_code == 200


def test_effective_access_follows_the_expiry_date_and_spares_administrators(client, db_session):
    admin = seed(db_session, "root", role="operator", checks=False, global_admin=True)
    user = seed(db_session, "anna", checks=True, time_role="viewer")
    headers = bearer(admin)

    expired = client.patch(f"/api/admin/users/{user.id}", headers=headers, json={"access_until": "2020-01-01"})
    effective = expired.json()["effective_access"]
    assert effective["state"] == "expired" and effective["can_sign_in"] is False
    assert [item["effective"] for item in effective["workspaces"]] == [False, False]
    assert expired.json()["access_summary"][0] == "Срок доступа истёк — войти нельзя"

    # Someone has to be able to extend everyone else's: an administrator's own date does not bind.
    own = client.patch(f"/api/admin/users/{admin.id}", headers=headers, json={"access_until": "2020-01-01"})
    assert own.json()["effective_access"]["state"] == "active"
    # A date moves the permissions version, so the client signs in again, and is let in.
    login = client.post("/api/auth/login", json={"username": "root", "password": PASSWORD}).json()
    headers = {"Authorization": f"Bearer {login['token']}"}
    assert client.get("/api/admin/users", headers=headers).status_code == 200

    cleared = client.patch(f"/api/admin/users/{user.id}", headers=headers, json={"access_until": ""})
    assert cleared.json()["effective_access"]["state"] == "active"


# ---------------------------------------------------------------- passwords and sessions


def test_a_password_is_reset_never_read(client, db_session):
    admin = seed(db_session, "root", role="admin")
    user = seed(db_session, "anna")
    headers = bearer(admin)
    login = client.post("/api/auth/login", json={"username": "anna", "password": PASSWORD}).json()
    old_headers = {"Authorization": f"Bearer {login['token']}"}

    weak = client.post(f"/api/admin/users/{user.id}/reset-password", headers=headers, json={"password": "short"})
    assert weak.status_code == 400 and error_code(weak) == "weak_password"

    reset = client.post(f"/api/admin/users/{user.id}/reset-password", headers=headers, json={"password": NEW_PASSWORD})
    assert reset.status_code == 200, reset.text

    # The old password and every session opened with it are gone.
    assert client.post("/api/auth/login", json={"username": "anna", "password": PASSWORD}).status_code == 401
    assert client.get("/api/auth/me", headers=old_headers).status_code == 401
    assert client.post("/api/auth/refresh", json={"refresh_token": login["refresh_token"]}).status_code == 401
    assert client.post("/api/auth/login", json={"username": "anna", "password": NEW_PASSWORD}).status_code == 200

    # Nothing the administration returns or records holds a password or its hash.
    db_session.refresh(user)
    listed = client.get("/api/admin/users", headers=headers)
    audit = client.get("/api/admin/audit", headers=headers)
    for text in (reset.text, listed.text, audit.text, all_audit_text(db_session)):
        for secret in (NEW_PASSWORD, PASSWORD, user.password_hash, "password_hash", "totp_secret", "backup_codes"):
            assert secret not in text
    assert not any("password" in key or "secret" in key or key == "salt" for key in listed.json()[0])


def test_sessions_of_one_account_are_listed_and_ended_without_touching_others(client, db_session):
    admin = seed(db_session, "root", role="admin")
    anna = seed(db_session, "anna", checks=True, time_role="manager")
    boris = seed(db_session, "boris")
    admin_headers = bearer(admin)
    anna_one, anna_two, boris_headers = bearer(anna), bearer(anna), bearer(boris)

    listed = client.get(f"/api/admin/users/{anna.id}/sessions", headers=admin_headers)
    assert listed.status_code == 200
    sessions = listed.json()["sessions"]
    assert len(sessions) == 2 and {item["user_id"] for item in sessions} == {anna.id}
    assert all(item["current"] is False for item in sessions)

    ended = client.post(f"/api/admin/users/{anna.id}/revoke-sessions", headers=admin_headers)
    assert ended.json() == {"ok": True, "user_id": anna.id, "revoked": 2, "kept_current": False}
    for headers in (anna_one, anna_two):
        refused = client.get("/api/auth/me", headers=headers)
        assert refused.status_code == 401 and error_code(refused) == "session_inactive"
    assert client.get("/api/auth/me", headers=boris_headers).status_code == 200
    assert client.get("/api/auth/me", headers=admin_headers).status_code == 200
    assert client.get(f"/api/admin/users/{anna.id}/sessions", headers=admin_headers).json()["sessions"] == []
    # Ending sessions is not a change of rights: nothing about the account moved.
    db_session.refresh(anna)
    assert (anna.permissions_version, anna.is_active, anna.time_role) == (1, True, "manager")
    assert audit_rows(db_session, "admin_user_revoke_sessions")[-1][1]["sessions_ended"] == 2


def test_ending_ones_own_sessions_keeps_the_one_in_use(client, db_session):
    admin = seed(db_session, "root", role="admin")
    here, elsewhere = bearer(admin), bearer(admin)
    marked = client.get(f"/api/admin/users/{admin.id}/sessions", headers=here).json()["sessions"]
    assert sorted(item["current"] for item in marked) == [False, True]

    ended = client.post(f"/api/admin/users/{admin.id}/revoke-sessions", headers=here)
    assert ended.json()["revoked"] == 1 and ended.json()["kept_current"] is True
    assert client.get("/api/auth/me", headers=here).status_code == 200
    assert client.get("/api/auth/me", headers=elsewhere).status_code == 401


def test_everybodys_sessions_belong_to_crm_administration(client, db_session):
    admin = seed(db_session, "owner", role="operator", checks=False, global_admin=True)
    anna = seed(db_session, "anna")
    admin_headers, anna_headers = bearer(admin), bearer(anna)

    listed = client.get("/api/security/sessions", headers=admin_headers)
    assert listed.status_code == 200
    by_user = {item["user_id"]: item["session_id"] for item in listed.json()}
    assert set(by_user) == {admin.id, anna.id}

    killed = client.delete(f"/api/security/sessions/{by_user[anna.id]}", headers=admin_headers)
    assert killed.status_code == 200
    assert client.get("/api/auth/me", headers=anna_headers).status_code == 401
    assert client.get("/api/auth/me", headers=admin_headers).status_code == 200


def test_unknown_accounts_are_not_found(client, db_session):
    admin = seed(db_session, "root", role="admin")
    headers = bearer(admin)
    for method, path in (("GET", "/api/admin/users/999/sessions"), ("POST", "/api/admin/users/999/revoke-sessions"),
                         ("PATCH", "/api/admin/users/999"), ("DELETE", "/api/admin/users/999"),
                         ("POST", "/api/admin/users/999/reset-password")):
        body = {"password": NEW_PASSWORD} if path.endswith("reset-password") else ({} if method != "GET" else None)
        response = client.request(method, path, headers=headers, json=body)
        assert response.status_code == 404 and error_code(response) == "user_not_found", (method, path)


# ---------------------------------------------------------------- Telegram identity


def test_a_telegram_identity_belongs_to_one_account_and_is_a_person(client, db_session):
    admin = seed(db_session, "root", role="admin")
    anna = seed(db_session, "anna", time_role="employee")
    boris = seed(db_session, "boris", time_role="employee")
    headers = bearer(admin)

    linked = client.post(f"/api/admin/users/{anna.id}/link-telegram", headers=headers,
                         json={"telegram_id": 111222333, "telegram_username": "anna_tg"})
    assert linked.status_code == 200 and linked.json()["telegram_id"] == 111222333

    taken = client.post(f"/api/admin/users/{boris.id}/link-telegram", headers=headers, json={"telegram_id": 111222333})
    assert taken.status_code == 409 and error_code(taken) == "telegram_id_already_linked"
    # A group or channel has a negative ID and is nobody's identity.
    for not_a_person in (-1001234567890, 0):
        refused = client.post(f"/api/admin/users/{boris.id}/link-telegram", headers=headers,
                              json={"telegram_id": not_a_person})
        assert refused.status_code == 422, refused.text

    unlinked = client.post(f"/api/admin/users/{anna.id}/unlink-telegram", headers=headers)
    assert unlinked.json()["telegram_id"] is None
    again = client.post(f"/api/admin/users/{boris.id}/link-telegram", headers=headers, json={"telegram_id": 111222333})
    assert again.status_code == 200
    actions = [action for action, _, _ in audit_rows(db_session)]
    assert actions.count("admin_user_link_telegram") == 2 and "admin_user_unlink_telegram" in actions


# ---------------------------------------------------------------- the record


def test_every_administrative_action_is_on_record_with_who_and_what(client, db_session):
    admin = seed(db_session, "root", role="admin")
    headers = bearer(admin)
    created = client.post("/api/admin/users", headers=headers,
                          json={"username": "anna", "password": NEW_PASSWORD}).json()
    user_id = created["id"]
    client.patch(f"/api/admin/users/{user_id}", headers=headers,
                 json={"workspaces": {"time": "viewer", "checks": None}})
    client.post(f"/api/admin/users/{user_id}/reset-password", headers=headers, json={"password": "Other#Pass7"})
    client.post(f"/api/admin/users/{user_id}/revoke-sessions", headers=headers)
    client.delete(f"/api/admin/users/{user_id}", headers=headers)
    client.patch(f"/api/admin/users/{user_id}", headers=headers, json={"display_name": "Анна"})

    recorded = [action for action, _, _ in audit_rows(db_session)]
    for action in ("admin_user_create", "admin_user_update", "admin_user_access_change", "admin_user_reset_password",
                   "admin_user_revoke_sessions", "admin_user_deactivate"):
        assert action in recorded, action
    assert all(row.user_id == admin.id for _, _, row in audit_rows(db_session))

    change = audit_rows(db_session, "admin_user_access_change")[0][1]
    assert change["before"] == {"is_active": True, "global_admin": False, "workspace_roles": {"checks": "operator"}}
    assert change["after"] == {"is_active": True, "global_admin": False, "workspace_roles": {"time": "viewer"}}
    # A change of the name alone is an update, not an access change.
    assert recorded.count("admin_user_access_change") == 1
    assert audit_rows(db_session, "admin_user_update")[-1][1]["fields"] == ["display_name"]

    listing = client.get("/api/admin/audit", headers=headers).json()["items"]
    assert listing[0]["action"] == "admin_user_update" and listing[0]["actor"] == "root"
    assert {item["action"] for item in listing} >= {"admin_user_create", "admin_user_access_change"}
    assert "Other#Pass7" not in json.dumps(listing, ensure_ascii=False)

    # Narrowed to one account: what was done to it, and what it did itself.
    other = client.post("/api/admin/users", headers=headers, json={"username": "boris", "password": NEW_PASSWORD}).json()
    about_anna = client.get(f"/api/admin/audit?user_id={user_id}", headers=headers).json()["items"]
    assert about_anna and all(item["details"].get("user_id") == user_id for item in about_anna)
    about_boris = client.get(f"/api/admin/audit?user_id={other['id']}", headers=headers).json()["items"]
    assert [item["action"] for item in about_boris] == ["admin_user_create"]


def test_the_account_audit_shows_sign_ins_and_nothing_of_the_receipts(client, db_session):
    admin = seed(db_session, "root", role="admin")
    for action in ("scope_unlock_ok", "system_token_ok", "excel_activation_failed", "sms_ingest_auth_ok"):
        db_session.add(AccessAuditLog(action=action, success=True, details_json='{"path": "/api/x"}'))
    db_session.commit()
    client.post("/api/auth/login", json={"username": "root", "password": "wrong-password"})
    client.post("/api/auth/login", json={"username": "root", "password": PASSWORD})

    items = client.get("/api/admin/audit", headers=bearer(admin)).json()["items"]
    assert {item["action"] for item in items} == {"login_failed", "login_success"}
    assert "wrong-password" not in json.dumps(items)


# ---------------------------------------------------------------- what the panel is drawn from


def test_the_panel_gets_what_it_needs_to_draw_its_forms(client, db_session, monkeypatch):
    admin = seed(db_session, "root", role="operator", checks=False, global_admin=True)
    db_session.add(AccessScope(name="2026", password_hash="unused", salt="unused", years_json="[2026]"))
    db_session.commit()
    meta = client.get("/api/admin/meta", headers=bearer(admin))
    assert meta.status_code == 200, meta.text
    body = meta.json()
    assert body["me"]["username"] == "root"

    assert [item["id"] for item in body["workspaces"]] == ["checks", "time"]
    checks, time = body["workspaces"]
    assert (checks["title"], time["title"]) == ("Чеки", "Учёт времени")
    assert [role["id"] for role in checks["roles"]] == ["operator", "admin"]
    assert [role["id"] for role in time["roles"]] == ["employee", "viewer", "manager"]
    assert (checks["default_role"], time["default_role"]) == ("operator", "employee")
    # An employee acts through the bot: the role is real, the Hub shows nothing for it.
    assert [role["in_hub"] for role in time["roles"]] == [False, True, True]
    assert all(role["title"] and role["hint"] for item in body["workspaces"] for role in item["roles"])
    # Time is not connected on this server, and the panel can say so.
    assert checks["available"] is True and time["available"] is False

    assert [tab["id"] for tab in body["checks"]["tabs"]] == ALL_OPERATOR_TABS
    assert body["checks"]["folders"] == [{"id": 1, "name": "2026", "is_active": True}]
    assert body["password"]["min_length"] >= 8
    assert body["session_ttl_days"] == {"min": 1, "max": 90, "default": 7}
    # Reference data only: no hash of a folder password, no account.
    assert "password_hash" not in meta.text and "salt" not in meta.text

    monkeypatch.setenv("TIMELIST_INTERNAL_URL", "http://timelist.internal:8080")
    monkeypatch.setenv("TIMELIST_API_TOKEN", "stub-time-token")
    again = client.get("/api/admin/meta", headers=bearer(admin)).json()
    assert again["workspaces"][1]["available"] is True


def test_another_workspace_is_a_registry_entry(client, db_session, monkeypatch):
    """The claim the design rests on: a workspace is added in one place.

    A third workspace is put into the registry and nothing else is touched.
    Sign-in, the path check, the administration API's reference data and the
    effective access all pick it up. The one thing it would add for real is
    the column its role is kept in, which is why the row here is a stand-in.
    """
    payroll = ws.Workspace(
        id="payroll",
        title="Зарплата",
        description="Пример будущего рабочего пространства.",
        roles=(
            ws.WorkspaceRole("clerk", "Расчётчик", "Считает."),
            ws.WorkspaceRole("chief", "Главный бухгалтер", "Утверждает."),
        ),
        default_role="clerk",
        hub_roles=frozenset({"clerk", "chief"}),
        api_prefixes=("/api/payroll",),
        role_column="payroll_role",
    )
    monkeypatch.setattr(ws, "WORKSPACES", ws.WORKSPACES + (payroll,))

    # The panel is told about it, with its roles.
    admin = seed(db_session, "root", role="admin")
    listed = client.get("/api/admin/meta", headers=bearer(admin)).json()["workspaces"]
    assert [item["id"] for item in listed] == ["checks", "time", "payroll"]
    assert [role["id"] for role in listed[2]["roles"]] == ["clerk", "chief"]

    # Its paths are its own, and an account without it is refused there.
    assert ws.required_workspace_for_path("/api/payroll/runs/7") == "payroll"
    assert ws.required_workspace_for_path("/api/payrolls") == "checks"
    from fake_auth_redis import issue_active_app_token

    token = issue_active_app_token(admin)
    conn = SimpleNamespace(headers={"authorization": f"Bearer {token}"}, url=SimpleNamespace(path="/api/payroll/runs"))
    with pytest.raises(HTTPException) as refused:
        asyncio.run(dependencies.get_current_user(conn, db_session))
    assert refused.value.status_code == 403
    assert refused.value.detail == {"error": "workspace_forbidden", "workspace": "payroll"}

    # Granting, listing and describing it go through the same functions as the others.
    row = SimpleNamespace(is_active=True, access_until=None, locked_until=None, global_admin=False,
                          role="operator", checks_access=False, time_role=None, payroll_role=None)
    assert ws.workspaces_of(row) == []
    ws.apply_workspace_role(row, "payroll", "chief")
    assert ws.workspace_roles_of(row) == {"payroll": "chief"} and ws.workspaces_of(row) == ["payroll"]
    assert ws.effective_access(row)["summary"] == ["Зарплата: главный бухгалтер"]
    with pytest.raises(ws.WorkspaceError, match="invalid_workspace_role"):
        ws.apply_workspace_role(row, "payroll", "manager")
    ws.apply_workspace_role(row, "payroll", None)
    assert ws.workspace_roles_of(row) == {}
    # And it made nobody an administrator of anything.
    assert ws.is_global_admin(row) is False


# ---------------------------------------------------------------- the first administrator


def _bootstrap_script(db_session, monkeypatch):
    path = Path(__file__).resolve().parents[1] / "scripts" / "create_admin.py"
    spec = importlib.util.spec_from_file_location("create_admin_under_test", path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    # The script closes its session; the test's session survives that.
    monkeypatch.setattr(module, "SessionLocal", lambda: db_session)
    return module


def test_the_first_crm_administrator_comes_from_the_servers_shell(client, db_session, monkeypatch, capsys):
    script = _bootstrap_script(db_session, monkeypatch)

    with pytest.raises(Exception):
        script.create_admin("owner", "short")
    assert db_session.query(User).count() == 0

    user_id = script.create_admin("owner", NEW_PASSWORD, "Владелец")
    owner = db_session.get(User, user_id)
    assert (owner.global_admin, owner.role, owner.checks_access, owner.time_role) == (True, "admin", True, None)
    assert NEW_PASSWORD not in (owner.password_hash or "")
    login = client.post("/api/auth/login", json={"username": "owner", "password": NEW_PASSWORD})
    assert login.status_code == 200 and login.json()["user"]["global_admin"] is True

    # Run again, it changes nothing: neither the password nor the rights.
    before = owner.password_hash
    assert script.create_admin("owner", "Another#Pass1") == user_id
    assert db_session.get(User, user_id).password_hash == before

    # The way back in when nobody administers: an existing account, by name.
    anna_id = seed(db_session, "anna", time_role="viewer").id
    script.promote_admin("anna")
    anna = db_session.get(User, anna_id)
    assert (anna.global_admin, anna.role, anna.time_role) == (True, "operator", "viewer")
    with pytest.raises(SystemExit):
        script.promote_admin("nobody-by-that-name")
    off_id = seed(db_session, "off", active=False).id
    with pytest.raises(SystemExit):
        script.promote_admin("off")
    assert db_session.get(User, off_id).global_admin is False

    # It is in the journal the panel shows, without the password.
    recorded = [action for action, _, _ in audit_rows(db_session)]
    assert "admin_bootstrap_create" in recorded and "admin_bootstrap_promote" in recorded
    assert NEW_PASSWORD not in all_audit_text(db_session) and NEW_PASSWORD not in capsys.readouterr().out
