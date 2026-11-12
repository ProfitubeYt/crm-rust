"""Account administration: who may do it, what it cannot break, what it leaves behind.

Real application, real tokens with registered sessions, isolated SQLite.
"""
from __future__ import annotations

import json

import pytest

from database.models import AccessAuditLog, AccessScope, User
from hub_support import PASSWORD, all_audit_text, app_client, audit_rows, bearer, error_code, seed

NEW_PASSWORD = "Fresh#Pass9"


@pytest.fixture
def client(db_session, monkeypatch, tmp_path):
    with app_client(db_session, monkeypatch, tmp_path) as test_client:
        yield test_client


# ---------------------------------------------------------------- who may administer


def _admin_routes():
    """Every /api/admin route of the running application, with a usable path."""
    import re

    from fastapi.routing import APIRoute

    from api.main import app

    routes = []
    for route in app.routes:
        if isinstance(route, APIRoute) and route.path.startswith("/api/admin"):
            for method in sorted(route.methods - {"HEAD", "OPTIONS"}):
                routes.append((method, re.sub(r"\{[^}]+\}", "1", route.path)))
    return sorted(routes)


def test_an_ordinary_account_cannot_call_any_administration_route(client, db_session):
    # Every right an operator can hold, in both workspaces: still not an administrator.
    seed(db_session, "root", role="admin")
    operator = seed(
        db_session, "op", checks=True, time_role="manager",
        tabs=["dashboard", "reference", "automation", "userbot", "logs"],
    )
    headers = bearer(operator)
    routes = _admin_routes()
    assert len(routes) >= 14, "the route table was not read"

    for method, path in routes:
        body = None if method in {"GET", "DELETE"} else {}
        signed_in = client.request(method, path, headers=headers, json=body)
        assert signed_in.status_code == 403 and error_code(signed_in) == "admin_only", (method, path, signed_in.text)
        anonymous = client.request(method, path, json=body)
        assert anonymous.status_code == 401, (method, path, anonymous.text)

    assert db_session.query(User).count() == 2
    assert db_session.get(User, operator.id).role == "operator"


def test_an_account_cannot_grant_itself_anything(client, db_session):
    seed(db_session, "root", role="admin")
    user = seed(db_session, "anna", checks=True, time_role=None)
    headers = bearer(user)
    for change in ({"time_role": "manager"}, {"role": "admin"}, {"checks_access": True}, {"allowed_tabs": ["logs"]}):
        response = client.patch(f"/api/admin/users/{user.id}", headers=headers, json=change)
        assert response.status_code == 403, response.text
    db_session.refresh(user)
    assert (user.role, user.time_role) == ("operator", None)


def test_an_administrator_without_any_workspace_still_administers(client, db_session):
    admin = seed(db_session, "root", role="admin", checks=False, time_role=None)
    headers = bearer(admin)
    assert client.get("/api/admin/users", headers=headers).status_code == 200
    assert client.get("/api/admin/meta", headers=headers).status_code == 200
    # ...and reaches a workspace only once it is granted, like anyone else.
    refused = client.get("/api/transactions/years", headers=headers)
    assert refused.status_code == 403 and error_code(refused) == "workspace_forbidden"
    granted = client.patch(f"/api/admin/users/{admin.id}", headers=headers, json={"checks_access": True})
    assert granted.status_code == 200, granted.text
    assert client.get("/api/transactions/years", headers=headers).status_code == 200
    # Granting oneself is allowed to an administrator, and it is on record.
    change = audit_rows(db_session, "admin_user_access_change")[-1][1]
    assert change["before"]["checks_access"] is False and change["after"]["checks_access"] is True


# ---------------------------------------------------------------- creating and granting


