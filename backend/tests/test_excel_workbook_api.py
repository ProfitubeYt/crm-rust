"""Workbooks the server prepares: POST /api/excel/workbooks and the file behind them.

The requests are the ones the desktop client sends (desktop/crates/tbs-core/src/
api/excel.rs) and the ones the workbook's own VBA sends once it is opened
(tools/excel_sync/ExcelSync.bas), with the address and identifiers read back
out of the downloaded file. Isolated SQLite, the real router and service and
the application's own error envelope. No Excel runs here and no VBA executes.
"""
from __future__ import annotations

import hashlib
import io
import json
import re
import warnings
import zipfile
from contextlib import contextmanager
from datetime import datetime
from types import SimpleNamespace
from typing import Optional
from urllib.parse import unquote
from uuid import UUID, uuid4

import openpyxl
import pytest
from fastapi import FastAPI, HTTPException
from fastapi.exceptions import RequestValidationError
from fastapi.testclient import TestClient
from sqlalchemy.exc import OperationalError
from starlette.exceptions import HTTPException as StarletteHTTPException

from api import dependencies
from api.routes import excel_sync as routes
from database.connection import get_db_session
from database.excel_sync_models import ExcelSyncActivation, ExcelSyncGrant
from database.models import AccessAuditLog, AppLaunchConfig, User
from services.access_control_service import hash_password, verify_password
from services.user_service import build_permissions_snapshot
from tests.test_excel_sync_api import operator, transaction

BACKEND = routes.workbook_builder.TEMPLATE_PATH.parents[1]
TEMPLATE = routes.workbook_builder.TEMPLATE_PATH.read_bytes()
SERVER = "https://sync.example.test"
PASSWORD = "correct horse 42"
PC_A, PC_B = "DESKTOP-ACCOUNTS:PC-A", "DESKTOP-STORE:PC-B"
PUBLIC_FIELDS = {"workbook_id", "name", "created_at", "updated_at", "locked_until"}
OMIT = object()


@pytest.fixture
def api(db_session, monkeypatch):
    from api import main

    monkeypatch.delenv("EXCEL_SYNC_PUBLIC_URL", raising=False)
    user = User(username="workbook-owner", password_hash="unused", salt="unused", role="admin")
    db_session.add(user)
    db_session.commit()
    state = SimpleNamespace(db=db_session, user=user, scope=None)

    def database():
        try:
            yield db_session
        except Exception:
            db_session.rollback()
            raise

    monkeypatch.setattr(routes, "system_access_enforced", lambda: False)
    for module in (routes, dependencies, routes.tx_api):
        monkeypatch.setattr(module, "get_config_scopes", lambda **_kwargs: [])
    app = FastAPI()
    app.include_router(routes.router)
    # The envelope of api.main: it is what the desktop client and the VBA parse.
    app.add_exception_handler(HTTPException, main.http_exception_handler)
    app.add_exception_handler(StarletteHTTPException, main.starlette_http_exception_handler)
    app.add_exception_handler(RequestValidationError, main.validation_exception_handler)
    app.dependency_overrides[get_db_session] = database
    app.dependency_overrides[routes._dashboard_user] = lambda: build_permissions_snapshot(state.user)
    app.dependency_overrides[dependencies.require_transactions_scope] = lambda: state.scope
    state.app = app
    with TestClient(app, base_url=SERVER) as client:
        state.client = client
        yield state


# --- the desktop client's calls ---------------------------------------------

def create(api, client=None, headers=None, **payload):
    body = {"name": "Чеки — бухгалтерия", "password": PASSWORD, "server_url": SERVER, **payload}
    return (client or api.client).post("/api/excel/workbooks", headers=headers,
                                       json={key: value for key, value in body.items() if value is not OMIT})


def made(api, **payload) -> dict:
    response = create(api, **payload)
    assert response.status_code == 201, response.text
    return response.json()["workbook"]


def download(api, workbook_id, client=None, **params):
    return (client or api.client).get(f"/api/excel/workbooks/{workbook_id}/file", params=params)


def code(response) -> Optional[str]:
    """The error code as error_from_body in desktop/crates/tbs-core/src/api/mod.rs finds it."""
    body = response.json()
    detail = body.get("detail")
    if isinstance(detail, dict):
        for key in ("error", "code"):
            if isinstance(detail.get(key), str):
                return detail[key]
    return body.get("error_code")


def attachment_name(response) -> str:
    header = response.headers["content-disposition"]
    assert header.startswith("attachment; ") and "\n" not in header and "\r" not in header
    extended = re.search(r"filename\*=utf-8''(\S+)$", header)
    return unquote(extended.group(1)) if extended else re.search(r'filename="([^"]*)"$', header).group(1)


def stored(api) -> int:
    return api.db.query(ExcelSyncActivation).count()


def another_user(api, name="workbook-other") -> User:
    user = User(username=name, password_hash="unused", salt="unused", role="admin")
    api.db.add(user)
    api.db.commit()
    return user


# --- the file, read the way Excel finds it ------------------------------------

