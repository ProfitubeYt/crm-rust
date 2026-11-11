"""HTTP contract and authorization for Excel; isolated SQLite, no app lifespan.

Only the dashboard login/scope dependencies and external configuration/events
are replaced. Protocol authentication, permissions, SQL filters and writes run
through the real router and service.
"""
from __future__ import annotations

import hashlib
import json
from contextlib import contextmanager
from datetime import date, datetime, timedelta, timezone
from decimal import Decimal
from types import SimpleNamespace
from uuid import uuid4

import pytest
from fastapi import FastAPI
from fastapi.testclient import TestClient
from sqlalchemy import event, update as sql_update

from api import dependencies
from api.routes import excel_sync as routes
from database.connection import get_db_session
from database.excel_sync_models import ExcelSyncActivation, ExcelSyncGrant, ExcelSyncOperation
from database.models import (
    AccessAuditLog, AccessScope, AnnotationArticle, AnnotationPerson, AppLaunchConfig,
    HiddenBotChat, LockedPeriod, MonitoredBotChat, SyncDeletion, Transaction, TransactionAnnotationEvent, User,
)
from services.excel_sync_service import serialize_record
from services.user_service import build_permissions_snapshot


@pytest.fixture
def api(db_session, monkeypatch):
    user = User(username="excel-api-owner", password_hash="unused", salt="unused", role="admin")
    db_session.add_all([
        user,
        AnnotationPerson(code="СТ", full_name="Test person"),
        AnnotationPerson(code="ТТ", full_name="Another person"),
        AnnotationPerson(code="OFF", full_name="Inactive", is_active=False),
        AnnotationArticle(code="р1", label="Test expense", direction="expense"),
        AnnotationArticle(code="д1", label="Test income", direction="income"),
    ])
    db_session.commit()
    state = SimpleNamespace(db=db_session, user=user, scope=None, events=[])

    def database():
        try:
            yield db_session
        except Exception:
            db_session.rollback()
            raise

    async def publish(action, ids):
        state.events.append((action, list(ids)))

    monkeypatch.setattr(routes, "system_access_enforced", lambda: False)
    for module in (routes, dependencies, routes.tx_api):
        monkeypatch.setattr(module, "get_config_scopes", lambda **_kwargs: [])
    monkeypatch.setattr(routes.tx_api, "_publish_sync_update", publish)
    app = FastAPI()
    app.include_router(routes.router)
    app.dependency_overrides[get_db_session] = database
    app.dependency_overrides[routes._dashboard_user] = lambda: build_permissions_snapshot(state.user)
    app.dependency_overrides[dependencies.require_transactions_scope] = lambda: state.scope
    state.app = app
    with TestClient(app) as client:
        state.client = client
        yield state


def issue(api, **payload):
    response = api.client.post("/api/excel/grants", json={"name": "Test workbook", **payload})
    assert response.status_code == 201, response.text
    assert response.headers["cache-control"] == "no-store"
    data = response.json()
    workbook = str(uuid4())
    return SimpleNamespace(
        data=data, workbook=workbook,
        headers={"Authorization": f"Bearer {data['token']}", "X-Excel-Workbook": workbook},
    )


def test_download_returns_the_shipped_macro_workbook(api):
    from pathlib import Path
    path = Path(__file__).resolve().parents[1] / "assets/excel_sync_template.xlsm"
    response = api.client.get("/api/excel/workbook")
    assert response.status_code == 200
    assert response.content == path.read_bytes()
    assert response.headers["cache-control"] == "no-store"
    assert response.headers["content-type"] == "application/vnd.ms-excel.sheet.macroEnabled.12"
    assert "filename*=utf-8''" in response.headers["content-disposition"].lower()


def snapshot(api, grant):
    response = api.client.get("/api/excel/v1/snapshot", headers=grant.headers)
    assert response.status_code == 200, response.text
    assert response.headers["cache-control"] == "no-store"
    return response.json()


def sync(api, grant, *operations):
    response = api.client.post("/api/excel/v1/sync", headers=grant.headers,
                               json={"workbook_id": grant.workbook, "operations": list(operations)})
    assert response.status_code == 200, response.text
    assert response.headers["cache-control"] == "no-store"
    return response.json()["results"]


def transaction(api, *, source=10, when="2026-09-01T12:30:00", **values):
    row = Transaction(
        source_type="AUTO", source_chat_id=source,
        transaction_date=datetime.fromisoformat(when), amount=Decimal("-100.25"),
        transaction_type="DEBIT", currency="UZS", operator_raw="Synthetic merchant",
        card_last_4="0001", raw_message="Synthetic receipt", is_p2p=False,
        **values,
    )
    api.db.add(row)
    api.db.commit()
    return row


def update(record, **fields):
    return {"operation_id": str(uuid4()), "kind": "update", "id": record["id"],
            "uuid": record["uuid"], "fields": fields,
            "base": {field: record["fields"][field] for field in fields}}


def delete(record):
    return {"operation_id": str(uuid4()), "kind": "delete", "id": record["id"],
            "uuid": record["uuid"], "version": record["version"]}


def create_fields():
    return {
        "transaction_date": "2026-09-01T12:30:00", "amount": "-1250.50",
        "transaction_type": "DEBIT", "currency": "UZS", "operator_raw": "New merchant",
        "application_mapped": "Test app", "balance_after": "400.25", "card_last_4": "0007",
        "receiver_name": "Test receiver", "receiver_card": "0000123412341234", "is_p2p": True,
        "raw_message": "Synthetic full-field receipt", "annotation_details": "Test note",
        "annotation_person_code": "СТ", "annotation_period": "2026-09-01", "annotation_article_code": "р1",
    }


def make_scope(api, *, name="2026 folder", years=(2026,)):
    row = AccessScope(name=name, password_hash="unused", salt="unused", years_json=json.dumps(years))
    api.db.add(row)
    api.db.commit()
    return row


def operator(api, sources=(0, 10)):
    folder = make_scope(api)
    api.user.role = "operator"
    api.user.allowed_sources = json.dumps(sources)
    api.user.allowed_folders = json.dumps([folder.id])
    api.db.commit()
    api.scope = routes._current_scope_definition(api.db, folder.id)
    return folder


def lock(api, day="2026-09-02", excluded=()):
    value = date.fromisoformat(day)
    row = LockedPeriod(date_from=value, date_to=value, locked_by_tg_id=1, excluded_user_ids=list(excluded))
    api.db.add(row)
    api.db.commit()
    return row


def test_grant_is_hashed_one_time_user_owned_and_revocable(api):
    grant = issue(api, name="  Work book  ", expires_in_days=30)
    row = api.db.query(ExcelSyncGrant).one()
    assert row.name == "Work book"
    assert row.token_hash == hashlib.sha256(grant.data["token"].encode()).hexdigest()
    assert row.expires_at - row.created_at == pytest.approx(timedelta(days=30), abs=timedelta(seconds=1))
    assert grant.data["server_url"] == "http://testserver"
    assert grant.data["grant"]["workbook_id"] is None

    listed = api.client.get("/api/excel/grants")
    assert listed.headers["cache-control"] == "no-store"
    assert listed.json()["grants"] == [grant.data["grant"]]
    assert grant.data["token"] not in listed.text and row.token_hash not in listed.text
    audit = api.db.query(AccessAuditLog).one()
    assert audit.action == "excel_grant_created" and grant.data["token"] not in audit.details_json

    owner = api.user
    other = User(username="excel-api-other", password_hash="unused", salt="unused", role="admin")
    api.db.add(other)
    api.db.commit()
    api.user = other
    assert api.client.get("/api/excel/grants").json() == {"grants": []}
    path = f"/api/excel/grants/{row.id}"
    assert api.client.delete(path).status_code == 404
    api.user = owner
    assert api.client.delete(path).json() == {"ok": True}
    assert api.client.delete(path).json() == {"ok": True}
    assert api.db.query(AccessAuditLog).filter_by(action="excel_grant_revoked").count() == 1
    assert api.client.get("/api/excel/v1/snapshot", headers=grant.headers).status_code == 401


