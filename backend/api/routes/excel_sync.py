"""User-scoped, revocable access for the desktop Excel sync workbook.

Only the v1 endpoints are reachable without an application login: snapshot,
sync and device use an Excel grant, activate uses the workbook's activation
password. Grant/password management and workbook download retain the normal
login, system and scope checks.
"""
from __future__ import annotations

import hashlib
import json
import logging
import os
import re
import secrets
import unicodedata
import uuid
from dataclasses import dataclass
from datetime import datetime, timedelta, timezone
from pathlib import Path
from typing import Any, Optional
from urllib.parse import quote, urlsplit

from fastapi import APIRouter, Depends, Header, HTTPException, Query, Request, Response
from fastapi.exceptions import RequestValidationError
from fastapi.responses import FileResponse
from fastapi.routing import APIRoute
from pydantic import BaseModel, ConfigDict, Field
from sqlalchemy import or_
from sqlalchemy.orm import Session

from api import dependencies
from api.routes import transactions as tx_api
from database.connection import get_db_session
from database.models import AccessScope, AnnotationArticle, AnnotationPerson, LockedPeriod, Transaction, User
from database.excel_sync_models import ExcelSyncActivation, ExcelSyncGrant
from services.access_control_service import (
    config_scope_to_public_dict, hash_password, is_datetime_allowed, verify_password, write_audit_log,
)
from services import excel_workbook_builder as workbook_builder
from services.excel_sync_service import apply_operation, serialize_record
from services.root_access_config_service import (
    get_config_scopes, is_system_blocked, system_access_enforced,
)
from services.user_service import build_permissions_snapshot
from services.workspace_access import WORKSPACE_CHECKS, has_workspace

logger = logging.getLogger(__name__)


class _QuietValidationRoute(APIRoute):
    """A validation error of these routes does not repeat what was sent: it may be a workbook password."""

    def get_route_handler(self):
        handler = super().get_route_handler()

        async def quiet(request: Request) -> Response:
            try:
                return await handler(request)
            except RequestValidationError as exc:
                errors = [{key: value for key, value in error.items() if key != "input"} for error in exc.errors()]
                raise RequestValidationError(errors) from None

        return quiet


router = APIRouter(prefix="/api/excel", tags=["Excel"], route_class=_QuietValidationRoute)
_dashboard_user = dependencies.require_tab_access("dashboard")
GRANT_ENDPOINTS = frozenset({"/api/excel/v1/snapshot", "/api/excel/v1/sync", "/api/excel/v1/device",
                             "/api/excel/v1/activate"})
DEVICE_ID = re.compile(r"^[A-Za-z0-9._:{}-]{8,128}$")
ACTIVATION_ATTEMPTS = 5
ACTIVATION_LOCKOUT = timedelta(minutes=15)
MAX_ACTIVE_GRANTS = 20
MAX_WORKBOOKS = 20
WORKBOOK_NAME_MAX = 80
WORKBOOK_MEDIA_TYPE = "application/vnd.ms-excel.sheet.macroEnabled.12"
WORKBOOK_FILENAME = "Чеки — синхронизация"


def _now() -> datetime:
    return datetime.now(timezone.utc).replace(tzinfo=None)


def _naive_utc(value: datetime) -> datetime:
    return value.astimezone(timezone.utc).replace(tzinfo=None) if value.tzinfo else value


def _iso(value: Optional[datetime]) -> Optional[str]:
    return _naive_utc(value).isoformat() + "Z" if value else None


def _grant_public(row: ExcelSyncGrant) -> dict:
    return {"id": str(row.id), "name": row.name, "workbook_id": str(row.workbook_id) if row.workbook_id else None,
            "device_name": row.device_name, "device_bound": row.device_id is not None,
            **{key: _iso(getattr(row, key)) for key in
               ("created_at", "expires_at", "revoked_at", "last_used_at")}}


def _current_scope_definition(db: Session, scope_id: int) -> Optional[dict]:
    row = db.get(AccessScope, scope_id)
    if row is not None:
        if not row.is_active:
            return None
        return {"scope_id": int(row.id), "scope_name": row.name,
                "years": tx_api.years_from_json(row.years_json),
                "date_from": row.period_start.isoformat() if row.period_start else None,
                "date_to": row.period_end.isoformat() if row.period_end else None,
                "allow_transactions": bool(row.allow_transactions), "allow_sources": bool(row.allow_sources),
                "_definition_updated_at": row.updated_at.isoformat() if row.updated_at else None}
    for config in get_config_scopes(active_only=True):
        public = config_scope_to_public_dict(config)
        if int(public.get("id") or 0) == scope_id:
            return {"scope_id": scope_id, "scope_name": public.get("name"), "years": public.get("years") or [],
                    "date_from": public.get("period_start"), "date_to": public.get("period_end"),
                    "allow_transactions": bool(public.get("allow_transactions", True)),
                    "allow_sources": bool(public.get("allow_sources", False)),
                    "_definition_updated_at": public.get("updated_at")}
    return None