def sync_cells(content: bytes) -> dict[str, object]:
    with warnings.catch_warnings():
        warnings.simplefilter("ignore")
        sheet = openpyxl.load_workbook(io.BytesIO(content), keep_vba=True)["_sync"]
    return {f"B{row}": sheet[f"B{row}"].value for row in range(1, 11)}


def parts(content: bytes) -> dict[str, bytes]:
    with zipfile.ZipFile(io.BytesIO(content)) as archive:
        assert archive.testzip() is None
        return {info.filename: archive.read(info) for info in archive.infolist()}


def configured(api, **payload) -> tuple[dict, dict]:
    """A new workbook and the cells of its downloaded file."""
    book = made(api, **payload)
    response = download(api, book["workbook_id"], server_url=SERVER)
    assert response.status_code == 200, response.text
    return book, sync_cells(response.content)


# --- the workbook's own calls (ExcelSync.bas), addressed from its cells --------

def activate(api, cells, password=PASSWORD, device=PC_A, name="Бухгалтерия"):
    return api.client.post(cells["B5"] + "/api/excel/v1/activate", json={"device_name": name, "password": password},
                           headers={"X-Excel-Workbook": cells["B1"], "X-Excel-Device": device})


def key_headers(cells, token, device=PC_A):
    return {"Authorization": f"Bearer {token}", "X-Excel-Workbook": cells["B1"], "X-Excel-Device": device}


def snapshot(api, cells, token, device=PC_A):
    return api.client.get(cells["B5"] + "/api/excel/v1/snapshot", headers=key_headers(cells, token, device))


# --- POST /api/excel/workbooks ---------------------------------------------------

def test_create_answers_with_the_record_the_desktop_client_reads(api):
    response = create(api, name="  Чеки —\tбухгалтерия \n")
    assert response.status_code == 201, response.text
    assert response.headers["cache-control"] == "no-store"
    body = response.json()
    assert set(body) == {"workbook", "server_url"} and body["server_url"] == SERVER
    workbook = body["workbook"]
    assert set(workbook) == PUBLIC_FIELDS
    assert str(UUID(workbook["workbook_id"])) == workbook["workbook_id"] and UUID(workbook["workbook_id"]).version == 4
    assert workbook["name"] == "Чеки — бухгалтерия"
    assert workbook["created_at"] == workbook["updated_at"] and workbook["created_at"].endswith("Z")
    assert workbook["locked_until"] is None

    row = api.db.query(ExcelSyncActivation).one()
    assert (row.workbook_id, row.user_id, row.name) == (workbook["workbook_id"], api.user.id, "Чеки — бухгалтерия")
    assert row.permissions_version == build_permissions_snapshot(api.user)["permissions_version"]
    assert row.scope is None and row.failed_attempts == 0 and row.locked_until is None
    # Only a salted hash of the password exists anywhere.
    assert verify_password(PASSWORD, row.password_hash, row.salt)
    assert PASSWORD not in row.password_hash + row.salt
    assert PASSWORD not in response.text and row.password_hash not in response.text and row.salt not in response.text
    audit = api.db.query(AccessAuditLog).one()
    assert (audit.action, audit.success, audit.user_id) == ("excel_workbook_created", True, api.user.id)
    assert json.loads(audit.details_json) == {"workbook_id": workbook["workbook_id"], "name": "Чеки — бухгалтерия"}
    # No computer holds a key yet: the workbook is its password record.
    assert api.db.query(ExcelSyncGrant).count() == 0


def test_a_made_workbook_is_listed_before_any_computer_opened_it(api):
    first, second = made(api, name="Касса"), made(api, name="Склад")
    assert first["workbook_id"] != second["workbook_id"]
    listed = api.client.get("/api/excel/activations")
    assert listed.headers["cache-control"] == "no-store"
    assert listed.json() == {"activations": [second, first]}
    assert api.client.get("/api/excel/grants").json() == {"grants": []}
    assert made(api, name="я" * routes.WORKBOOK_NAME_MAX)["name"] == "я" * routes.WORKBOOK_NAME_MAX


@pytest.mark.parametrize("payload,expected", [
    ({"name": ""}, "validation_error"),
    ({"name": " \t\n "}, "workbook_name_invalid"),
    ({"name": "x" * (routes.WORKBOOK_NAME_MAX + 1)}, "workbook_name_invalid"),
    ({"name": "x" * 201}, "validation_error"),
    ({"name": "Касса\x00"}, "workbook_name_invalid"),
    ({"name": "Кас\x7fса"}, "workbook_name_invalid"),
    ({"name": None}, "validation_error"),
    ({"name": 5}, "validation_error"),
    ({"name": OMIT}, "validation_error"),
    ({"password": "seven 7"}, "validation_error"),
    ({"password": "x" * 129}, "validation_error"),
    ({"password": " " * 8}, "password_required"),
    ({"password": None}, "validation_error"),
    ({"password": OMIT}, "validation_error"),
    ({"server_url": 5}, "validation_error"),
    ({"server_url": SERVER + "/" + "x" * 2048}, "validation_error"),
    # Nothing but a name, a password and an address can be supplied.
    ({"owner_id": 2}, "validation_error"),
    ({"user_id": 2}, "validation_error"),
    ({"workbook_id": str(uuid4())}, "validation_error"),
    ({"scope": {"scope_id": 0, "years": []}}, "validation_error"),
    ({"permissions_version": 1}, "validation_error"),
    ({"token": "excel_" + "t" * 43}, "validation_error"),
])
def test_a_request_that_is_not_a_name_and_a_password_stores_nothing(api, payload, expected):
    response = create(api, **payload)
    assert response.status_code == 422, response.text
    assert code(response) == expected
    assert PASSWORD not in response.text
    assert stored(api) == 0 and api.db.query(AccessAuditLog).count() == 0