def test_admin_permanent_grant_authenticates_and_edits_after_long_time(api, monkeypatch):
    tx = transaction(api)
    grant = issue(api, expires_in_days=None)
    row = api.db.query(ExcelSyncGrant).one()
    assert row.expires_at is None
    assert grant.data["grant"]["expires_at"] is None
    audit = json.loads(api.db.query(AccessAuditLog).one().details_json)
    assert "expires_at" in audit and audit["expires_at"] is None

    future = row.created_at + timedelta(days=3650)
    monkeypatch.setattr(routes, "_now", lambda: future)
    record = snapshot(api, grant)["records"][0]
    assert record["id"] == str(tx.id)
    assert row.workbook_id == grant.workbook and row.last_used_at == future
    assert api.client.get("/api/excel/grants").json()["grants"][0]["expires_at"] is None
    result = sync(api, grant, update(record, annotation_details="Permanent grant edit"))[0]
    assert result["status"] == "applied"
    assert api.db.get(Transaction, tx.id).annotation_details == "Permanent grant edit"
    assert api.db.query(ExcelSyncOperation).count() == 1


def test_operator_cannot_issue_permanent_grant_but_keeps_default_expiry(api):
    operator(api)
    response = api.client.post("/api/excel/grants", json={"name": "Permanent", "expires_in_days": None})
    assert response.status_code == 403
    assert response.json()["detail"] == "permanent_excel_connection_admin_only"
    assert api.db.query(ExcelSyncGrant).count() == 0
    assert api.db.query(AccessAuditLog).count() == 0

    issue(api)
    row = api.db.query(ExcelSyncGrant).one()
    assert row.expires_at - row.created_at == pytest.approx(timedelta(days=90), abs=timedelta(seconds=1))


def test_permanent_grant_does_not_trust_cached_admin_role(api):
    async def login_before_role_change():
        previous = build_permissions_snapshot(api.user)
        # Reproduce a DB change after authentication while the ORM identity map
        # still holds the old role. The locking query must refresh that object.
        api.db.execute(sql_update(User).where(User.id == api.user.id).values(
            role="operator", permissions_version=api.user.permissions_version + 1,
        ).execution_options(synchronize_session=False))
        return previous

    api.app.dependency_overrides[routes._dashboard_user] = login_before_role_change
    response = api.client.post("/api/excel/grants", json={"name": "Permanent", "expires_in_days": None})
    assert response.status_code == 403
    assert response.json()["detail"] == "permanent_excel_connection_admin_only"
    assert api.db.query(ExcelSyncGrant).count() == 0
    assert api.db.query(AccessAuditLog).count() == 0


@pytest.mark.parametrize("field,value", [("is_active", False), ("locked_until", datetime(2100, 1, 1))])
def test_permanent_grant_creation_checks_current_admin_state(api, field, value):
    setattr(api.user, field, value)
    api.db.commit()
    response = api.client.post("/api/excel/grants", json={"name": "Permanent", "expires_in_days": None})
    assert response.status_code == 403
    assert api.db.query(ExcelSyncGrant).count() == 0
    assert api.db.query(AccessAuditLog).count() == 0


@pytest.mark.parametrize("guard,expected_status", [
    ("revoked", 401), ("inactive", 403), ("permissions", 403), ("locked", 403),
    ("workbook", 403), ("scope", 403), ("system", 503),
])
def test_permanent_grants_preserve_read_and_write_authorization(api, monkeypatch, guard, expected_status):
    tx = transaction(api)
    if guard == "scope":
        folder = make_scope(api)
        api.scope = routes._current_scope_definition(api.db, folder.id)
    grant = issue(api, expires_in_days=None)
    record = snapshot(api, grant)["records"][0]
    headers = dict(grant.headers)
    if guard == "revoked":
        assert api.client.delete(f"/api/excel/grants/{grant.data['grant']['id']}").status_code == 200
    elif guard == "inactive":
        api.user.is_active = False
    elif guard == "permissions":
        api.user.permissions_version += 1
    elif guard == "locked":
        api.user.locked_until = datetime(2100, 1, 1)
    elif guard == "workbook":
        headers["X-Excel-Workbook"] = str(uuid4())
    elif guard == "scope":
        folder.allow_transactions = False
    else:
        monkeypatch.setattr(routes, "system_access_enforced", lambda: True)
        monkeypatch.setattr(routes, "is_system_blocked", lambda: (True, "test blocked configuration"))
    api.db.commit()

    assert api.client.get("/api/excel/v1/snapshot", headers=headers).status_code == expected_status
    response = api.client.post("/api/excel/v1/sync", headers=headers, json={
        "workbook_id": grant.workbook,
        "operations": [update(record, annotation_details="Must not be applied")],
    })
    assert response.status_code == expected_status
    assert api.db.get(Transaction, tx.id).annotation_details is None
    assert api.db.query(ExcelSyncOperation).count() == 0
    assert api.db.query(AccessAuditLog).filter_by(action="excel_sync_update").count() == 0
    assert api.events == []


@pytest.mark.parametrize("payload", [
    {"name": "   "}, {"name": "x" * 121}, {"expires_in_days": 0}, {"expires_in_days": 366},
    {"allowed_sources": [123]}, {"scope": {"years": []}},
])
def test_grant_validation_cannot_supply_wider_permissions(api, payload):
    response = api.client.post("/api/excel/grants", json={"name": "Book", **payload})
    assert response.status_code == 422
    assert api.db.query(ExcelSyncGrant).count() == 0


@pytest.mark.parametrize("permanent", [False, True])
def test_grant_limit_requires_revoking_unused_connections(api, permanent):
    api.db.add_all([ExcelSyncGrant(
        user_id=api.user.id, name=f"Book {i}", token_hash=hashlib.sha256(str(i).encode()).hexdigest(),
        permissions_version=1,
        expires_at=None if permanent else datetime.now(timezone.utc).replace(tzinfo=None) + timedelta(days=1),
    ) for i in range(20)])
    api.db.commit()
    assert api.client.post("/api/excel/grants", json={"name": "One too many"}).status_code == 409
    assert api.db.query(ExcelSyncGrant).count() == 20
    row = api.db.query(ExcelSyncGrant).first()
    assert api.client.delete(f"/api/excel/grants/{row.id}").status_code == 200
    issue(api, expires_in_days=None if permanent else 90)
    assert api.db.query(ExcelSyncGrant).count() == 21


@pytest.mark.parametrize("authorization", [None, "Basic synthetic", "Bearer shared-read-only-export-key",
                                         "Bearer dashboard.jwt.value", "Bearer excel_" + "x" * 43])