def _capture_scope(db: Session, scope: Optional[dict]) -> Optional[dict]:
    if not scope:
        return None
    if int(scope.get("scope_id") or 0) > 0:
        current = _current_scope_definition(db, int(scope["scope_id"]))
        if current is None or not dependencies._scope_claims_match(scope, current):
            raise HTTPException(403, "scope_changed")
        return current
    return dict(scope)


def _check_user(user: User) -> dict:
    now = _now()
    if not user.is_active:
        raise HTTPException(403, "inactive_user")
    if user.locked_until and _naive_utc(user.locked_until) > now:
        raise HTTPException(403, "user_locked")
    snapshot = build_permissions_snapshot(user)
    # A workbook is a way into Checks; without the workspace there is none.
    if not has_workspace(snapshot, WORKSPACE_CHECKS):
        raise HTTPException(403, "workspace_forbidden:checks")
    if snapshot["role"] != "admin":
        if dependencies.access_expired_at(snapshot):
            raise HTTPException(403, "access_expired")
        if "dashboard" not in snapshot["allowed_tabs"]:
            raise HTTPException(403, "tab_forbidden:dashboard")
    return snapshot


@dataclass
class ExcelContext:
    grant: ExcelSyncGrant
    user: dict
    scopes: list[dict]
    workbook_id: str


async def require_excel_grant(
    request: Request,
    db: Session = Depends(get_db_session),
    authorization: Optional[str] = Header(None),
    x_excel_workbook: Optional[str] = Header(None, alias="X-Excel-Workbook"),
    x_excel_device: Optional[str] = Header(None, alias="X-Excel-Device"),
) -> ExcelContext:
    """A grant is not a login JWT or the shared read-only CSV key."""
    if system_access_enforced():
        blocked, _reason = is_system_blocked()
        if blocked:
            raise HTTPException(503, "system_access_blocked")
    scheme, _, token = (authorization or "").partition(" ")
    if scheme.lower() != "bearer" or not token.startswith("excel_") or not 40 <= len(token) <= 200:
        raise HTTPException(401, "excel_connection_required")
    try:
        workbook_id = str(uuid.UUID(x_excel_workbook or ""))
    except ValueError:
        raise HTTPException(400, "invalid_workbook_id")
    digest = hashlib.sha256(token.encode("utf-8")).hexdigest()
    # Serialize first binding and retries from the same grant. User lock below
    # also prevents permissions changing between authorization and commit.
    grant = db.query(ExcelSyncGrant).filter(ExcelSyncGrant.token_hash == digest).with_for_update().first()
    if grant is None or grant.revoked_at or (
        grant.expires_at is not None and _naive_utc(grant.expires_at) <= _now()
    ):
        raise HTTPException(401, "excel_connection_expired_or_revoked")
    if grant.workbook_id and str(grant.workbook_id) != workbook_id:
        raise HTTPException(403, "excel_connection_belongs_to_another_workbook")
    if grant.device_id and not secrets.compare_digest(str(grant.device_id), x_excel_device or ""):
        raise HTTPException(403, "excel_connection_belongs_to_another_device")
    user_row = db.query(User).filter(User.id == grant.user_id).with_for_update().first()
    if user_row is None:
        raise HTTPException(401, "excel_connection_expired_or_revoked")
    user = _check_user(user_row)
    if int(grant.permissions_version) != user["permissions_version"]:
        raise HTTPException(403, "permissions_changed_reconnect_excel")
    granted_scope = grant.scope
    if granted_scope and int(granted_scope.get("scope_id") or 0) > 0:
        current = _current_scope_definition(db, int(granted_scope["scope_id"]))
        if current != granted_scope:
            raise HTTPException(403, "scope_changed_reconnect_excel")
    if user["role"] != "admin" and not granted_scope:
        if dependencies.has_active_scopes(db) and dependencies.is_otp_enabled(db):
            raise HTTPException(403, "scope_required_reconnect_excel")
    # Apply both constraints separately. Merging disjoint year sets into []
    # would incorrectly turn a disjoint intersection into an unbounded scope.
    folder_scope = dependencies.get_effective_folder_scope_for_user(user, db)
    scopes = [s for s in (granted_scope, folder_scope) if s]
    if any(not s.get("allow_transactions", True) for s in scopes):
        raise HTTPException(403, "scope_forbidden_transactions")
    grant.workbook_id = workbook_id
    grant.last_used_at = _now()
    return ExcelContext(grant, user, scopes, workbook_id)