@pytest.mark.parametrize(
    "extra,workspaces,summary",
    [
        ({}, ["checks"], ["Чеки: оператор"]),
        ({"checks_access": False, "time_role": "manager"}, ["time"], ["Учёт времени: руководитель"]),
        ({"time_role": "viewer"}, ["checks", "time"], ["Чеки: оператор", "Учёт времени: просмотр"]),
        ({"checks_access": False}, [], ["Нет доступа к рабочим пространствам"]),
        ({"checks_access": False, "time_role": "employee"}, [],
         ["Учёт времени: сотрудник (только отметки через бота)"]),
    ],
)
def test_an_account_is_created_with_exactly_the_access_asked_for(client, db_session, extra, workspaces, summary):
    admin = seed(db_session, "root", role="admin")
    response = client.post(
        "/api/admin/users", headers=bearer(admin),
        json={"username": "newbie", "password": NEW_PASSWORD, "display_name": "Новый", **extra},
    )
    assert response.status_code == 200, response.text
    body = response.json()
    assert body["workspaces"] == workspaces
    assert body["access_summary"] == summary
    assert body["checks_access"] is extra.get("checks_access", True)
    assert body["time_role"] == extra.get("time_role")

    created = audit_rows(db_session, "admin_user_create")[-1][1]
    assert created["access"]["checks_access"] is extra.get("checks_access", True)
    assert created["access"]["time_role"] == extra.get("time_role")

    # The account signs in with that password and the Hub is told the same list.
    login = client.post("/api/auth/login", json={"username": "newbie", "password": NEW_PASSWORD})
    assert login.status_code == 200 and login.json()["user"]["workspaces"] == workspaces


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

    client.patch(f"/api/admin/users/{user_id}", headers=headers, json={"time_role": "manager"})
    edited = client.patch(f"/api/admin/users/{user_id}", headers=headers,
                          json={"display_name": "Оператор", "allowed_tabs": ["dashboard", "reference"]})
    assert edited.status_code == 200, edited.text
    # An edit that does not mention workspaces leaves them as they were.
    assert edited.json()["workspaces"] == ["checks", "time"]
    assert edited.json()["time_role"] == "manager"


def test_a_time_role_is_one_of_three_or_nothing(client, db_session):
    admin = seed(db_session, "root", role="admin")
    user = seed(db_session, "anna")
    headers = bearer(admin)
    for bad in ("owner", "admin", "MANAGER ", "viewer,manager"):
        response = client.patch(f"/api/admin/users/{user.id}", headers=headers, json={"time_role": bad})
        assert response.status_code == 422, (bad, response.text)
    db_session.refresh(user)
    assert user.time_role is None
    for role in ("employee", "viewer", "manager"):
        assert client.patch(f"/api/admin/users/{user.id}", headers=headers, json={"time_role": role}).json()["time_role"] == role
    for clear in ("", "none"):
        client.patch(f"/api/admin/users/{user.id}", headers=headers, json={"time_role": "viewer"})
        assert client.patch(f"/api/admin/users/{user.id}", headers=headers, json={"time_role": clear}).json()["time_role"] is None
    bad_create = client.post("/api/admin/users", headers=headers,
                             json={"username": "x", "password": NEW_PASSWORD, "time_role": "owner"})
    assert bad_create.status_code == 422


def test_checks_restrictions_are_untouched_by_workspace_changes(client, db_session):
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

    for change in ({"time_role": "manager"}, {"checks_access": False}, {"checks_access": True}, {"time_role": ""}):
        after = client.patch(f"/api/admin/users/{created['id']}", headers=headers, json=change).json()
        assert {key: after[key] for key in restrictions} == restrictions, change


# ---------------------------------------------------------------- nobody is locked out by accident