def test_protocol_rejects_missing_other_and_unknown_credentials(api, authorization):
    headers = {"X-Excel-Workbook": str(uuid4()), "X-Export-Key": "shared-read-only-export-key"}
    if authorization:
        headers["Authorization"] = authorization
    assert api.client.get("/api/excel/v1/snapshot", headers=headers).status_code == 401
    response = api.client.post("/api/excel/v1/sync", headers=headers,
                               json={"workbook_id": headers["X-Excel-Workbook"],
                                     "operations": [{"kind": "create", "operation_id": str(uuid4()), "fields": create_fields()}]})
    assert response.status_code == 401
    assert api.db.query(Transaction).count() == 0


def test_workbook_binding_and_expiry_are_enforced(api):
    grant = issue(api)
    bad_headers = {**grant.headers, "X-Excel-Workbook": "not-a-uuid"}
    assert api.client.get("/api/excel/v1/snapshot", headers=bad_headers).status_code == 400
    response = api.client.post("/api/excel/v1/sync", headers=grant.headers,
                               json={"workbook_id": str(uuid4()), "operations": [{"kind": "create"}]})
    assert response.status_code == 400
    assert api.db.query(ExcelSyncGrant).one().workbook_id is None
    assert snapshot(api, grant)["workbook_id"] == grant.workbook
    row = api.db.query(ExcelSyncGrant).one()
    assert row.workbook_id == grant.workbook and row.last_used_at is not None
    assert api.client.get("/api/excel/v1/snapshot", headers={**grant.headers, "X-Excel-Workbook": str(uuid4())}).status_code == 403
    row.expires_at = datetime.now(timezone.utc).replace(tzinfo=None) - timedelta(seconds=1)
    api.db.commit()
    assert api.client.get("/api/excel/v1/snapshot", headers=grant.headers).status_code == 401


def test_snapshot_is_complete_has_stable_identity_and_active_references(api, monkeypatch):
    rows = [transaction(api, source=9007199254740993, source_message_id=9007199254740993), transaction(api), transaction(api)]
    grant = issue(api)
    data = snapshot(api, grant)
    assert data["complete"] is True and data["protocol_version"] == 1
    assert data["user_id"] == str(api.user.id)
    assert [r["id"] for r in data["records"]] == [str(row.id) for row in rows]
    record = data["records"][0]
    assert record["uuid"] == str(rows[0].uuid) and len(record["version"]) == 64
    assert record["fields"]["amount"] == "-100.25"
    assert set(record["fields"]) == set(create_fields()) | {"excel_bank", "p2p_confirmed"}
    assert record["fields"]["excel_bank"] is None
    assert record["fields"]["p2p_confirmed"] is None
    assert record["metadata"]["source_message_id"] == "9007199254740993"
    assert record["metadata"]["source_chat_id"] == "9007199254740993"
    assert {p["code"] for p in data["references"]["persons"]} == {"СТ", "ТТ"}
    monkeypatch.setenv("EXCEL_SYNC_MAX_ROWS", "2")
    response = api.client.get("/api/excel/v1/snapshot", headers=grant.headers)
    assert response.status_code == 413
    assert "records" not in response.json()  # Never label a truncated snapshot complete.


def test_snapshot_skips_redundant_fastapi_encoding_and_keeps_json_contract(api, monkeypatch):
    import fastapi.routing

    row = transaction(api, source=9007199254740993, balance_after=Decimal("123.45"),
                      annotation_period=date(2026, 9, 1), annotation_details="Проверка текста")
    expected = serialize_record(row)
    grant = issue(api)

    def redundant_encoding(*_args, **_kwargs):
        pytest.fail("The canonical snapshot must not be traversed by jsonable_encoder again")

    monkeypatch.setattr(fastapi.routing, "jsonable_encoder", redundant_encoding)
    response = api.client.get("/api/excel/v1/snapshot", headers=grant.headers)
    assert response.status_code == 200
    assert response.headers["cache-control"] == "no-store"
    assert response.headers["content-type"] == "application/json"
    data = response.json()
    record = data["records"][0]
    assert data["complete"] is True and data["workbook_id"] == grant.workbook
    assert record["fields"] == expected["fields"]
    assert record["version"] == expected["version"] and record["uuid"] == expected["uuid"]
    assert record["metadata"]["source_chat_id"] == "9007199254740993"
    assert {p["code"] for p in data["references"]["persons"]} == {"СТ", "ТТ"}


@contextmanager
def capture_selects(db):
    statements = []

    def capture(_conn, _cursor, statement, _parameters, _context, _executemany):
        normalized = " ".join(statement.lower().split())
        if normalized.startswith("select "):
            statements.append(normalized)

    engine = db.get_bind()
    event.listen(engine, "before_cursor_execute", capture)
    try:
        yield statements
    finally:
        event.remove(engine, "before_cursor_execute", capture)


def test_snapshot_etag_ignores_clock_but_rechecks_database_and_commits_usage(api, monkeypatch):
    transaction(api)
    grant = issue(api)
    first = api.client.get("/api/excel/v1/snapshot", headers=grant.headers)
    etag = first.headers["etag"]
    stable = {key: value for key, value in first.json().items() if key != "server_time"}
    encoded = json.dumps(stable, ensure_ascii=False, sort_keys=True, separators=(",", ":"), allow_nan=False)
    assert etag == 'W/"' + hashlib.sha256(encoded.encode("utf-8")).hexdigest() + '"'
    old_usage = api.db.query(ExcelSyncGrant).one().last_used_at
    later = routes._now() + timedelta(seconds=1)
    monkeypatch.setattr(routes, "_now", lambda: later)
    second = api.client.get("/api/excel/v1/snapshot", headers=grant.headers)
    assert second.status_code == 200 and second.headers["etag"] == etag
    assert second.json()["server_time"] != first.json()["server_time"]
    with capture_selects(api.db) as statements:
        unchanged = api.client.get("/api/excel/v1/snapshot", headers={**grant.headers, "If-None-Match": etag})
    assert unchanged.status_code == 304 and unchanged.content == b""
    assert unchanged.headers["etag"] == etag and unchanged.headers["cache-control"] == "no-store"
    for table in ("excel_sync_grants", "users", "transactions", "annotation_articles", "annotation_persons"):
        assert any(f" from {table} " in sql for sql in statements)
    assert api.db.query(ExcelSyncGrant).one().last_used_at == later > old_usage


@pytest.mark.parametrize("validator", ["strong", "wildcard", "list"])
def test_snapshot_only_accepts_an_exact_validator(api, validator):
    transaction(api)
    grant = issue(api)
    etag = api.client.get("/api/excel/v1/snapshot", headers=grant.headers).headers["etag"]
    supplied = {"strong": etag[2:], "wildcard": "*", "list": etag + ', "other"'}[validator]
    response = api.client.get("/api/excel/v1/snapshot", headers={**grant.headers, "If-None-Match": supplied})
    assert response.status_code == 200 and response.json()["complete"] is True


@pytest.mark.parametrize("guard", ["revoked", "expired", "inactive_user"])
def test_snapshot_validator_never_bypasses_current_authorization(api, guard):
    transaction(api)
    grant = issue(api)
    etag = api.client.get("/api/excel/v1/snapshot", headers=grant.headers).headers["etag"]
    row = api.db.query(ExcelSyncGrant).one()
    if guard == "revoked":
        row.revoked_at = routes._now()
    elif guard == "expired":
        row.expires_at = routes._now() - timedelta(seconds=1)
    else:
        api.user.is_active = False
    api.db.commit()
    response = api.client.get("/api/excel/v1/snapshot", headers={**grant.headers, "If-None-Match": etag})
    assert response.status_code == (403 if guard == "inactive_user" else 401)
    assert "records" not in response.json()