class GrantCreate(BaseModel):
    model_config = ConfigDict(extra="forbid")
    name: str = Field(default="Excel", min_length=1, max_length=120)
    expires_in_days: Optional[int] = Field(default=90, ge=1, le=365)


@router.post("/grants", status_code=201)
async def create_grant(
    payload: GrantCreate, request: Request, response: Response,
    db: Session = Depends(get_db_session), current_user: dict = Depends(_dashboard_user),
    scope: Optional[dict] = Depends(dependencies.require_transactions_scope),
):
    name = payload.name.strip()
    if not name:
        raise HTTPException(422, "connection_name_required")
    uid = int(current_user.get("id") or current_user.get("user_id") or 0)
    user_row = db.query(User).filter(User.id == uid).with_for_update().populate_existing().first()
    if user_row is None:
        raise HTTPException(401, "registered_user_required")
    user = _check_user(user_row)
    if payload.expires_in_days is None and user["role"] != "admin":
        raise HTTPException(403, "permanent_excel_connection_admin_only")
    active_count = db.query(ExcelSyncGrant).filter(
        ExcelSyncGrant.user_id == uid, ExcelSyncGrant.revoked_at.is_(None),
        or_(ExcelSyncGrant.expires_at.is_(None), ExcelSyncGrant.expires_at > _now()),
    ).count()
    if active_count >= MAX_ACTIVE_GRANTS:
        raise HTTPException(409, "too_many_excel_connections_revoke_unused")
    token = "excel_" + secrets.token_urlsafe(32)
    now = _now()  # one reading: the term runs from the moment of creation
    grant = ExcelSyncGrant(user_id=uid, name=name, token_hash=hashlib.sha256(token.encode()).hexdigest(),
                           permissions_version=user["permissions_version"], scope=_capture_scope(db, scope),
                           created_at=now, expires_at=(
                               now + timedelta(days=payload.expires_in_days)
                               if payload.expires_in_days is not None else None
                           ))
    db.add(grant)
    db.flush()
    write_audit_log(db, action="excel_grant_created", success=True, user_id=uid,
                    ip_address=tx_api._request_ip(request),
                    details={"grant_id": str(grant.id), "name": name, "expires_at": _iso(grant.expires_at)})
    db.commit()
    response.headers["Cache-Control"] = "no-store"
    return {"grant": _grant_public(grant), "token": token, "server_url": str(request.base_url).rstrip("/")}


@router.get("/grants")
async def list_grants(
    response: Response, db: Session = Depends(get_db_session), current_user: dict = Depends(_dashboard_user),
):
    uid = int(current_user.get("id") or current_user.get("user_id") or 0)
    rows = db.query(ExcelSyncGrant).filter(ExcelSyncGrant.user_id == uid).order_by(ExcelSyncGrant.created_at.desc()).all()
    response.headers["Cache-Control"] = "no-store"
    return {"grants": [_grant_public(row) for row in rows]}


@router.delete("/grants/{grant_id}")
async def revoke_grant(
    grant_id: uuid.UUID, request: Request, db: Session = Depends(get_db_session),
    current_user: dict = Depends(_dashboard_user),
):
    uid = int(current_user.get("id") or current_user.get("user_id") or 0)
    row = db.query(ExcelSyncGrant).filter(ExcelSyncGrant.id == grant_id, ExcelSyncGrant.user_id == uid).with_for_update().first()
    if row is None:
        raise HTTPException(404, "connection_not_found")
    if row.revoked_at is None:
        row.revoked_at = _now()
        write_audit_log(db, action="excel_grant_revoked", success=True, user_id=uid,
                        ip_address=tx_api._request_ip(request), details={"grant_id": str(row.id)})
        db.commit()
    return {"ok": True}


@router.get("/workbook")
async def download_workbook(current_user: dict = Depends(_dashboard_user)):
    path = Path(__file__).resolve().parents[2] / "assets" / "excel_sync_template.xlsm"
    if not path.is_file():
        raise HTTPException(503, "excel_workbook_not_built")
    return FileResponse(path, filename="Чеки — синхронизация.xlsm",
                        media_type="application/vnd.ms-excel.sheet.macroEnabled.12",
                        headers={"Cache-Control": "no-store"})


def _active_grants(db: Session, user_id: int):
    return db.query(ExcelSyncGrant).filter(
        ExcelSyncGrant.user_id == user_id, ExcelSyncGrant.revoked_at.is_(None),
        or_(ExcelSyncGrant.expires_at.is_(None), ExcelSyncGrant.expires_at > _now()),
    )


def _device_id(value: Optional[str]) -> str:
    value = (value or "").strip()
    if not DEVICE_ID.fullmatch(value):
        raise HTTPException(400, "invalid_device_id")
    return value