def test_the_last_administrator_cannot_be_disabled_or_demoted(client, db_session):
    admin = seed(db_session, "root", role="admin")
    second = seed(db_session, "second", role="admin")
    headers = bearer(admin)

    # Not oneself, whoever else is there.
    assert error_code(client.delete(f"/api/admin/users/{admin.id}", headers=headers)) == "cannot_disable_self"
    assert error_code(client.patch(f"/api/admin/users/{admin.id}", headers=headers, json={"is_active": False})) == "cannot_disable_self"
    assert error_code(client.patch(f"/api/admin/users/{admin.id}", headers=headers, json={"role": "operator"})) == "cannot_demote_self"

    # Another administrator can be removed while one stays...
    demoted = client.patch(f"/api/admin/users/{second.id}", headers=headers, json={"role": "operator"})
    assert demoted.status_code == 200 and demoted.json()["role"] == "operator"
    # ...and the one who stays cannot be removed by any path, the service included.
    from services.user_service import UserServiceError, deactivate_user, update_user

    db_session.refresh(admin)
    for attempt in (lambda: deactivate_user(db_session, admin),
                    lambda: update_user(db_session, admin, is_active=False),
                    lambda: update_user(db_session, admin, role="operator")):
        with pytest.raises(UserServiceError, match="last_admin"):
            attempt()
    db_session.refresh(admin)
    assert admin.role == "admin" and admin.is_active is True

    # With two administrators again, either may remove the other, never both.
    client.patch(f"/api/admin/users/{second.id}", headers=headers, json={"role": "admin"})
    second_headers = bearer(db_session.get(User, second.id))
    removed = client.delete(f"/api/admin/users/{admin.id}", headers=second_headers)
    assert removed.status_code == 200 and removed.json()["is_active"] is False
    last = client.patch(f"/api/admin/users/{second.id}", headers=second_headers, json={"is_active": False})
    assert last.status_code == 409 and error_code(last) == "cannot_disable_self"
    assert db_session.query(User).filter(User.role == "admin", User.is_active.is_(True)).count() == 1


def test_an_account_is_disabled_and_enabled_again_without_losing_its_access(client, db_session):
    admin = seed(db_session, "root", role="admin")
    user = seed(db_session, "anna", checks=True, time_role="viewer")
    headers = bearer(admin)
    user_headers = bearer(user)

    disabled = client.patch(f"/api/admin/users/{user.id}", headers=headers, json={"is_active": False})
    assert disabled.status_code == 200 and disabled.json()["is_active"] is False
    assert disabled.json()["access_summary"][0] == "Учётная запись отключена"
    assert client.get("/api/auth/me", headers=user_headers).status_code in (401, 403)
    assert client.get(f"/api/admin/users/{user.id}/sessions", headers=headers).json()["sessions"] == []

    enabled = client.patch(f"/api/admin/users/{user.id}", headers=headers, json={"is_active": True})
    assert enabled.json()["is_active"] is True and enabled.json()["workspaces"] == ["checks", "time"]
    # The old token stays dead; the account signs in again with its own password.
    assert client.get("/api/auth/me", headers=user_headers).status_code == 401
    assert client.post("/api/auth/login", json={"username": "anna", "password": PASSWORD}).status_code == 200


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
    client.patch(f"/api/admin/users/{user_id}", headers=headers, json={"time_role": "viewer", "checks_access": False})
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
    assert change["before"] == {"is_active": True, "role": "operator", "checks_access": True, "time_role": None}
    assert change["after"] == {"is_active": True, "role": "operator", "checks_access": False, "time_role": "viewer"}
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


def test_the_panel_gets_what_it_needs_to_draw_its_forms(client, db_session):
    admin = seed(db_session, "root", role="admin")
    db_session.add(AccessScope(name="2026", password_hash="unused", salt="unused", years_json="[2026]"))
    db_session.commit()
    meta = client.get("/api/admin/meta", headers=bearer(admin))
    assert meta.status_code == 200, meta.text
    body = meta.json()
    assert body["me"]["username"] == "root"
    assert [role["id"] for role in body["roles"]] == ["operator", "admin"]
    assert [role["id"] for role in body["time_roles"]] == ["employee", "viewer", "manager"]
    assert [tab["id"] for tab in body["tabs"]] == ["dashboard", "reference", "automation", "userbot", "logs"]
    assert body["folders"] == [{"id": 1, "name": "2026", "is_active": True}]
    assert body["password"]["min_length"] >= 8
    assert body["time"] == {"configured": False}
    # Reference data only: no hash of a folder password, no account.
    assert "password_hash" not in meta.text and "salt" not in meta.text