def test_snapshot_validator_is_bound_to_workbook_and_user(api):
    transaction(api)
    first = issue(api)
    initial = api.client.get("/api/excel/v1/snapshot", headers=first.headers)
    etag = initial.headers["etag"]
    another_book = issue(api)
    wrong_binding = api.client.get("/api/excel/v1/snapshot", headers={
        **first.headers, "X-Excel-Workbook": another_book.workbook, "If-None-Match": etag,
    })
    assert wrong_binding.status_code == 403
    other = api.client.get("/api/excel/v1/snapshot", headers={**another_book.headers, "If-None-Match": etag})
    assert other.status_code == 200 and other.headers["etag"] != etag
    api.user = User(username="another-excel-owner", password_hash="unused", salt="unused", role="admin")
    api.db.add(api.user)
    api.db.commit()
    other_user = issue(api)
    # Even identical visible data and the same workbook UUID cannot share a
    # validator across owners.
    response = api.client.get("/api/excel/v1/snapshot", headers={
        **other_user.headers, "X-Excel-Workbook": first.workbook, "If-None-Match": etag,
    })
    assert response.status_code == 200 and response.headers["etag"] != etag
    assert response.json()["user_id"] != initial.json()["user_id"]


@pytest.mark.parametrize("change", ["editable", "metadata", "source_label", "person_label", "article_label", "new_row"])
def test_snapshot_validator_changes_with_visible_content(api, change):
    row = transaction(api)
    api.db.add(MonitoredBotChat(chat_id=10, chat_title="CardXabar", chat_type="private"))
    api.db.commit()
    grant = issue(api)
    etag = api.client.get("/api/excel/v1/snapshot", headers=grant.headers).headers["etag"]
    if change == "editable":
        row.receiver_name = "Changed receiver"
    elif change == "metadata":
        row.annotation_details_src = "manual"
    elif change == "source_label":
        api.db.get(MonitoredBotChat, 10).chat_title = "Changed source"
    elif change == "person_label":
        api.db.query(AnnotationPerson).filter_by(code="СТ").one().full_name = "Changed person"
    elif change == "article_label":
        api.db.query(AnnotationArticle).filter_by(code="р1").one().label = "Changed article"
    else:
        transaction(api)
    api.db.commit()
    response = api.client.get("/api/excel/v1/snapshot", headers={**grant.headers, "If-None-Match": etag})
    assert response.status_code == 200 and response.headers["etag"] != etag
    unchanged = api.client.get("/api/excel/v1/snapshot", headers={
        **grant.headers, "If-None-Match": response.headers["etag"],
    })
    assert unchanged.status_code == 304 and unchanged.content == b""


def test_snapshot_validator_cannot_bypass_complete_snapshot_limit(api, monkeypatch):
    transaction(api)
    transaction(api)
    grant = issue(api)
    etag = api.client.get("/api/excel/v1/snapshot", headers=grant.headers).headers["etag"]
    monkeypatch.setenv("EXCEL_SYNC_MAX_ROWS", "1")
    response = api.client.get("/api/excel/v1/snapshot", headers={**grant.headers, "If-None-Match": etag})
    assert response.status_code == 413 and "records" not in response.json()


def test_snapshot_validator_recomputes_period_visibility_and_exemptions(api):
    operator(api)
    transaction(api)
    grant = issue(api)
    first = api.client.get("/api/excel/v1/snapshot", headers=grant.headers)
    assert len(first.json()["records"]) == 1
    period = LockedPeriod(date_from=date(2026, 9, 1), date_to=date(2026, 9, 1), locked_by_tg_id=1)
    api.db.add(period)
    api.db.commit()
    hidden = api.client.get("/api/excel/v1/snapshot", headers={**grant.headers, "If-None-Match": first.headers["etag"]})
    assert hidden.status_code == 200 and hidden.json()["records"] == []
    assert hidden.headers["etag"] != first.headers["etag"]
    period.excluded_user_ids = [api.user.id]
    api.db.commit()
    visible = api.client.get("/api/excel/v1/snapshot", headers={**grant.headers, "If-None-Match": hidden.headers["etag"]})
    assert visible.status_code == 200 and len(visible.json()["records"]) == 1
    assert visible.headers["etag"] == first.headers["etag"]


@pytest.mark.parametrize("count", [1, 20])
@pytest.mark.parametrize("replay", [False, True])
def test_sync_batches_source_reads_without_skipping_operation_checks(api, count, replay):
    api.db.add_all([
        MonitoredBotChat(chat_id=10, chat_title="CardXabar", chat_type="private"),
        HiddenBotChat(chat_id=11, title_snapshot="HUMO Card"),
        MonitoredBotChat(chat_id=12, chat_title="Unrelated source", chat_type="private"),
    ])
    api.db.commit()
    records = [serialize_record(transaction(api, source=10 + i % 2)) for i in range(count)]
    grant = issue(api)
    operations = [update(record, receiver_name="Updated receiver") for record in records]
    if replay:
        assert all(item["status"] == "applied" for item in sync(api, grant, *operations))
    # A previous fixture reference must not hide a redundant query in the identity map.
    api.db.expunge_all()
    with capture_selects(api.db) as statements:
        results = sync(api, grant, *operations)
    assert [item["record"]["id"] for item in results] == [record["id"] for record in records]
    assert all(item["status"] == "applied" for item in results)
    for i, item in enumerate(results):
        metadata = item["record"]["metadata"]
        assert metadata["source_chat_id"] == str(10 + i % 2)
        assert metadata["source_display"] == ("HUMO Card" if i % 2 else "CardXabar")
    tx_reads = [sql for sql in statements if " from transactions " in sql]
    # Each operation retains its authorized, fresh lock read; display enrichment
    # adds a single narrow query for the whole response, including replay batches.
    assert len(tx_reads) == count + 1
    assert sum("transactions.amount" not in sql for sql in tx_reads) == 1
    assert sum(" from excel_sync_operations " in sql for sql in statements) == count
    source_reads = [sql for sql in statements if
                    " from monitored_bot_chats " in sql or " from hidden_bot_chats " in sql]
    assert len(source_reads) == 2 and all(" in (" in sql for sql in source_reads)
    assert api.db.query(AccessAuditLog).filter_by(action="excel_sync_update").count() == count


def test_sync_without_returned_records_does_not_load_source_metadata(api):
    grant = issue(api)
    with capture_selects(api.db) as statements:
        result = sync(api, grant, {"operation_id": str(uuid4()), "kind": "invalid"})[0]
    assert result["status"] == "invalid"
    assert not any(" from monitored_bot_chats " in sql or " from hidden_bot_chats " in sql
                   for sql in statements)


@pytest.mark.parametrize("fields", [
    {"annotation_details": "Changed note"},
    {"annotation_period": "2026-09-01"},
    {"annotation_person_code": None, "annotation_article_code": None},
])
def test_annotation_values_without_reference_codes_do_not_query_dictionaries(api, fields):
    records = [serialize_record(transaction(api)) for _ in range(4)]
    grant = issue(api)
    with capture_selects(api.db) as statements:
        results = sync(api, grant, *(update(record, **fields) for record in records))
    assert all(item["status"] == "applied" for item in results)
    assert not any(" from annotation_articles " in sql or " from annotation_persons " in sql
                   for sql in statements)
    assert all(item["record"]["fields"][key] == value for item in results for key, value in fields.items())