def _issue_device_grant(db: Session, request: Request, *, user_id: int, workbook_id: str, device_id: str,
                        device_name: str, permissions_version: int, scope: Optional[dict],
                        expires_at: Optional[datetime], action: str) -> tuple[ExcelSyncGrant, str]:
    """One live grant per computer and workbook: a repeated activation replaces it."""
    replaced = _active_grants(db, user_id).filter(
        ExcelSyncGrant.workbook_id == workbook_id, ExcelSyncGrant.device_id == device_id,
    ).with_for_update().all()
    for row in replaced:
        row.revoked_at = _now()
    db.flush()
    if _active_grants(db, user_id).count() >= MAX_ACTIVE_GRANTS:
        raise HTTPException(409, "too_many_excel_connections_revoke_unused")
    token = "excel_" + secrets.token_urlsafe(32)
    grant = ExcelSyncGrant(user_id=user_id, name=f"ПК {device_name}"[:120],
                           token_hash=hashlib.sha256(token.encode()).hexdigest(),
                           workbook_id=workbook_id, device_id=device_id, device_name=device_name,
                           permissions_version=permissions_version, scope=scope, created_at=_now(),
                           expires_at=expires_at)
    db.add(grant)
    db.flush()
    write_audit_log(db, action=action, success=True, user_id=user_id, ip_address=tx_api._request_ip(request),
                    details={"grant_id": str(grant.id), "workbook_id": workbook_id, "device_name": device_name,
                             "replaced": [str(row.id) for row in replaced]})
    return grant, token


class DeviceName(BaseModel):
    model_config = ConfigDict(extra="forbid")
    device_name: str = Field(min_length=1, max_length=80)


class ActivationRequest(DeviceName):
    password: str = Field(min_length=1, max_length=128)


class ActivationPassword(BaseModel):
    model_config = ConfigDict(extra="forbid")
    workbook_id: uuid.UUID
    password: str = Field(min_length=8, max_length=128)


def _activation_public(row: ExcelSyncActivation) -> dict:
    locked = row.locked_until if row.locked_until and _naive_utc(row.locked_until) > _now() else None
    return {"workbook_id": row.workbook_id, "name": row.name, "created_at": _iso(row.created_at),
            "updated_at": _iso(row.updated_at), "locked_until": _iso(locked)}


def _owned_workbook(db: Session, uid: int, workbook_id: str, *,
                    lock: bool = False) -> tuple[bool, Optional[ExcelSyncActivation]]:
    """Whether this user manages the workbook, and its password record if there is one.

    A workbook with a password record belongs to that record's user alone: they
    made it here, or protected it. Without a record it belongs to a user whose
    grant is bound to it.
    """
    query = db.query(ExcelSyncActivation).filter(ExcelSyncActivation.workbook_id == workbook_id)
    row = (query.with_for_update() if lock else query).first()
    if row is not None:
        return (True, row) if row.user_id == uid else (False, None)
    bound = db.query(ExcelSyncGrant.id).filter(
        ExcelSyncGrant.user_id == uid, ExcelSyncGrant.workbook_id == workbook_id,
    ).first()
    return bound is not None, None


@router.get("/activations")
async def list_activations(
    response: Response, db: Session = Depends(get_db_session), current_user: dict = Depends(_dashboard_user),
):
    uid = int(current_user.get("id") or current_user.get("user_id") or 0)
    rows = db.query(ExcelSyncActivation).filter(ExcelSyncActivation.user_id == uid).order_by(
        ExcelSyncActivation.created_at.desc(), ExcelSyncActivation.workbook_id).all()
    response.headers["Cache-Control"] = "no-store"
    return {"activations": [_activation_public(row) for row in rows]}


@router.put("/activations")
async def set_activation_password(
    payload: ActivationPassword, request: Request, response: Response,
    db: Session = Depends(get_db_session), current_user: dict = Depends(_dashboard_user),
    scope: Optional[dict] = Depends(dependencies.require_transactions_scope),
):
    """Protect a workbook this user made or connected; new computers must enter the password."""
    uid = int(current_user.get("id") or current_user.get("user_id") or 0)
    user_row = db.query(User).filter(User.id == uid).with_for_update().populate_existing().first()
    if user_row is None:
        raise HTTPException(401, "registered_user_required")
    user = _check_user(user_row)
    if not payload.password.strip():
        raise HTTPException(422, "password_required")
    workbook_id = str(payload.workbook_id)
    owned, row = _owned_workbook(db, uid, workbook_id, lock=True)
    if not owned:
        raise HTTPException(404, "workbook_not_found")
    digest, salt = hash_password(payload.password)
    if row is None:
        row = ExcelSyncActivation(workbook_id=workbook_id, user_id=uid, created_at=_now())
        db.add(row)
    row.password_hash, row.salt = digest, salt
    row.permissions_version = user["permissions_version"]
    row.scope = _capture_scope(db, scope)
    row.failed_attempts, row.locked_until, row.updated_at = 0, None, _now()
    write_audit_log(db, action="excel_activation_password_set", success=True, user_id=uid,
                    ip_address=tx_api._request_ip(request), details={"workbook_id": workbook_id})
    db.commit()
    response.headers["Cache-Control"] = "no-store"
    return {"activation": _activation_public(row)}