@pytest.mark.parametrize("field", ["name", "password"])
def test_text_that_cannot_be_stored_is_refused_not_crashed_on(api, field):
    body = {"name": "Касса", "password": PASSWORD, "server_url": SERVER, field: "LONE-SURROGATE-HERE"}
    raw = json.dumps(body).replace("LONE-SURROGATE-HERE", "\\ud800 padding")
    response = api.client.post("/api/excel/workbooks", content=raw, headers={"Content-Type": "application/json"})
    assert response.status_code == 422, response.text
    assert stored(api) == 0


# --- the address written into the file -----------------------------------------

@pytest.mark.parametrize("given", [SERVER, SERVER + "/", "HTTPS://SYNC.EXAMPLE.TEST", SERVER + ":443",
                                   SERVER + ":0443/", OMIT, None, "", "  "])
def test_the_file_carries_the_one_spelling_of_the_address_this_server_was_reached_by(api, given):
    response = create(api, server_url=given)
    assert response.status_code == 201, response.text
    assert response.json()["server_url"] == SERVER
    book = response.json()["workbook"]["workbook_id"]
    params = {} if given in (OMIT, None) else {"server_url": given}
    assert sync_cells(download(api, book, **params).content)["B5"] == SERVER


MALFORMED = [
    SERVER + "/api", SERVER + "/?next=1", SERVER + "?next=1", SERVER + "#fragment", SERVER + "//",
    "https://user:secret@sync.example.test", "https://sync.example.test@evil.example",
    SERVER + ":0", SERVER + ":65536", SERVER + ":port", SERVER + ":443:443", SERVER + ":",
    "http://sync.example.test", "ftp://sync.example.test", "//sync.example.test", "sync.example.test",
    "https://", "https:///", "javascript:alert(1)", "file:///etc/passwd",
    SERVER + "\r\nX-Injected: 1", SERVER + "\n", " " + SERVER, SERVER + " ", "https://sync.example.test\x00",
    "https://sync.exаmple.test",  # a Cyrillic "а"
    "https://sync_example.test", "https://-sync.example.test", "https://sync..example.test", "https://[::1]",
    "https://" + "a" * 64 + ".example.test",
]
OTHER_SERVERS = [
    "https://evil.example", "https://evil.example:443", "https://sync.example.test.evil.example",
    "https://example.test", "https://sync.example.tes", "https://xsync.example.test",
    "http://localhost", "http://127.0.0.1:8000",
]


@pytest.mark.parametrize("given", MALFORMED + OTHER_SERVERS)
def test_an_address_that_is_not_this_servers_makes_no_workbook_and_no_file(api, given):
    refused = create(api, server_url=given)
    assert refused.status_code == 422, refused.text
    assert code(refused) == "invalid_server_url"
    assert stored(api) == 0 and api.db.query(AccessAuditLog).count() == 0
    # ... and an existing workbook's file is not handed out aimed anywhere else.
    book = made(api)
    refused = download(api, book["workbook_id"], server_url=given)
    assert refused.status_code == 422 and code(refused) == "invalid_server_url"
    assert not refused.content.startswith(b"PK")


def test_a_port_named_by_the_request_has_to_agree(api):
    with TestClient(api.app, base_url=SERVER + ":8443") as client:
        assert create(api, client, server_url=SERVER + ":8443").json()["server_url"] == SERVER + ":8443"
        assert create(api, client, server_url=OMIT).json()["server_url"] == SERVER + ":8443"
        for other in (SERVER, SERVER + ":9443"):
            refused = create(api, client, server_url=other)
            assert refused.status_code == 422 and code(refused) == "invalid_server_url"
    # A proxy may drop the port from Host; then only the host can be compared.
    assert create(api, server_url=SERVER + ":8443").json()["server_url"] == SERVER + ":8443"
    assert stored(api) == 3


def test_a_local_test_server_is_reached_over_plain_http(api):
    with TestClient(api.app, base_url="http://127.0.0.1:8765") as client:
        for given in ("http://127.0.0.1:8765", OMIT):
            response = create(api, client, server_url=given)
            assert response.status_code == 201, response.text
            assert response.json()["server_url"] == "http://127.0.0.1:8765"
        book = response.json()["workbook"]["workbook_id"]
        assert sync_cells(download(api, book, client).content)["B5"] == "http://127.0.0.1:8765"
        # The same machine under another name is another address to the workbook.
        refused = create(api, client, server_url="http://localhost:8765")
        assert refused.status_code == 422 and code(refused) == "invalid_server_url"