def test_reference_validation_observes_deactivation_on_next_request(api):
    record = serialize_record(transaction(api))
    grant = issue(api)
    applied = sync(api, grant, update(record, annotation_person_code="СТ"))[0]
    assert applied["status"] == "applied"
    api.db.query(AnnotationPerson).filter_by(code="СТ").one().is_active = False
    api.db.commit()
    result = sync(api, grant, update(applied["record"], annotation_person_code="СТ"))[0]
    assert result["status"] == "invalid" and result["error"] == "invalid_annotation_reference"


def test_full_field_create_update_delete_and_retry_round_trip(api):
    grant = issue(api)
    create = {"operation_id": str(uuid4()), "kind": "create", "fields": create_fields()}
    result = sync(api, grant, create)[0]
    assert result["status"] == "applied"
    record = result["record"]
    assert record["fields"] == {**create_fields(), "excel_bank": None, "p2p_confirmed": None}
    assert record["metadata"]["source_type"] == "MANUAL"
    assert record["metadata"]["source_chat_id"] == "0"
    assert sync(api, grant, create)[0]["record"]["id"] == record["id"]
    assert api.db.query(Transaction).count() == 1
    assert api.db.query(ExcelSyncOperation).count() == 1

    fields = {
        "transaction_date": "2026-10-02T13:45:00", "amount": "250.75", "transaction_type": "CREDIT",
        "currency": "USD", "operator_raw": "Changed merchant", "application_mapped": "Another app",
        "balance_after": None, "card_last_4": "0042", "receiver_name": None, "receiver_card": None,
        "is_p2p": False, "raw_message": "Changed synthetic receipt", "annotation_details": None,
        "annotation_person_code": "ТТ", "annotation_period": "2026-10-01", "annotation_article_code": "д1",
    }
    operation = update(record, **fields)
    changed = sync(api, grant, operation)[0]
    expected = {**fields, "excel_bank": None, "p2p_confirmed": None}
    assert changed["status"] == "applied" and changed["record"]["fields"] == expected
    assert sync(api, grant, operation)[0]["record"]["fields"] == expected
    assert api.db.query(TransactionAnnotationEvent).count() == 8
    assert api.db.query(ExcelSyncOperation).count() == 2
    removal = delete(changed["record"])
    assert sync(api, grant, removal)[0]["status"] == "deleted"
    assert sync(api, grant, removal)[0]["status"] == "deleted"
    assert api.db.query(Transaction).count() == 0
    assert api.db.query(SyncDeletion).filter_by(record_id=int(record["id"])).count() == 1
    assert snapshot(api, grant)["records"] == []


@pytest.mark.parametrize("bank", ["CardXabar", "HUMO Card"])
def test_manual_excel_bank_survives_ack_snapshot_and_another_workbook(api, bank):
    grant = issue(api)
    operation = {"operation_id": str(uuid4()), "kind": "create", "fields": {**create_fields(), "excel_bank": bank}}
    result = sync(api, grant, operation)[0]
    assert result["status"] == "applied"
    assert result["record"]["fields"]["excel_bank"] == bank
    assert result["record"]["metadata"]["source_type"] == "MANUAL"
    assert sync(api, grant, operation)[0] == result
    other = issue(api)
    received = snapshot(api, other)["records"][0]
    assert received["id"] == result["record"]["id"]
    assert received["fields"]["excel_bank"] == bank
    cleared = sync(api, other, update(received, excel_bank=None))[0]
    assert cleared["status"] == "applied"
    assert cleared["record"]["fields"]["excel_bank"] is None
    assert snapshot(api, grant)["records"][0]["fields"]["excel_bank"] is None
    assert api.db.query(Transaction).count() == 1


def test_excel_bank_does_not_bypass_manual_source_authorization(api):
    operator(api, sources=(10,))
    grant = issue(api)
    operation = {"operation_id": str(uuid4()), "kind": "create", "fields": {**create_fields(), "excel_bank": "HUMO Card"}}
    assert sync(api, grant, operation)[0]["status"] == "forbidden"
    assert api.db.query(Transaction).count() == 0


@pytest.mark.parametrize("confirmed", [None, False, True])
def test_independent_p2p_confirmation_round_trip_and_clear(api, confirmed):
    grant = issue(api)
    operation = {"operation_id": str(uuid4()), "kind": "create", "fields": {**create_fields(), "p2p_confirmed": confirmed}}
    result = sync(api, grant, operation)[0]
    assert result["status"] == "applied"
    assert result["record"]["fields"]["p2p_confirmed"] is confirmed
    assert result["record"]["fields"]["is_p2p"] is True
    assert sync(api, grant, operation)[0] == result
    received = snapshot(api, grant)["records"][0]
    assert received["fields"]["p2p_confirmed"] is confirmed
    changed = sync(api, grant, update(received, is_p2p=False))[0]
    assert changed["record"]["fields"]["p2p_confirmed"] is confirmed
    cleared = sync(api, grant, update(changed["record"], p2p_confirmed=None))[0]
    assert cleared["status"] == ("noop" if confirmed is None else "applied")
    assert snapshot(api, grant)["records"][0]["fields"]["p2p_confirmed"] is None


def test_p2p_confirmation_transport_does_not_coerce_numeric_one(api):
    grant = issue(api)
    operation = {"operation_id": str(uuid4()), "kind": "create", "fields": {**create_fields(), "p2p_confirmed": 1}}
    result = sync(api, grant, operation)[0]
    assert result["status"] == "invalid" and result["error"] == "invalid_p2p_confirmed"
    assert api.db.query(Transaction).count() == 0


def test_conflicts_are_atomic_and_new_operation_can_resolve_them(api):
    tx = transaction(api, annotation_details="Original")
    grant = issue(api)
    before = snapshot(api, grant)["records"][0]
    tx.annotation_details = "Other editor"
    api.db.commit()
    patch = update(before, annotation_details="Our note", operator_raw="Our merchant")
    result = sync(api, grant, patch)[0]
    assert result["status"] == "conflict"
    assert result["conflicts"]["annotation_details"] == {"base": "Original", "current": "Other editor", "proposed": "Our note"}
    assert api.db.get(Transaction, tx.id).operator_raw == "Synthetic merchant"
    assert sync(api, grant, {**patch, "fields": {"annotation_details": "Reused id"}})[0]["status"] == "operation_id_reused"
    assert sync(api, grant, update(result["record"], annotation_details="Our note"))[0]["status"] == "applied"
    stale_delete = sync(api, grant, delete(before))[0]
    assert stale_delete["status"] == "conflict"
    assert api.db.query(Transaction).count() == 1


def test_batch_validation_does_not_corrupt_valid_neighbor(api):
    first, second = transaction(api), transaction(api)
    grant = issue(api)
    records = snapshot(api, grant)["records"]
    results = sync(api, grant, update(records[0], operator_raw="Accepted"), update(records[1], amount="NaN", operator_raw="Rejected"))
    assert [r["status"] for r in results] == ["applied", "invalid"]
    assert api.db.get(Transaction, first.id).operator_raw == "Accepted"
    assert api.db.get(Transaction, second.id).operator_raw == "Synthetic merchant"