@router.delete("/activations/{workbook_id}")
async def remove_activation_password(
    workbook_id: uuid.UUID, request: Request, db: Session = Depends(get_db_session),
    current_user: dict = Depends(_dashboard_user),
):
    """New computers can no longer activate; computers already activated keep their grants."""
    uid = int(current_user.get("id") or current_user.get("user_id") or 0)
    row = db.query(ExcelSyncActivation).filter(
        ExcelSyncActivation.workbook_id == str(workbook_id), ExcelSyncActivation.user_id == uid,
    ).with_for_update().first()
    if row is None:
        raise HTTPException(404, "activation_not_found")
    db.delete(row)
    write_audit_log(db, action="excel_activation_password_removed", success=True, user_id=uid,
                    ip_address=tx_api._request_ip(request), details={"workbook_id": str(workbook_id)})
    db.commit()
    return {"ok": True}


def _refused(status: int, code: str, message: str) -> HTTPException:
    # A code in detail.error survives the application's error envelope, which
    # replaces a bare string with one generic code per status.
    return HTTPException(status, {"error": code, "message": message})


def _request_address(request: Request) -> tuple[str, Optional[int]]:
    """Host and explicit port this request was addressed to; no host when its Host header is unusable."""
    try:
        return (request.url.hostname or "").lower(), request.url.port
    except ValueError:  # an unbalanced bracket or a port that is not a number
        return "", None


def _workbook_server_url(request: Request, requested: Optional[str]) -> str:
    """The address written into a workbook file.

    EXCEL_SYNC_PUBLIC_URL decides when it is set, whatever the client sent.
    Otherwise it is the address the client says it uses, accepted only for the
    host this very request came to, or failing that the request's own address.
    """
    configured = os.getenv("EXCEL_SYNC_PUBLIC_URL", "").strip()
    try:
        if configured:
            return workbook_builder.normalize_server_url(configured)
        if not (requested or "").strip():
            return workbook_builder.normalize_server_url(str(request.base_url))
    except workbook_builder.InvalidServerUrl as exc:
        logger.warning("No usable Excel workbook address (EXCEL_SYNC_PUBLIC_URL %s): %s",
                       "is set" if configured else "is not set", exc)
        raise _refused(503, "excel_public_url_not_configured",
                       "Set EXCEL_SYNC_PUBLIC_URL to the https address workbooks reach this server by.")
    try:
        url = workbook_builder.normalize_server_url(requested)
    except workbook_builder.InvalidServerUrl as exc:
        raise _refused(422, "invalid_server_url", f"The workbook cannot use this server address: {exc}.")
    # A crafted link must not aim someone's workbook, and the password typed
    # into it, at another server. A proxy may drop the port from Host, so only
    # a port that is there has to agree.
    host, port = _request_address(request)
    target = urlsplit(url)
    if not host or target.hostname != host or (
            port is not None and port != (target.port or (443 if target.scheme == "https" else 80))):
        raise _refused(422, "invalid_server_url", "server_url must be the address of this server.")
    return url


def _configured_workbook(workbook_id: str, server_url: str, owner_id: int) -> bytes:
    try:
        template = workbook_builder.TEMPLATE_PATH.read_bytes()
    except OSError:
        raise _refused(503, "excel_workbook_not_built", "The Excel workbook template is not installed.")
    try:
        return workbook_builder.configure_workbook(
            template, workbook_id=workbook_id, server_url=server_url, owner_id=owner_id)
    except workbook_builder.WorkbookTemplateError as exc:
        logger.error("The Excel workbook template cannot be configured: %s", exc)
        raise _refused(503, "excel_workbook_template_invalid", "The Excel workbook template cannot be configured.")


def _workbook_attachment(name: Optional[str]) -> str:
    # Characters Windows refuses in a file name, and invisible ones, become spaces.
    kept = "".join(" " if ch in '<>:"/\\|?*' or unicodedata.category(ch).startswith("C") else ch
                   for ch in name or "")
    filename = (" ".join(kept.split()).strip(". ") or WORKBOOK_FILENAME) + ".xlsm"
    encoded = quote(filename)
    if encoded == filename:
        return f'attachment; filename="{filename}"'
    return f"attachment; filename*=utf-8''{encoded}"


