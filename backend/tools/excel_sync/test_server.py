"""Loopback-only synthetic API for native Excel/WinHTTP verification.

Run directly with the backend Python interpreter, for example::

    python tools/excel_sync/test_server.py --state-dir C:/Dev/tmp/excel-e2e-UUID

The database is always fresh, in-memory SQLite. No application lifespan, dotenv,
background workers, external configuration or notifications are started. A test
grant is written to credentials.json in the supplied empty directory; never use
this helper as the application server. Stop its process after the native test.

Control endpoints (only loopback clients are accepted):
  GET    /__test__/state                    records, counts, requests, faults
  POST   /__test__/server-edit              {id, fields} through the real service
  DELETE /__test__/record/{id}              explicit server-side deletion
  POST   /__test__/fault                    offline, fail_next_snapshot (count),
                                           fail_next_sync (count), lose_next_sync_ack
  POST   /__test__/grant                    {owner_id:1|2, revoke_previous:false}
  POST   /__test__/revoke-devices           revoke every per-computer grant

Per-computer grants come from the real /v1/activate and /v1/device routes; the
owner password is set through the real PUT /api/excel/activations route.

Faults return HTTP 503. lose_next_sync_ack returns 503 AFTER a successful sync
has committed, simulating an acknowledgement that the client cannot accept.
The /__test__ controls remain available while offline=true. Replacement grants
are written to separate credential files; the initial grant binds on first use.
"""
from __future__ import annotations

import argparse
import hashlib
import os
from pathlib import Path
import secrets
import sys
from datetime import datetime, timedelta, timezone
from decimal import Decimal
from uuid import UUID, uuid4

# Set before importing any project module. This helper never connects to the
# configured application database, even when launched from the backend folder.
os.environ["PYTHON_DOTENV_DISABLED"] = "1"
os.environ["DATABASE_URL"] = "sqlite://"
os.environ["DEBUG"] = "False"
BACKEND = Path(__file__).resolve().parents[2]
if str(BACKEND) not in sys.path:
    sys.path.insert(0, str(BACKEND))

import json
import uvicorn
from fastapi import FastAPI, HTTPException, Request
from fastapi.responses import JSONResponse
from pydantic import BaseModel, ConfigDict, Field
from sqlalchemy import create_engine
from sqlalchemy.orm import Session
from sqlalchemy.pool import StaticPool

from api import dependencies
from api.routes import excel_sync as routes
from database.connection import get_db_session
from database.excel_sync_models import ExcelSyncGrant, ExcelSyncOperation
from database.models import (
    AccessAuditLog, AnnotationArticle, AnnotationPerson, Base, SyncDeletion,
    Transaction, TransactionAnnotationEvent, User,
)
from services.excel_sync_service import apply_operation, serialize_record
from services.user_service import build_permissions_snapshot


class ServerEdit(BaseModel):
    model_config = ConfigDict(extra="forbid")
    id: int
    fields: dict


class Faults(BaseModel):
    model_config = ConfigDict(extra="forbid")
    offline: bool | None = None
    fail_next_snapshot: int | None = Field(default=None, ge=0, le=100)
    fail_next_sync: int | None = Field(default=None, ge=0, le=100)
    lose_next_sync_ack: bool | None = None
    omit_snapshot_etag: bool | None = None
    bad_snapshot_etag: bool | None = None


class ReplacementGrant(BaseModel):
    model_config = ConfigDict(extra="forbid")
    owner_id: int = 1
    revoke_previous: bool = False