def test_reconnecting_same_workbook_does_not_duplicate_an_unacknowledged_create(api):
    original = issue(api)
    operation = {"operation_id": str(uuid4()), "kind": "create", "fields": create_fields()}
    first = sync(api, original, operation)[0]
    # Client lost this acknowledgement, then replaced its expiring/revoked key.
    api.client.delete(f"/api/excel/grants/{original.data['grant']['id']}")
    replacement = issue(api, name="Renewed connection")
    replacement.workbook = original.workbook
    replacement.headers["X-Excel-Workbook"] = original.workbook
    repeated = sync(api, replacement, operation)[0]
    assert repeated["status"] == "applied"
    assert repeated["record"]["id"] == first["record"]["id"]
    assert api.db.query(Transaction).count() == 1


def test_source_scope_user_dates_and_locks_apply_to_reads_and_writes(api):
    operator(api)
    api.user.forbidden_periods = json.dumps([{"from": "2026-08-01", "to": "2026-08-31"}])
    api.db.commit()
    allowed = transaction(api)
    hidden = [transaction(api, source=20), transaction(api, when="2025-09-01T12:30:00"),
              transaction(api, when="2026-08-15T12:30:00"), transaction(api, when="2026-09-02T12:30:00")]
    lock(api)
    grant = issue(api)
    assert [r["id"] for r in snapshot(api, grant)["records"]] == [str(allowed.id)]
    results = sync(api, grant, *(update(serialize_record(row), operator_raw="Forbidden") for row in hidden))
    assert all(r["status"] == "forbidden" and "record" not in r and "conflicts" not in r for r in results)
    assert all(api.db.get(Transaction, row.id).operator_raw == "Synthetic merchant" for row in hidden)


@pytest.mark.parametrize("proposed", ["2027-01-01T00:00:00", "2026-08-15T00:00:00", "2026-09-02T00:00:00"])
def test_date_edit_cannot_move_row_outside_current_access(api, proposed):
    operator(api)
    api.user.forbidden_periods = json.dumps([{"from": "2026-08-01", "to": "2026-08-31"}])
    api.db.commit()
    tx = transaction(api)
    lock(api)
    grant = issue(api)
    result = sync(api, grant, update(serialize_record(tx), transaction_date=proposed, operator_raw="Must not apply"))[0]
    assert result["status"] == "forbidden" and "record" not in result
    assert api.db.get(Transaction, tx.id).operator_raw == "Synthetic merchant"


def test_manual_create_requires_source_zero_and_cannot_forge_source(api):
    operator(api, sources=(10,))
    grant = issue(api)
    operation = {"operation_id": str(uuid4()), "kind": "create", "fields": create_fields()}
    assert sync(api, grant, operation)[0]["status"] == "forbidden"
    operation = {"operation_id": str(uuid4()), "kind": "create", "fields": {**create_fields(), "source_chat_id": 10}}
    assert sync(api, grant, operation)[0]["status"] == "invalid"
    assert api.db.query(Transaction).count() == 0


@pytest.mark.parametrize("field,value", [
    ("is_active", False), ("permissions_version", 2),
    ("access_until", datetime(2020, 1, 1)), ("locked_until", datetime(2100, 1, 1)),
])
def test_grants_recheck_current_user_state_on_every_request(api, field, value):
    operator(api)
    grant = issue(api)
    snapshot(api, grant)
    setattr(api.user, field, value)
    api.db.commit()
    assert api.client.get("/api/excel/v1/snapshot", headers=grant.headers).status_code == 403
    response = api.client.post("/api/excel/v1/sync", headers=grant.headers,
                               json={"workbook_id": grant.workbook, "operations": [{"kind": "create"}]})
    assert response.status_code == 403


@pytest.mark.parametrize("field,value", [("is_active", False), ("years_json", "[2025]"), ("allow_transactions", False)])
def test_changed_scope_requires_a_new_grant(api, field, value):
    folder = operator(api)
    grant = issue(api)
    setattr(folder, field, value)
    api.db.commit()
    assert api.client.get("/api/excel/v1/snapshot", headers=grant.headers).status_code == 403


def test_scope_changed_since_dashboard_unlock_cannot_issue_a_grant(api):
    folder = operator(api)
    folder.years_json = "[2025]"
    api.db.commit()
    response = api.client.post("/api/excel/grants", json={"name": "Stale scope"})
    assert response.status_code == 403
    assert api.db.query(ExcelSyncGrant).count() == 0


def test_scope_date_bounds_restrict_snapshot_and_proposed_edits(api):
    folder = operator(api)
    folder.period_start = datetime(2026, 9, 1, 12)
    folder.period_end = datetime(2026, 9, 1, 13)
    api.db.commit()
    api.scope = routes._current_scope_definition(api.db, folder.id)
    inside = transaction(api)
    transaction(api, when="2026-09-01T11:59:59")
    transaction(api, when="2026-09-01T13:00:01")
    grant = issue(api)
    records = snapshot(api, grant)["records"]
    assert [record["id"] for record in records] == [str(inside.id)]
    assert sync(api, grant, update(records[0], transaction_date="2026-09-01T13:00:01"))[0]["status"] == "forbidden"


def test_folder_and_granted_scopes_intersect_without_widening_disjoint_years(api):
    operator(api)
    other_folder = make_scope(api, name="2025 folder", years=(2025,))
    grant = issue(api)
    api.user.allowed_folders = json.dumps([other_folder.id])
    api.db.commit()
    transaction(api)
    transaction(api, when="2025-09-01T12:30:00")
    assert snapshot(api, grant)["records"] == []


def test_current_source_changes_and_replay_do_not_reveal_previous_record(api):
    operator(api)
    tx = transaction(api)
    grant = issue(api)
    operation = update(serialize_record(tx), operator_raw="Previously allowed")
    assert sync(api, grant, operation)[0]["status"] == "applied"
    api.user.allowed_sources = "[]"
    api.db.commit()
    assert snapshot(api, grant)["records"] == []
    repeated = sync(api, grant, operation)[0]
    assert repeated["status"] == "forbidden" and "record" not in repeated


def test_lost_create_ack_retains_applied_marker_while_access_is_restricted(api):
    operator(api)
    grant = issue(api)
    operation = {"operation_id": str(uuid4()), "kind": "create", "fields": create_fields()}
    original = sync(api, grant, operation)[0]
    assert original["status"] == "applied"
    # The workbook did not receive this response; manual-source access changes
    # before its retry. It must not turn this already-created receipt into a draft.
    api.user.allowed_sources = "[10]"
    api.db.commit()
    assert snapshot(api, grant)["records"] == []
    repeated = sync(api, grant, operation)[0]
    assert repeated == {
        "status": "forbidden", "operation_id": operation["operation_id"],
        "replayed": True, "original_status": "applied", "applied": True,
    }
    assert api.db.query(Transaction).count() == 1
    api.user.allowed_sources = "[0, 10]"
    api.db.commit()
    restored = sync(api, grant, operation)[0]
    assert restored["status"] == "applied"
    assert restored["record"]["id"] == original["record"]["id"]
    assert api.db.query(Transaction).count() == 1


def test_new_period_lock_blocks_cached_operation_response(api):
    operator(api)
    tx = transaction(api)
    grant = issue(api)
    operation = update(serialize_record(tx), operator_raw="Previously allowed")
    assert sync(api, grant, operation)[0]["status"] == "applied"
    period = lock(api, day="2026-09-01")
    assert snapshot(api, grant)["records"] == []
    repeated = sync(api, grant, operation)[0]
    assert repeated["status"] == "forbidden" and "record" not in repeated
    period.excluded_user_ids = [api.user.id]
    api.db.commit()
    assert len(snapshot(api, grant)["records"]) == 1
    assert sync(api, grant, update(serialize_record(tx), operator_raw="Excluded user can edit"))[0]["status"] == "applied"