def test_plain_http_to_a_public_host_needs_the_address_from_the_client_or_the_configuration(api, monkeypatch):
    with TestClient(api.app, base_url="http://sync.example.test") as client:
        refused = create(api, client, server_url=OMIT)
        assert refused.status_code == 503 and code(refused) == "excel_public_url_not_configured"
        assert stored(api) == 0
        # TLS ends at a proxy: the client knows the https address it used.
        book = create(api, client, server_url=SERVER).json()["workbook"]["workbook_id"]
        assert download(api, book, client).status_code == 503
        assert sync_cells(download(api, book, client, server_url=SERVER).content)["B5"] == SERVER
        monkeypatch.setenv("EXCEL_SYNC_PUBLIC_URL", "https://public.example.test")
        assert sync_cells(download(api, book, client).content)["B5"] == "https://public.example.test"


def test_the_configured_public_address_wins_over_anything_the_client_sends(api, monkeypatch):
    monkeypatch.setenv("EXCEL_SYNC_PUBLIC_URL", " https://Public.Example.Test:443/ ")
    for given in (OMIT, SERVER, "https://evil.example", "not an address"):
        response = create(api, server_url=given)
        assert response.status_code == 201, response.text
        assert response.json()["server_url"] == "https://public.example.test"
    book = response.json()["workbook"]["workbook_id"]
    for params in ({}, {"server_url": "https://evil.example"}, {"server_url": SERVER}):
        assert sync_cells(download(api, book, **params).content)["B5"] == "https://public.example.test"


@pytest.mark.parametrize("value", ["http://public.example.test", "https://public.example.test/api",
                                   "public.example.test", "https://public.example.test:70000"])
def test_an_unusable_configured_address_refuses_every_workbook(api, monkeypatch, value):
    book = made(api)
    monkeypatch.setenv("EXCEL_SYNC_PUBLIC_URL", value)
    for refused in (create(api), create(api, server_url=OMIT), download(api, book["workbook_id"], server_url=SERVER)):
        assert refused.status_code == 503 and code(refused) == "excel_public_url_not_configured"
        assert value not in refused.text
    assert stored(api) == 1


@pytest.mark.parametrize("host", ["[::1", "sync.example.test:port", "sync.example.test:99999", "",
                                  "sync.example.test@evil.example", "evil.example"])
def test_a_host_header_that_is_not_this_address_is_refused_without_a_crash(api, host):
    refused = create(api, headers={"Host": host})
    assert refused.status_code == 422, refused.text
    assert code(refused) == "invalid_server_url"
    if host != "evil.example":  # that one is a usable address, only another one
        refused = create(api, headers={"Host": host}, server_url=OMIT)
        assert refused.status_code == 503 and code(refused) == "excel_public_url_not_configured"
    assert stored(api) == 0


# --- nothing is stored unless a file can be handed out ---------------------------

def without_macros() -> bytes:
    out = io.BytesIO()
    with zipfile.ZipFile(io.BytesIO(TEMPLATE)) as source, zipfile.ZipFile(out, "w", zipfile.ZIP_DEFLATED) as target:
        for info in source.infolist():
            if info.filename != "xl/vbaProject.bin":
                target.writestr(info, source.read(info))
    return out.getvalue()