def create_app(state_dir: Path, port: int, seed_count: int = 2) -> FastAPI:
    if not 2 <= seed_count <= 5000:
        raise ValueError("Use between 2 and 5000 synthetic seed records")
    state_dir = state_dir.resolve()
    project = BACKEND.parent
    if state_dir == project or project in state_dir.parents:
        raise ValueError("Test state and credentials must be outside the project")
    state_dir.mkdir(parents=True, exist_ok=True)
    if any(state_dir.iterdir()):
        raise ValueError("Use a new empty directory for each synthetic server")

    engine = create_engine("sqlite://", connect_args={"check_same_thread": False}, poolclass=StaticPool)
    Base.metadata.create_all(engine)
    instance_id = str(uuid4())
    api_url = f"http://127.0.0.1:{port}"
    request_log: list[dict] = []
    published: list[dict] = []
    faults = {"offline": False, "fail_next_snapshot": 0, "fail_next_sync": 0,
              "lose_next_sync_ack": False, "lost_ack_count": 0,
              "omit_snapshot_etag": False, "bad_snapshot_etag": False}
    seed_ids: list[str] = []
    public_grants: list[UUID] = []

    def write_credentials(session: Session, owner_id: int, *, initial: bool = False) -> dict:
        owner = session.get(User, owner_id)
        if owner is None:
            raise HTTPException(422, "unknown_synthetic_owner")
        token = "excel_" + secrets.token_urlsafe(32)
        grant = ExcelSyncGrant(
            user_id=owner_id, name="Native Excel synthetic test", token_hash=hashlib.sha256(token.encode()).hexdigest(),
            permissions_version=build_permissions_snapshot(owner)["permissions_version"],
            workbook_id=None, expires_at=datetime.now(timezone.utc).replace(tzinfo=None) + timedelta(days=1),
        )
        session.add(grant)
        session.commit()
        public_grants.append(grant.id)
        filename = "credentials.json" if initial else f"credentials-{grant.id}.json"
        path = state_dir / filename
        path.write_text(json.dumps({
            "synthetic": True, "api_url": api_url, "token": token, "grant_id": str(grant.id),
            "user_id": str(owner_id), "workbook_id": None, "seed_ids": seed_ids,
            "pid": os.getpid(), "instance_id": instance_id,
        }, ensure_ascii=False, indent=2), encoding="utf-8")
        return {"credentials_path": str(path), "grant_id": str(grant.id), "user_id": str(owner_id)}

    with Session(engine) as session:
        session.add_all([
            User(id=1, username="excel-native-test", password_hash="unused", salt="unused", role="admin"),
            User(id=2, username="excel-native-other-owner", password_hash="unused", salt="unused", role="admin"),
            AnnotationPerson(code="СТ", full_name="Synthetic person one"),
            AnnotationPerson(code="ТТ", full_name="Synthetic person two"),
            AnnotationArticle(code="р1", label="Synthetic expense", direction="expense"),
            AnnotationArticle(code="д1", label="Synthetic income", direction="income"),
        ])
        session.flush()
        for index in range(1, seed_count + 1):
            kind = "DEBIT" if index % 2 else "CREDIT"
            row = Transaction(
                source_type="AUTO", source_chat_id=10, source_message_id=index,
                transaction_date=datetime(2026, 9, 1, 12, 30) + timedelta(days=index - 1),
                amount=Decimal("-123.45") if kind == "DEBIT" else Decimal("2500.00"),
                transaction_type=kind, currency="UZS", operator_raw=f"Synthetic merchant {index}",
                application_mapped="Synthetic app", card_last_4=f"{index:04d}",
                balance_after=Decimal("9000.00"), raw_message=f"Synthetic receipt {index}; no real data",
                is_p2p=False, annotation_details=f"Initial synthetic note {index}", annotation_details_src="manual",
            )
            session.add(row)
            session.flush()
            seed_ids.append(str(row.id))
        control_grant = ExcelSyncGrant(
            user_id=1, name="Synthetic control only", token_hash=secrets.token_hex(32),
            workbook_id=str(uuid4()), permissions_version=1,
            expires_at=datetime.now(timezone.utc).replace(tzinfo=None) + timedelta(days=1),
        )
        session.add(control_grant)
        session.commit()
        control_grant_id = control_grant.id
        initial_config = write_credentials(session, 1, initial=True)

    def database():
        with Session(engine) as session:
            try:
                yield session
            except Exception:
                session.rollback()
                raise

    def dashboard_user():
        with Session(engine) as session:
            return build_permissions_snapshot(session.get(User, 1))

    async def publish(action, ids):
        published.append({"action": action, "ids": [str(value) for value in ids]})

    routes.system_access_enforced = lambda: False
    for module in (routes, dependencies, routes.tx_api):
        module.get_config_scopes = lambda **_kwargs: []
    routes.tx_api._publish_sync_update = publish
    app = FastAPI(title="Synthetic Excel native test", docs_url=None, redoc_url=None)
    app.include_router(routes.router)
    app.dependency_overrides[get_db_session] = database
    app.dependency_overrides[routes._dashboard_user] = dashboard_user
    app.dependency_overrides[dependencies.require_transactions_scope] = lambda: None

    @app.middleware("http")
    async def isolated_requests(request: Request, call_next):
        if request.client is None or request.client.host not in {"127.0.0.1", "::1"}:
            return JSONResponse({"detail": "loopback_only"}, status_code=403)
        path = request.url.path
        conditional = "if-none-match" in request.headers
        if path == "/api/excel/v1/snapshot" and faults["omit_snapshot_etag"]:
            request.scope["headers"] = [(key, value) for key, value in request.scope["headers"]
                                         if key.lower() != b"if-none-match"]
            if hasattr(request, "_headers"):
                del request._headers
        fault = None
        if path.startswith("/api/excel/v1/"):
            if faults["offline"]:
                fault = "synthetic_offline"
            elif path == "/api/excel/v1/snapshot" and faults["fail_next_snapshot"]:
                faults["fail_next_snapshot"] -= 1
                fault = "synthetic_snapshot_failure"
            elif path == "/api/excel/v1/sync" and faults["fail_next_sync"]:
                faults["fail_next_sync"] -= 1
                fault = "synthetic_sync_failure_before_commit"
        response = JSONResponse({"detail": fault}, status_code=503) if fault else await call_next(request)
        if (not fault and request.method == "POST" and path == "/api/excel/v1/sync"
                and response.status_code == 200 and faults["lose_next_sync_ack"]):
            faults["lose_next_sync_ack"] = False
            faults["lost_ack_count"] += 1
            response = JSONResponse({"detail": "synthetic_ack_lost_after_commit"}, status_code=503)
        response.headers["Cache-Control"] = "no-store"
        if path == "/api/excel/v1/snapshot":
            if faults["omit_snapshot_etag"] and "etag" in response.headers:
                del response.headers["etag"]
            if faults["bad_snapshot_etag"] and response.status_code == 304:
                response.headers["ETag"] = 'W/"synthetic-mismatch"'
        request_log.append({"method": request.method, "path": path, "status": response.status_code,
                            "conditional": conditional})
        del request_log[:-100]
        return response

    def state():
        with Session(engine) as session:
            return {
                "synthetic": True, "instance_id": instance_id, "pid": os.getpid(), "api_url": api_url,
                "seed_ids": seed_ids, "records": [serialize_record(row) for row in session.query(Transaction).order_by(Transaction.id)],
                "counts": {name: session.query(model).count() for name, model in (
                    ("transactions", Transaction), ("operations", ExcelSyncOperation),
                    ("audit", AccessAuditLog), ("annotation_events", TransactionAnnotationEvent), ("deletions", SyncDeletion),
                )},
                "operations": [{"operation_id": str(row.operation_id), "user_id": str(row.user_id),
                                "workbook_id": row.workbook_id, "status": row.response.get("status")}
                               for row in session.query(ExcelSyncOperation).order_by(ExcelSyncOperation.created_at)],
                "grants": [{"id": str(row.id), "user_id": str(row.user_id), "workbook_id": row.workbook_id,
                            "revoked": row.revoked_at is not None}
                           for row in session.query(ExcelSyncGrant).filter(ExcelSyncGrant.id.in_(public_grants))],
                "device_grants": [{"id": str(row.id), "workbook_id": row.workbook_id, "device_id": row.device_id,
                                   "device_name": row.device_name, "revoked": row.revoked_at is not None}
                                  for row in session.query(ExcelSyncGrant).filter(ExcelSyncGrant.device_id.is_not(None))
                                  .order_by(ExcelSyncGrant.created_at)],
                "faults": dict(faults), "events": list(published), "requests": list(request_log),
            }

    @app.get("/__test__/state")
    async def get_state():
        result = state()
        (state_dir / "state.json").write_text(json.dumps(result, ensure_ascii=False, indent=2), encoding="utf-8")
        return result

    def control_operation(session, operation):
        result = apply_operation(session, session.get(ExcelSyncGrant, control_grant_id), operation,
                                 {"user_id": 1}, lambda _tx, **_kwargs: None)
        session.commit()
        return result

    @app.post("/__test__/server-edit")
    async def server_edit(payload: ServerEdit):
        with Session(engine) as session:
            row = session.get(Transaction, payload.id)
            if row is None:
                raise HTTPException(404, "synthetic_record_not_found")
            record = serialize_record(row)
            return control_operation(session, {"operation_id": str(uuid4()), "kind": "update",
                "id": record["id"], "uuid": record["uuid"], "fields": payload.fields,
                "base": {key: record["fields"].get(key) for key in payload.fields}})

    @app.delete("/__test__/record/{record_id}")
    async def delete_record(record_id: int):
        with Session(engine) as session:
            row = session.get(Transaction, record_id)
            if row is None:
                raise HTTPException(404, "synthetic_record_not_found")
            record = serialize_record(row)
            return control_operation(session, {"operation_id": str(uuid4()), "kind": "delete",
                "id": record["id"], "uuid": record["uuid"], "version": record["version"]})

    @app.post("/__test__/fault")
    async def set_fault(payload: Faults):
        faults.update({key: value for key, value in payload.model_dump().items() if value is not None})
        return dict(faults)

    @app.post("/__test__/grant")
    async def replacement_grant(payload: ReplacementGrant):
        with Session(engine) as session:
            if payload.revoke_previous:
                session.query(ExcelSyncGrant).filter(ExcelSyncGrant.id.in_(public_grants)).update(
                    {"revoked_at": datetime.now(timezone.utc).replace(tzinfo=None)}, synchronize_session=False)
            return write_credentials(session, payload.owner_id)

    @app.post("/__test__/revoke-devices")
    async def revoke_devices():
        with Session(engine) as session:
            count = session.query(ExcelSyncGrant).filter(
                ExcelSyncGrant.device_id.is_not(None), ExcelSyncGrant.revoked_at.is_(None),
            ).update({"revoked_at": datetime.now(timezone.utc).replace(tzinfo=None)}, synchronize_session=False)
            session.commit()
            return {"revoked": count}

    (state_dir / "service.json").write_text(json.dumps({
        "synthetic": True, "instance_id": instance_id, "pid": os.getpid(), "api_url": api_url,
        "credentials_path": initial_config["credentials_path"], "seed_ids": seed_ids,
        "database": "fresh in-memory SQLite", "stop": "Stop this PID after native testing",
    }, ensure_ascii=False, indent=2), encoding="utf-8")
    return app


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--state-dir", type=Path, required=True, help="New empty directory outside the project")
    parser.add_argument("--port", type=int, default=8767)
    parser.add_argument("--seed-count", type=int, default=2,
                        help="Synthetic receipts; use 240 for native viewport regression")
    args = parser.parse_args()
    if not 1024 <= args.port <= 65535:
        parser.error("Use an unprivileged localhost port")
    if not 2 <= args.seed_count <= 5000:
        parser.error("Use between 2 and 5000 synthetic seed records")
    app = create_app(args.state_dir, args.port, seed_count=args.seed_count)
    print(f"Synthetic Excel service: {args.state_dir.resolve() / 'service.json'}", flush=True)
    uvicorn.run(app, host="127.0.0.1", port=args.port, log_level="warning", access_log=False, proxy_headers=False)


if __name__ == "__main__":
    main()