def test_new_otp_scope_requirement_invalidates_unscoped_operator_grant(api, monkeypatch):
    operator(api)
    api.scope = None
    monkeypatch.setattr(dependencies, "is_otp_enabled", lambda _db: False)
    grant = issue(api)
    snapshot(api, grant)
    monkeypatch.setattr(dependencies, "is_otp_enabled", lambda _db: True)
    monkeypatch.setattr(dependencies, "has_active_scopes", lambda _db: True)
    assert api.client.get("/api/excel/v1/snapshot", headers=grant.headers).status_code == 403


def test_protocol_still_honors_a_blocked_system(api, monkeypatch):
    grant = issue(api)
    monkeypatch.setattr(routes, "system_access_enforced", lambda: True)
    monkeypatch.setattr(routes, "is_system_blocked", lambda: (True, "test blocked configuration"))
    assert api.client.get("/api/excel/v1/snapshot", headers=grant.headers).status_code == 503


@pytest.mark.parametrize("middleware_name", ["SystemAccessMiddleware", "LaunchSessionMiddleware"])
def test_middleware_exempts_only_exact_protocol_paths(api, monkeypatch, middleware_name):
    # Import classes only; the application's real lifespan is never entered.
    from api import main
    from database import connection

    grant = issue(api)
    monkeypatch.setattr(main, "is_internal_request", lambda _request: False)
    monkeypatch.setattr(main, "system_access_enforced", lambda: True)
    monkeypatch.setattr(main, "is_system_blocked", lambda: (False, ""))
    monkeypatch.setattr(main, "verify_system_access_token", lambda _token: (False, None, "test denied"))
    monkeypatch.setattr(main, "_write_system_access_audit", lambda **_kwargs: None)
    monkeypatch.setattr(main, "_get_launch_gate_enabled_runtime", lambda: True)

    @contextmanager
    def local_db():
        yield api.db

    monkeypatch.setattr(connection, "get_db", local_db)
    api.db.add(AppLaunchConfig(id=1, password_hash="unused", salt="unused"))
    api.db.commit()
    protected = FastAPI()
    protected.include_router(routes.router)
    protected.dependency_overrides.update(api.app.dependency_overrides)
    protected.add_middleware(getattr(main, middleware_name))
    with TestClient(protected) as client:
        assert client.get("/api/excel/v1/snapshot", headers=grant.headers).status_code == 200
        assert client.get("/api/excel/v1/snapshot").status_code == 401
        response = client.post("/api/excel/v1/sync", headers=grant.headers,
                               json={"workbook_id": grant.workbook, "operations": [{"kind": "invalid"}]})
        assert response.status_code == 200
        response = client.post("/api/excel/v1/device", headers=grant.headers, json={"device_name": "PC"})
        assert response.status_code == 400 and response.json()["detail"] == "invalid_device_id"
        response = client.post("/api/excel/v1/activate", json={"device_name": "PC", "password": "x"},
                               headers={"X-Excel-Workbook": grant.workbook, "X-Excel-Device": "PC-0000-A"})
        assert response.json()["detail"] == "excel_activation_unavailable"
        for path in ("/api/excel/grants", "/api/excel/workbook", "/api/excel/v1/sync/extra", "/api/excel/v1/snapshot-extra"):
            assert client.get(path, headers=grant.headers).status_code == 403
        assert client.post("/api/excel/grants", headers=grant.headers, json={"name": "Forbidden"}).status_code == 403


PC_A = "4C4C4544-0042-3510-8052-B4C04F4E4A32:PC-A"
PC_B = "03000200-0400-0500-0006-000700080009:PC-B"


def protect(api, password="correct horse 42"):
    grant = issue(api)
    snapshot(api, grant)
    response = api.client.put("/api/excel/activations", json={"workbook_id": grant.workbook, "password": password})
    assert response.status_code == 200, response.text
    return grant


def activate(api, workbook, device=PC_A, password="correct horse 42", name="Бухгалтерия"):
    return api.client.post("/api/excel/v1/activate", json={"device_name": name, "password": password},
                           headers={"X-Excel-Workbook": workbook, "X-Excel-Device": device})


def device_headers(workbook, token, device=PC_A):
    return {"Authorization": f"Bearer {token}", "X-Excel-Workbook": workbook, "X-Excel-Device": device}


def test_protocol_paths_exempted_by_middleware_match_router():
    from api import main
    assert main.EXCEL_PROTOCOL_PATHS == routes.GRANT_ENDPOINTS


def test_activation_password_is_hashed_owned_and_only_for_known_workbooks(api):
    unknown = api.client.put("/api/excel/activations", json={"workbook_id": str(uuid4()), "password": "long enough"})
    assert unknown.status_code == 404
    short = api.client.put("/api/excel/activations", json={"workbook_id": str(uuid4()), "password": "short"})
    assert short.status_code == 422
    grant = protect(api)
    row = api.db.query(ExcelSyncActivation).one()
    assert row.workbook_id == grant.workbook and row.user_id == api.user.id
    assert "correct horse" not in row.password_hash + row.salt
    listed = api.client.get("/api/excel/activations")
    assert listed.headers["cache-control"] == "no-store"
    assert [item["workbook_id"] for item in listed.json()["activations"]] == [grant.workbook]
    assert "correct horse" not in listed.text and row.password_hash not in listed.text
    audits = api.db.query(AccessAuditLog).filter_by(action="excel_activation_password_set").all()
    assert len(audits) == 1 and "correct horse" not in audits[0].details_json

    other = User(username="excel-api-other", password_hash="unused", salt="unused", role="admin")
    api.db.add(other)
    api.db.commit()
    api.user = other
    assert api.client.get("/api/excel/activations").json() == {"activations": []}
    taken = api.client.put("/api/excel/activations", json={"workbook_id": grant.workbook, "password": "hijack it"})
    assert taken.status_code == 404
    assert api.client.delete(f"/api/excel/activations/{grant.workbook}").status_code == 404


def test_activation_issues_a_grant_bound_to_that_computer(api):
    book = protect(api).workbook
    assert activate(api, book, password="wrong").status_code == 401
    response = activate(api, book)
    assert response.status_code == 201, response.text
    assert response.headers["cache-control"] == "no-store"
    data = response.json()
    assert data["grant"]["device_bound"] and data["grant"]["device_name"] == "Бухгалтерия"
    assert data["grant"]["workbook_id"] == book and data["grant"]["expires_at"] is None
    row = api.db.query(ExcelSyncGrant).filter(ExcelSyncGrant.device_id.isnot(None)).one()
    assert row.device_id == PC_A and row.token_hash == hashlib.sha256(data["token"].encode()).hexdigest()
    audit = api.db.query(AccessAuditLog).filter_by(action="excel_device_activated").one()
    assert data["token"] not in audit.details_json

    headers = device_headers(book, data["token"])
    assert api.client.get("/api/excel/v1/snapshot", headers=headers).status_code == 200
    # The same key copied to another PC, or sent without the device, is refused.
    assert api.client.get("/api/excel/v1/snapshot", headers=device_headers(book, data["token"], PC_B)).status_code == 403
    no_device = {k: v for k, v in headers.items() if k != "X-Excel-Device"}
    assert api.client.get("/api/excel/v1/snapshot", headers=no_device).status_code == 403
    other_book = {**headers, "X-Excel-Workbook": str(uuid4())}
    assert api.client.get("/api/excel/v1/snapshot", headers=other_book).status_code == 403
    response = api.client.post("/api/excel/v1/sync", headers=device_headers(book, data["token"], PC_B),
                               json={"workbook_id": book, "operations": []})
    assert response.status_code == 403