class WorkbookCreate(BaseModel):
    model_config = ConfigDict(extra="forbid")
    name: str = Field(min_length=1, max_length=200)
    password: str = Field(min_length=8, max_length=128)
    server_url: Optional[str] = Field(default=None, max_length=2048)


@router.post("/workbooks", status_code=201)
async def create_workbook(
    payload: WorkbookCreate, request: Request, response: Response,
    db: Session = Depends(get_db_session), current_user: dict = Depends(_dashboard_user),
    scope: Optional[dict] = Depends(dependencies.require_transactions_scope),
):
    """Make a workbook whose file needs nothing but its password typed in Excel.

    The workbook is its password record: it exists before any computer holds a
    grant, so the first activation needs no earlier connection.
    """
    uid = int(current_user.get("id") or current_user.get("user_id") or 0)
    user_row = db.query(User).filter(User.id == uid).with_for_update().populate_existing().first()
    if user_row is None:
        raise HTTPException(401, "registered_user_required")
    user = _check_user(user_row)
    name = " ".join(payload.name.split())
    if not name or len(name) > WORKBOOK_NAME_MAX or any(unicodedata.category(ch) == "Cc" for ch in name):
        raise _refused(422, "workbook_name_invalid", f"The name must have 1 to {WORKBOOK_NAME_MAX} characters.")
    if not payload.password.strip():
        raise _refused(422, "password_required", "The password cannot be blank.")
    if db.query(ExcelSyncActivation).filter(ExcelSyncActivation.user_id == uid).count() >= MAX_WORKBOOKS:
        raise _refused(409, "too_many_excel_workbooks", "Remove the passwords of workbooks no longer in use.")
    server_url = _workbook_server_url(request, payload.server_url)
    captured = _capture_scope(db, scope)
    workbook_id = str(uuid.uuid4())
    # Build the file before storing anything: a workbook that cannot be handed
    # out must not leave a record behind.
    _configured_workbook(workbook_id, server_url, uid)
    digest, salt = hash_password(payload.password)
    now = _now()
    row = ExcelSyncActivation(workbook_id=workbook_id, user_id=uid, name=name, password_hash=digest, salt=salt,
                              permissions_version=user["permissions_version"], scope=captured,
                              failed_attempts=0, created_at=now, updated_at=now)
    db.add(row)
    write_audit_log(db, action="excel_workbook_created", success=True, user_id=uid,
                    ip_address=tx_api._request_ip(request), details={"workbook_id": workbook_id, "name": name})
    db.commit()
    response.headers["Cache-Control"] = "no-store"
    return {"workbook": _activation_public(row), "server_url": server_url}


@router.get("/workbooks/{workbook_id}/file")
async def download_configured_workbook(
    workbook_id: uuid.UUID, request: Request, server_url: Optional[str] = Query(None, max_length=2048),
    db: Session = Depends(get_db_session), current_user: dict = Depends(_dashboard_user),
):
    """The owner's workbook file, as often as needed: it holds no key and no
    password, so every computer that opens it still has to be activated."""
    uid = int(current_user.get("id") or current_user.get("user_id") or 0)
    owned, row = _owned_workbook(db, uid, str(workbook_id))
    if not owned:
        raise _refused(404, "workbook_not_found", "No such workbook among yours.")
    content = _configured_workbook(str(workbook_id), _workbook_server_url(request, server_url), uid)
    return Response(content=content, media_type=WORKBOOK_MEDIA_TYPE, headers={
        "Cache-Control": "no-store", "Content-Disposition": _workbook_attachment(row.name if row else None)})