BROKEN_TEMPLATES = {
    "missing": (None, "excel_workbook_not_built"),
    "empty": (b"", "excel_workbook_template_invalid"),
    "not a workbook": (b"PK\x03\x04 this is no archive", "excel_workbook_template_invalid"),
    "cut short": (TEMPLATE[:len(TEMPLATE) // 2], "excel_workbook_template_invalid"),
    "without macros": (without_macros(), "excel_workbook_template_invalid"),
    "the export layout, not the sync workbook": (
        (BACKEND / "services/excel_client_template.xlsx").read_bytes(), "excel_workbook_template_invalid"),
    "somebody's configured workbook": (
        routes.workbook_builder.configure_workbook(TEMPLATE, workbook_id=str(uuid4()), server_url=SERVER, owner_id=7),
        "excel_workbook_template_invalid"),
}


@pytest.mark.parametrize("case", list(BROKEN_TEMPLATES))
def test_a_template_that_cannot_be_configured_leaves_no_record(api, tmp_path, monkeypatch, case):
    content, expected = BROKEN_TEMPLATES[case]
    existing = made(api)
    audits = api.db.query(AccessAuditLog).count()
    path = tmp_path / "excel_sync_template.xlsm"
    if content is not None:
        path.write_bytes(content)
    monkeypatch.setattr(routes.workbook_builder, "TEMPLATE_PATH", path)

    refused = create(api, name="Вторая")
    assert refused.status_code == 503, refused.text
    assert code(refused) == expected
    assert str(tmp_path) not in refused.text and PASSWORD not in refused.text
    assert stored(api) == 1 and api.db.query(AccessAuditLog).count() == audits
    # The workbook made earlier keeps its record; only its file cannot be built now.
    refused = download(api, existing["workbook_id"])
    assert refused.status_code == 503 and code(refused) == expected
    assert api.client.get("/api/excel/activations").json() == {"activations": [existing]}


@pytest.mark.parametrize("failing", ["the audit entry", "the commit"])
def test_a_failure_while_storing_rolls_the_record_back(api, monkeypatch, failing):
    def broken(*_args, **_kwargs):
        raise OperationalError("INSERT", {}, Exception("the disk is full"))

    with monkeypatch.context() as patch:
        if failing == "the audit entry":
            patch.setattr(routes, "write_audit_log", broken)
        else:
            patch.setattr(api.db, "commit", broken)
        with pytest.raises(OperationalError):
            create(api)
    api.db.expire_all()
    assert stored(api) == 0 and api.db.query(AccessAuditLog).count() == 0
    # Nothing is left half-done: the next request starts clean.
    assert made(api)["name"] == "Чеки — бухгалтерия"
    assert stored(api) == 1 and api.db.query(AccessAuditLog).count() == 1


def test_the_number_of_workbooks_is_limited_for_each_owner(api):
    other = another_user(api)
    digest, salt = hash_password(PASSWORD)
    api.db.add_all([ExcelSyncActivation(workbook_id=str(uuid4()), user_id=user, password_hash=digest, salt=salt,
                                        permissions_version=1)
                    for user in [api.user.id] * (routes.MAX_WORKBOOKS - 1) + [other.id]])
    api.db.commit()
    last = made(api)
    refused = create(api)
    assert refused.status_code == 409 and code(refused) == "too_many_excel_workbooks"
    assert api.db.query(ExcelSyncActivation).filter_by(user_id=api.user.id).count() == routes.MAX_WORKBOOKS
    # Removing a password frees a place; another owner's count is their own.
    assert api.client.delete(f"/api/excel/activations/{last['workbook_id']}").json() == {"ok": True}
    made(api)
    assert create(api).status_code == 409
    api.user = other
    made(api)


def test_the_owners_scope_and_permissions_are_captured_as_they_are_now(api):
    folder = operator(api)
    made(api)
    row = api.db.query(ExcelSyncActivation).one()
    assert row.scope == routes._current_scope_definition(api.db, folder.id) and row.scope["scope_id"] == folder.id
    assert row.permissions_version == build_permissions_snapshot(api.user)["permissions_version"]


def test_a_scope_changed_since_it_was_unlocked_makes_no_workbook(api):
    folder = operator(api)
    folder.years_json = "[2025]"
    api.db.commit()
    refused = create(api)
    assert refused.status_code == 403 and refused.json()["detail"] == "scope_changed"
    assert stored(api) == 0


@pytest.mark.parametrize("field,value,detail", [
    ("is_active", False, "inactive_user"),
    ("locked_until", datetime(2100, 1, 1), "user_locked"),
    ("access_until", datetime(2020, 1, 1), "access_expired"),
])
def test_the_owner_is_checked_as_they_are_now(api, field, value, detail):
    operator(api)
    setattr(api.user, field, value)
    api.db.commit()
    refused = create(api)
    assert refused.status_code == 403 and refused.json()["detail"] == detail
    assert stored(api) == 0


# --- GET /api/excel/workbooks/{id}/file --------------------------------------------

def test_the_file_is_the_template_with_this_workbook_server_and_owner_and_nothing_else(api):
    book = made(api, name='Касса: 1/2? "март"')
    response = download(api, book["workbook_id"], server_url=SERVER)
    assert response.status_code == 200, response.text
    assert response.headers["content-type"] == routes.WORKBOOK_MEDIA_TYPE
    assert response.headers["cache-control"] == "no-store"
    assert attachment_name(response) == "Касса 1 2 март.xlsm"
    # What StartSync and ConfigureSharedProfile read; B2 to B4 and B10 are the template's own.
    assert sync_cells(response.content) == {
        "B1": book["workbook_id"], "B2": "1", "B3": "_transactions", "B4": "1", "B5": SERVER,
        "B6": None, "B7": str(api.user.id), "B8": "1", "B9": None, "B10": None}

    before, after = parts(TEMPLATE), parts(response.content)
    assert list(after) == list(before)
    changed = [name for name in before if after[name] != before[name]]
    assert len(changed) == 1 and re.fullmatch(r"xl/worksheets/sheet\d+\.xml", changed[0])
    assert after["xl/vbaProject.bin"] == before["xl/vbaProject.bin"]
    assert hashlib.sha256(after["xl/vbaProject.bin"]).digest() == hashlib.sha256(before["xl/vbaProject.bin"]).digest()

    # A key is issued, and still the file carries no key, password or hash.
    cells = sync_cells(response.content)
    token = activate(api, cells).json()["token"]
    row = api.db.query(ExcelSyncActivation).one()
    grant = api.db.query(ExcelSyncGrant).one()
    again = download(api, book["workbook_id"], server_url=SERVER)
    for secret in (PASSWORD, row.password_hash, row.salt, token, grant.token_hash):
        assert secret.encode() not in again.content
        assert all(secret.encode() not in part and secret.encode("utf-16-le") not in part
                   for part in parts(again.content).values())
    # The same bytes every time, for whichever computer needs the file next.
    assert again.content == response.content == download(api, book["workbook_id"]).content


@pytest.mark.parametrize("name,expected", [
    ("Касса", "Касса.xlsm"), ("books-2026_v1.2", "books-2026_v1.2.xlsm"), ("My receipts", "My receipts.xlsm"),
    ("..", routes.WORKBOOK_FILENAME + ".xlsm"), ("a\u202eb", "a b.xlsm"), ("отчёт.xlsm", "отчёт.xlsm.xlsm"),
    ('<>:"/\\|?*', routes.WORKBOOK_FILENAME + ".xlsm"),
])
def test_the_download_is_named_after_the_workbook_in_a_safe_header(api, name, expected):
    book = made(api, name=name)
    assert attachment_name(download(api, book["workbook_id"])) == expected


def test_another_user_cannot_download_repassword_or_remove_the_workbook(api):
    book = made(api)["workbook_id"]
    owner, api.user = api.user, another_user(api)
    for attempt in range(2):
        refused = download(api, book, server_url=SERVER)
        assert refused.status_code == 404 and code(refused) == "workbook_not_found"
        assert not refused.content.startswith(b"PK")
        assert api.client.get("/api/excel/activations").json() == {"activations": []}
        taken = api.client.put("/api/excel/activations", json={"workbook_id": book, "password": "hijack it now"})
        assert taken.status_code == 404
        assert api.client.delete(f"/api/excel/activations/{book}").status_code == 404
        if attempt == 0:
            # A key of their own, bound to that id, shows them their own receipts and gives no hold on the workbook.
            token = api.client.post("/api/excel/grants", json={"name": "Squatter"}).json()["token"]
            headers = {"Authorization": f"Bearer {token}", "X-Excel-Workbook": book}
            assert api.client.get("/api/excel/v1/snapshot", headers=headers).json()["user_id"] == str(api.user.id)
    api.user = owner
    row = api.db.query(ExcelSyncActivation).one()
    assert row.user_id == owner.id and verify_password(PASSWORD, row.password_hash, row.salt)
    assert sync_cells(download(api, book).content)["B7"] == str(owner.id)


def test_an_unknown_or_malformed_workbook_is_not_found(api):
    made(api)
    refused = download(api, str(uuid4()))
    assert refused.status_code == 404 and code(refused) == "workbook_not_found"
    assert download(api, "not-a-workbook-id").status_code == 422
    assert download(api, "..%2F..%2Fetc%2Fpasswd").status_code in (404, 422)


@pytest.mark.parametrize("gate", ["SystemAccessMiddleware", "LaunchSessionMiddleware"])
def test_the_application_gates_cover_making_and_fetching_but_not_the_workbooks_own_calls(api, monkeypatch, gate):
    # Import classes only; the application's real lifespan is never entered.
    from api import main
    from database import connection

    book, cells = configured(api)
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
    gated = FastAPI()
    gated.include_router(routes.router)
    gated.dependency_overrides.update(api.app.dependency_overrides)
    gated.add_middleware(getattr(main, gate))
    with TestClient(gated, base_url=SERVER) as client:
        assert create(api, client).status_code == 403
        assert download(api, book["workbook_id"], client).status_code == 403
        assert client.get("/api/excel/activations").status_code == 403
        assert stored(api) == 1
        # An opened workbook has no application login: its password is its way in.
        activated = client.post("/api/excel/v1/activate", json={"device_name": "ПК", "password": PASSWORD},
                                headers={"X-Excel-Workbook": cells["B1"], "X-Excel-Device": PC_A})
        assert activated.status_code == 201, activated.text
        key = key_headers(cells, activated.json()["token"])
        assert client.get("/api/excel/v1/snapshot", headers=key).status_code == 200


# --- workbooks from before this feature -----------------------------------------------

def test_a_workbook_connected_by_hand_keeps_working_and_can_be_fetched_configured(api):
    # The older flow: a key pasted into the workbook binds it on first use.
    token = api.client.post("/api/excel/grants", json={"name": "Моя книга Excel"}).json()["token"]
    book = str(uuid4())
    headers = {"Authorization": f"Bearer {token}", "X-Excel-Workbook": book}
    assert api.client.get("/api/excel/v1/snapshot", headers=headers).status_code == 200
    assert api.client.get("/api/excel/activations").json() == {"activations": []}

    response = download(api, book, server_url=SERVER)
    assert response.status_code == 200 and attachment_name(response) == routes.WORKBOOK_FILENAME + ".xlsm"
    cells = sync_cells(response.content)
    assert (cells["B1"], cells["B5"], cells["B7"], cells["B8"]) == (book, SERVER, str(api.user.id), "1")
    # Without a password no new computer gets in; the pasted key goes on working.
    assert activate(api, cells).json()["detail"] == "excel_activation_unavailable"
    protected = api.client.put("/api/excel/activations", json={"workbook_id": book, "password": PASSWORD})
    assert protected.status_code == 200
    assert set(protected.json()["activation"]) == PUBLIC_FIELDS and protected.json()["activation"]["name"] is None
    assert activate(api, cells).status_code == 201
    assert api.client.get("/api/excel/v1/snapshot", headers=headers).status_code == 200

    api.user = another_user(api)
    assert download(api, book).status_code == 404


def test_a_password_record_from_before_names_is_listed_and_still_activates(api):
    # A row as migration 0031 left it: no name.
    digest, salt = hash_password(PASSWORD)
    book = str(uuid4())
    api.db.add(ExcelSyncActivation(workbook_id=book, user_id=api.user.id, password_hash=digest, salt=salt,
                                   permissions_version=1, scope=None))
    api.db.commit()
    listed = api.client.get("/api/excel/activations").json()["activations"]
    assert [(item["workbook_id"], item["name"]) for item in listed] == [(book, None)]
    assert set(listed[0]) == PUBLIC_FIELDS
    response = download(api, book)
    assert attachment_name(response) == routes.WORKBOOK_FILENAME + ".xlsm"
    cells = sync_cells(response.content)
    token = activate(api, cells).json()["token"]
    assert snapshot(api, cells, token).json()["user_id"] == cells["B7"]


# --- the opened workbook: activation, receipts, revocation ----------------------------

def test_a_new_workbook_is_activated_by_its_password_and_shows_only_permitted_receipts(api):
    operator(api, sources=(0, 10))
    permitted = transaction(api, source=10)
    transaction(api, source=99)                               # a source this owner may not see
    transaction(api, source=10, when="2025-09-01T12:30:00")  # outside the folder that was unlocked
    book, cells = configured(api)
    # StartSync takes the id; ConfigureSharedProfile the address, the owner and shared mode.
    assert (cells["B1"], cells["B5"], cells["B7"], cells["B8"]) == (book["workbook_id"], SERVER, str(api.user.id), "1")

    wrong = activate(api, cells, password="not the password")
    assert wrong.status_code == 401 and wrong.json()["detail"] == "excel_activation_password_invalid"
    assert api.db.query(ExcelSyncGrant).count() == 0
    without_key = api.client.get(cells["B5"] + "/api/excel/v1/snapshot",
                                 headers={"X-Excel-Workbook": cells["B1"], "X-Excel-Device": PC_A})
    assert without_key.status_code == 401 and "records" not in without_key.text

    activated = activate(api, cells)
    assert activated.status_code == 201, activated.text
    assert activated.headers["cache-control"] == "no-store"
    data = activated.json()
    token = data["token"]
    assert token.startswith("excel_") and 40 <= len(token) <= 200  # what ActivateWithPassword accepts as a key
    assert data["grant"]["workbook_id"] == book["workbook_id"] and data["grant"]["device_bound"]
    assert data["grant"]["device_name"] == "Бухгалтерия" and data["grant"]["expires_at"] is not None
    grant = api.db.query(ExcelSyncGrant).one()
    assert grant.token_hash == hashlib.sha256(token.encode()).hexdigest() and grant.device_id == PC_A
    assert grant.scope == api.db.query(ExcelSyncActivation).one().scope

    received = snapshot(api, cells, token)
    assert received.status_code == 200, received.text
    body = received.json()
    # CheckIdentity: the owner the server reports is the owner written into the file.
    assert body["user_id"] == cells["B7"] and body["workbook_id"] == cells["B1"] and body["complete"] is True
    assert [record["id"] for record in body["records"]] == [str(permitted.id)]

    # The key belongs to this computer and this workbook.
    assert snapshot(api, cells, token, device=PC_B).json()["detail"] == "excel_connection_belongs_to_another_device"
    elsewhere = {**key_headers(cells, token), "X-Excel-Workbook": str(uuid4())}
    assert api.client.get("/api/excel/v1/snapshot", headers=elsewhere).status_code == 403
    rebind = api.client.post("/api/excel/v1/device", headers=key_headers(cells, token), json={"device_name": "ПК"})
    assert rebind.status_code == 409 and rebind.json()["detail"] == "excel_connection_already_device_bound"

    listed = api.client.get("/api/excel/grants")
    assert [(item["id"], item["workbook_id"], item["device_bound"], item["revoked_at"])
            for item in listed.json()["grants"]] == [(data["grant"]["id"], book["workbook_id"], True, None)]
    assert token not in listed.text and grant.token_hash not in listed.text

    # The owner disconnects the computer: its key stops working at once.
    assert api.client.delete(f"/api/excel/grants/{data['grant']['id']}").json() == {"ok": True}
    revoked = snapshot(api, cells, token)
    assert revoked.status_code == 401 and revoked.json()["detail"] == "excel_connection_expired_or_revoked"
    write = api.client.post(cells["B5"] + "/api/excel/v1/sync", headers=key_headers(cells, token),
                            json={"workbook_id": cells["B1"], "operations": [{"kind": "create"}]})
    assert write.status_code == 401
    assert api.db.get(type(permitted), permitted.id).operator_raw == "Synthetic merchant"
    # The password admits the computer again under a new key; the revoked one stays dead.
    fresh = activate(api, cells).json()["token"]
    assert fresh != token and snapshot(api, cells, fresh).status_code == 200
    assert snapshot(api, cells, token).status_code == 401


def test_wrong_passwords_lock_a_new_workbook_and_setting_a_password_unlocks_it(api):
    book, cells = configured(api)
    for _ in range(routes.ACTIVATION_ATTEMPTS):
        assert activate(api, cells, password="a guess").status_code == 401
    locked = activate(api, cells)  # the right password, too late
    assert locked.status_code == 429 and locked.json()["detail"] == "excel_activation_locked"
    assert activate(api, cells, device=PC_B).status_code == 429
    assert api.db.query(ExcelSyncGrant).count() == 0
    listed = api.client.get("/api/excel/activations").json()["activations"]
    assert [(item["name"], item["locked_until"] is not None) for item in listed] == [(book["name"], True)]

    changed = api.client.put("/api/excel/activations",
                             json={"workbook_id": book["workbook_id"], "password": "a new password"})
    assert changed.status_code == 200 and changed.headers["cache-control"] == "no-store"
    activation = changed.json()["activation"]
    assert set(activation) == PUBLIC_FIELDS and "a new password" not in changed.text
    assert (activation["workbook_id"], activation["name"], activation["created_at"], activation["locked_until"]) == (
        book["workbook_id"], book["name"], book["created_at"], None)
    assert activation["updated_at"] > book["updated_at"]
    assert activate(api, cells).status_code == 401
    assert activate(api, cells, password="a new password").status_code == 201


def test_a_new_password_keeps_activated_computers_and_admits_others_only_with_it(api):
    book, cells = configured(api)
    token = activate(api, cells).json()["token"]
    assert api.client.put("/api/excel/activations",
                          json={"workbook_id": book["workbook_id"], "password": "a new password"}).status_code == 200
    assert snapshot(api, cells, token).status_code == 200
    assert activate(api, cells, device=PC_B).status_code == 401
    assert activate(api, cells, device=PC_B, password="a new password").status_code == 201
    # The file never changes with the password: there is none in it.
    assert sync_cells(download(api, book["workbook_id"]).content) == cells


def test_changed_permissions_need_the_password_set_again(api):
    folder = operator(api)
    book, cells = configured(api)
    token = activate(api, cells).json()["token"]
    api.user.permissions_version = 2
    api.db.commit()
    refused = activate(api, cells, device=PC_B)
    assert refused.status_code == 403 and refused.json()["detail"] == "permissions_changed_reset_excel_password"
    assert snapshot(api, cells, token).json()["detail"] == "permissions_changed_reconnect_excel"
    payload = {"workbook_id": book["workbook_id"], "password": PASSWORD}
    assert api.client.put("/api/excel/activations", json=payload).status_code == 200
    assert activate(api, cells, device=PC_B).status_code == 201

    folder.years_json = "[2025]"
    api.db.commit()
    refused = activate(api, cells, device=PC_A)
    assert refused.status_code == 403 and refused.json()["detail"] == "scope_changed_reset_excel_password"
    # A scope that is no longer the one unlocked cannot be captured either.
    assert api.client.put("/api/excel/activations", json=payload).status_code == 403
    api.scope = routes._current_scope_definition(api.db, folder.id)
    assert api.client.put("/api/excel/activations", json=payload).status_code == 200
    assert activate(api, cells, device=PC_A).status_code == 201


def test_a_computer_replaces_its_own_key_and_the_limit_counts_computers(api):
    _book, cells = configured(api)
    first = activate(api, cells).json()["token"]
    second = activate(api, cells).json()["token"]
    assert snapshot(api, cells, first).status_code == 401 and snapshot(api, cells, second).status_code == 200
    api.db.add_all([ExcelSyncGrant(user_id=api.user.id, name=f"Book {index}", permissions_version=1,
                                   token_hash=hashlib.sha256(str(index).encode()).hexdigest())
                    for index in range(routes.MAX_ACTIVE_GRANTS - 1)])
    api.db.commit()
    full = activate(api, cells, device=PC_B)
    assert full.status_code == 409 and full.json()["detail"] == "too_many_excel_connections_revoke_unused"
    # The refused computer cost nobody a key, and a known computer still renews its own.
    assert snapshot(api, cells, second).status_code == 200
    third = activate(api, cells).json()["token"]
    assert snapshot(api, cells, second).status_code == 401 and snapshot(api, cells, third).status_code == 200


def test_removing_the_password_ends_a_workbook_nobody_opened_and_spares_activated_computers(api):
    unopened, unopened_cells = configured(api, name="Не открыта")
    assert api.client.delete(f"/api/excel/activations/{unopened['workbook_id']}").json() == {"ok": True}
    assert api.client.get("/api/excel/activations").json() == {"activations": []}
    assert download(api, unopened["workbook_id"]).status_code == 404
    assert activate(api, unopened_cells).json()["detail"] == "excel_activation_unavailable"

    opened, cells = configured(api, name="Открыта")
    token = activate(api, cells).json()["token"]
    assert api.client.delete(f"/api/excel/activations/{opened['workbook_id']}").json() == {"ok": True}
    assert snapshot(api, cells, token).status_code == 200
    assert activate(api, cells, device=PC_B).status_code == 403
    # Its computer still ties it to the owner, who can fetch the file and protect it again.
    assert sync_cells(download(api, opened["workbook_id"], server_url=SERVER).content) == cells
    again = api.client.put("/api/excel/activations", json={"workbook_id": opened["workbook_id"], "password": PASSWORD})
    assert again.status_code == 200 and again.json()["activation"]["name"] is None
    assert activate(api, cells, device=PC_B).status_code == 201