def test_reactivating_a_computer_replaces_only_its_own_grant(api):
    book = protect(api).workbook
    first = activate(api, book).json()["token"]
    second = activate(api, book).json()["token"]
    other = activate(api, book, device=PC_B, name="Склад").json()["token"]
    assert api.client.get("/api/excel/v1/snapshot", headers=device_headers(book, first)).status_code == 401
    assert api.client.get("/api/excel/v1/snapshot", headers=device_headers(book, second)).status_code == 200
    assert api.client.get("/api/excel/v1/snapshot", headers=device_headers(book, other, PC_B)).status_code == 200
    live = api.db.query(ExcelSyncGrant).filter(ExcelSyncGrant.device_id.isnot(None),
                                                ExcelSyncGrant.revoked_at.is_(None)).all()
    assert sorted(row.device_name for row in live) == ["Бухгалтерия", "Склад"]


@pytest.mark.parametrize("device", [None, "", "short", "has space in-it", "x" * 129, "bad/slash-device"])
def test_activation_requires_a_device_identity(api, device):
    book = protect(api).workbook
    headers = {"X-Excel-Workbook": book}
    if device is not None:
        headers["X-Excel-Device"] = device
    response = api.client.post("/api/excel/v1/activate", headers=headers,
                               json={"device_name": "PC", "password": "correct horse 42"})
    assert response.status_code == 400
    assert api.db.query(ExcelSyncActivation).one().failed_attempts == 0


def test_activation_rejects_unknown_workbook_and_blank_name(api):
    book = protect(api).workbook
    assert activate(api, str(uuid4())).status_code == 403
    assert activate(api, "not-a-uuid").status_code == 400
    assert activate(api, book, name="   ").status_code == 422


def test_wrong_passwords_lock_activation_for_everyone(api, monkeypatch):
    book = protect(api).workbook
    for _ in range(routes.ACTIVATION_ATTEMPTS):
        assert activate(api, book, password="guess").status_code == 401
    locked = activate(api, book)
    assert locked.status_code == 429 and int(locked.headers["retry-after"]) > 0
    assert activate(api, book, device=PC_B).status_code == 429
    assert api.db.query(ExcelSyncGrant).filter(ExcelSyncGrant.device_id.isnot(None)).count() == 0
    failed = api.db.query(AccessAuditLog).filter_by(action="excel_activation_failed").count()
    assert failed == routes.ACTIVATION_ATTEMPTS
    listed = api.client.get("/api/excel/activations").json()["activations"][0]
    assert listed["locked_until"] is not None

    later = datetime.now(timezone.utc).replace(tzinfo=None) + routes.ACTIVATION_LOCKOUT + timedelta(seconds=1)
    monkeypatch.setattr(routes, "_now", lambda: later)
    assert activate(api, book).status_code == 201
    assert api.db.query(ExcelSyncActivation).one().failed_attempts == 0


def test_resetting_the_password_clears_the_lock(api):
    book = protect(api).workbook
    for _ in range(routes.ACTIVATION_ATTEMPTS):
        activate(api, book, password="guess")
    assert activate(api, book).status_code == 429
    api.client.put("/api/excel/activations", json={"workbook_id": book, "password": "a new password"})
    assert activate(api, book).status_code == 401
    assert activate(api, book, password="a new password").status_code == 201


@pytest.mark.parametrize("field,value,detail", [
    ("permissions_version", 2, "permissions_changed_reset_excel_password"),
    ("is_active", False, "inactive_user"),
])
def test_activation_rechecks_the_owner(api, field, value, detail):
    book = protect(api).workbook
    setattr(api.user, field, value)
    api.db.commit()
    response = activate(api, book)
    assert response.status_code == 403 and response.json()["detail"] == detail


def test_operator_activation_expires_and_keeps_the_captured_scope(api):
    operator(api)
    book = protect(api).workbook
    data = activate(api, book).json()
    assert data["grant"]["expires_at"] is not None
    row = api.db.query(ExcelSyncGrant).filter(ExcelSyncGrant.device_id.isnot(None)).one()
    assert row.scope == api.db.query(ExcelSyncActivation).one().scope


def test_removing_the_password_stops_new_computers_only(api):
    book = protect(api).workbook
    token = activate(api, book).json()["token"]
    assert api.client.delete(f"/api/excel/activations/{book}").json() == {"ok": True}
    assert activate(api, book, device=PC_B).status_code == 403
    assert api.client.get("/api/excel/v1/snapshot", headers=device_headers(book, token)).status_code == 200
    assert api.client.delete(f"/api/excel/activations/{book}").status_code == 404


def test_legacy_shared_grant_is_exchanged_for_a_device_grant(api):
    legacy = issue(api, expires_in_days=None)
    snapshot(api, legacy)

    def bind(headers):
        return api.client.post("/api/excel/v1/device", headers=headers, json={"device_name": "Old PC"})

    assert bind(legacy.headers).status_code == 400
    response = bind({**legacy.headers, "X-Excel-Device": PC_A})
    assert response.status_code == 201, response.text
    data = response.json()
    assert data["grant"]["device_bound"] and data["grant"]["workbook_id"] == legacy.workbook
    row = api.db.query(ExcelSyncGrant).filter(ExcelSyncGrant.device_id == PC_A).one()
    old = api.db.query(ExcelSyncGrant).filter(ExcelSyncGrant.device_id.is_(None)).one()
    assert row.permissions_version == old.permissions_version and row.expires_at == old.expires_at
    # Other computers still on the shared key keep working until the owner revokes it.
    assert api.client.get("/api/excel/v1/snapshot", headers=legacy.headers).status_code == 200
    bound = device_headers(legacy.workbook, data["token"])
    assert api.client.get("/api/excel/v1/snapshot", headers=bound).status_code == 200
    again = bind(bound)
    assert again.status_code == 409 and again.json()["detail"] == "excel_connection_already_device_bound"
    # A second exchange from the same PC replaces its previous device grant.
    replacement = bind({**legacy.headers, "X-Excel-Device": PC_A}).json()["token"]
    assert api.client.get("/api/excel/v1/snapshot", headers=bound).status_code == 401
    assert api.client.get("/api/excel/v1/snapshot",
                          headers=device_headers(legacy.workbook, replacement)).status_code == 200
    assert bind({"X-Excel-Device": PC_A, "X-Excel-Workbook": legacy.workbook}).status_code == 401


def test_device_grants_count_toward_the_connection_limit(api):
    book = protect(api).workbook
    api.db.add_all([ExcelSyncGrant(
        user_id=api.user.id, name=f"Book {i}", token_hash=hashlib.sha256(str(i).encode()).hexdigest(),
        permissions_version=1,
    ) for i in range(routes.MAX_ACTIVE_GRANTS - 1)])
    api.db.commit()
    assert activate(api, book).status_code == 409
    assert api.db.query(ExcelSyncGrant).filter(ExcelSyncGrant.device_id.isnot(None)).count() == 0