@router.post("/v1/activate", status_code=201)
async def activate_device(
    payload: ActivationRequest, request: Request, response: Response,
    db: Session = Depends(get_db_session),
    x_excel_workbook: Optional[str] = Header(None, alias="X-Excel-Workbook"),
    x_excel_device: Optional[str] = Header(None, alias="X-Excel-Device"),
):
    """Exchange the workbook password for a grant that works only on this computer."""
    if system_access_enforced():
        blocked, _reason = is_system_blocked()
        if blocked:
            raise HTTPException(503, "system_access_blocked")
    try:
        workbook_id = str(uuid.UUID(x_excel_workbook or ""))
    except ValueError:
        raise HTTPException(400, "invalid_workbook_id")
    device_id = _device_id(x_excel_device)
    device_name = " ".join(payload.device_name.split())
    if not device_name:
        raise HTTPException(422, "device_name_required")
    response.headers["Cache-Control"] = "no-store"
    row = db.query(ExcelSyncActivation).filter(
        ExcelSyncActivation.workbook_id == workbook_id,
    ).with_for_update().first()
    if row is None:
        raise HTTPException(403, "excel_activation_unavailable")
    now = _now()
    if row.locked_until and _naive_utc(row.locked_until) > now:
        retry = int((_naive_utc(row.locked_until) - now).total_seconds()) + 1
        raise HTTPException(429, "excel_activation_locked", headers={"Retry-After": str(retry)})
    if not verify_password(payload.password, row.password_hash, row.salt):
        row.failed_attempts = int(row.failed_attempts or 0) + 1
        locked = row.failed_attempts >= ACTIVATION_ATTEMPTS
        if locked:
            row.failed_attempts, row.locked_until = 0, now + ACTIVATION_LOCKOUT
        write_audit_log(db, action="excel_activation_failed", success=False, user_id=row.user_id,
                        ip_address=tx_api._request_ip(request),
                        details={"workbook_id": workbook_id, "device_name": device_name, "locked": locked})
        db.commit()
        raise HTTPException(401, "excel_activation_password_invalid")
    row.failed_attempts, row.locked_until = 0, None
    user_row = db.query(User).filter(User.id == row.user_id).with_for_update().first()
    if user_row is None:
        raise HTTPException(403, "excel_activation_unavailable")
    user = _check_user(user_row)
    if int(row.permissions_version) != user["permissions_version"]:
        raise HTTPException(403, "permissions_changed_reset_excel_password")
    if row.scope and int(row.scope.get("scope_id") or 0) > 0:
        if _current_scope_definition(db, int(row.scope["scope_id"])) != row.scope:
            raise HTTPException(403, "scope_changed_reset_excel_password")
    grant, token = _issue_device_grant(
        db, request, user_id=row.user_id, workbook_id=workbook_id, device_id=device_id, device_name=device_name,
        permissions_version=row.permissions_version, scope=row.scope,
        expires_at=None if user["role"] == "admin" else now + timedelta(days=90),
        action="excel_device_activated",
    )
    db.commit()
    return {"grant": _grant_public(grant), "token": token}


@router.post("/v1/device", status_code=201)
async def bind_device(
    payload: DeviceName, request: Request, response: Response,
    db: Session = Depends(get_db_session),
    context: ExcelContext = Depends(require_excel_grant),
    x_excel_device: Optional[str] = Header(None, alias="X-Excel-Device"),
):
    """Give a computer that already uses an older shared grant its own grant.

    The shared grant is not revoked here: other computers may still use it.
    Once every computer has been updated the owner revokes it in the panel.
    """
    response.headers["Cache-Control"] = "no-store"
    if context.grant.device_id:
        db.commit()
        raise HTTPException(409, "excel_connection_already_device_bound")
    device_name = " ".join(payload.device_name.split())
    if not device_name:
        raise HTTPException(422, "device_name_required")
    grant, token = _issue_device_grant(
        db, request, user_id=context.grant.user_id, workbook_id=context.workbook_id,
        device_id=_device_id(x_excel_device), device_name=device_name,
        permissions_version=context.grant.permissions_version, scope=context.grant.scope,
        expires_at=context.grant.expires_at, action="excel_device_bound",
    )
    db.commit()
    return {"grant": _grant_public(grant), "token": token}

def _authorize(db: Session, context: ExcelContext, tx: Transaction, *,
               candidate_date: Optional[datetime] = None, creating: bool = False) -> None:
    # Manual records belong to source 0, just as in the existing transaction API.
    # Operators need source 0 explicitly; a spreadsheet cannot manufacture access
    # to a Telegram source by labelling a new row with that chat's name.
    tx_api._ensure_source_allowed(tx, context.user)
    dates = [tx.transaction_date]
    if candidate_date is not None:
        dates.append(candidate_date)
    for value in dates:
        for scope in context.scopes:
            if not is_datetime_allowed(scope, value):
                raise HTTPException(403, "transaction_outside_scope")
        tx_api._assert_user_date_allowed(context.user, value)
        tx_api._assert_not_in_locked_period(db, value, context.user)


def _enrich_record(record: dict, tx: Transaction, source_map: dict) -> dict:
    record.setdefault("metadata", {}).update(tx_api._resolve_transaction_source_fields(tx, source_map))
    # Telegram identifiers may exceed the integer precision of Excel/JavaScript.
    # Keep the protocol representation textual after enriching display labels.
    record["metadata"]["source_chat_id"] = str(tx.source_chat_id)
    return record


@router.get("/v1/snapshot")
async def snapshot(
    request: Request, db: Session = Depends(get_db_session), context: ExcelContext = Depends(require_excel_grant),
):
    query = tx_api._apply_allowed_sources_to_query(db.query(Transaction), context.user)
    for scope in context.scopes:
        query = tx_api._apply_scope_to_query(query, scope)
    query = tx_api._apply_user_forbidden_ranges_to_query(query, tx_api._user_forbidden_ranges(context.user))
    # Excluded users retain their period access, consistently with mutation checks.
    if context.user["role"] != "admin":
        for lock in db.query(LockedPeriod).filter(LockedPeriod.is_active.is_(True)).all():
            if int(context.user["id"]) not in [int(x) for x in (lock.excluded_user_ids or [])]:
                from sqlalchemy import and_, func
                query = query.filter(~and_(func.date(Transaction.transaction_date) >= lock.date_from,
                                           func.date(Transaction.transaction_date) <= lock.date_to))
    max_rows = max(1, min(200000, int(os.getenv("EXCEL_SYNC_MAX_ROWS", "50000"))))
    rows = query.order_by(Transaction.id).limit(max_rows + 1).all()
    if len(rows) > max_rows:
        raise HTTPException(413, "excel_snapshot_limit_exceeded")
    source_map = tx_api._load_source_meta_map(db, list({int(row.source_chat_id) for row in rows}) or [-1])
    records = [_enrich_record(serialize_record(row), row, source_map) for row in rows]
    references = {
        "persons": [{"code": p.code, "label": p.full_name or p.code}
                    for p in db.query(AnnotationPerson).filter(AnnotationPerson.is_active.is_(True)).order_by(AnnotationPerson.sort_order, AnnotationPerson.code)],
        "articles": [{"code": p.code, "label": p.label or p.code}
                     for p in db.query(AnnotationArticle).filter(AnnotationArticle.is_active.is_(True)).order_by(AnnotationArticle.sort_order, AnnotationArticle.code)],
    }
    db.commit()
    # Authorize and rebuild every snapshot before comparing validators. Include
    # identity, visibility, source metadata and references, but not clock time.
    # One canonical encoding serves both the validator and the response body.
    body = json.dumps(
        {"protocol_version": 1, "workbook_id": context.workbook_id, "complete": True,
         "user_id": str(context.user["id"]), "records": records, "references": references},
        ensure_ascii=False, sort_keys=True, separators=(",", ":"), allow_nan=False,
    ).encode("utf-8")
    # server_time makes the full representation byte-variant, so use a weak
    # validator for its stable content and treat the header as an opaque value.
    etag = 'W/"' + hashlib.sha256(body).hexdigest() + '"'
    headers = {"Cache-Control": "no-store", "ETag": etag}
    if request.headers.get("if-none-match") == etag:
        return Response(status_code=304, headers=headers)
    body = body[:-1] + b',"server_time":' + json.dumps(_iso(_now())).encode("ascii") + b"}"
    return Response(content=body, media_type="application/json", headers=headers)


class SyncRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")
    workbook_id: uuid.UUID
    operations: list[dict[str, Any]] = Field(min_length=1, max_length=100)


@router.post("/v1/sync")
async def synchronize(
    payload: SyncRequest, response: Response, db: Session = Depends(get_db_session),
    context: ExcelContext = Depends(require_excel_grant),
):
    if str(payload.workbook_id) != context.workbook_id:
        raise HTTPException(400, "workbook_id_mismatch")
    def authorize(tx, *, candidate_date=None, creating=False):
        return _authorize(db, context, tx, candidate_date=candidate_date, creating=creating)

    results = [apply_operation(db, context.grant, op, context.user, authorize) for op in payload.operations]
    records = [item["record"] for item in results if isinstance(item.get("record"), dict)]
    if records:
        # apply_operation has already locked and authorized each row, including
        # replays. Fetch only display inputs in one batch, not whole rows again.
        rows = db.query(Transaction.id, Transaction.source_type, Transaction.source_chat_id,
                        Transaction.raw_message).filter(
                            Transaction.id.in_({int(record["id"]) for record in records}),
                        ).all()
        source_map = tx_api._load_source_meta_map(db, list({int(row.source_chat_id) for row in rows}) or [-1])
        by_id = {row.id: row for row in rows}
        for record in records:
            row = by_id.get(int(record["id"]))
            if row is not None:
                _enrich_record(record, row, source_map)
    db.commit()
    # Publish only after durable commit. Failure of the notification channel
    # cannot turn a committed operation into a retry that writes twice.
    for status, action in (("applied", "updated"), ("deleted", "deleted")):
        ids = [int(r.get("record", {}).get("id") or r.get("id")) for r in results
               if r.get("status") == status and (r.get("record", {}).get("id") or r.get("id"))]
        if ids:
            await tx_api._publish_sync_update(action, ids)
    response.headers["Cache-Control"] = "no-store"
    return {"results": results, "server_time": _iso(_now())}
