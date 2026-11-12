"""CRM administration API: the one account directory and who may open what.

Everything here belongs to the CRM as a whole and to no workspace. It is
reached by a CRM administrator (``users.global_admin``) whether or not that
person may open any workspace, and by nobody else: a workspace role, the
Checks administrator's included, does not open it.
"""
from __future__ import annotations

import json
import logging
from typing import Any, Dict, List, Optional

from fastapi import APIRouter, Depends, HTTPException, Query, Request
from pydantic import BaseModel, Field
from sqlalchemy.orm import Session

from api.dependencies import ACCOUNT_ADMINISTRATION_AUDIT, require_global_admin
from database.connection import get_db_session
from database.models import AccessAuditLog, AccessScope, User
from services.access_control_service import (
    config_scope_to_public_dict,
    scope_to_public_dict,
    write_audit_log,
)
from services.root_access_config_service import get_config_scopes
from services.user_service import (
    UserServiceError,
    build_permissions_snapshot,
    create_user,
    deactivate_user,
    get_user_by_id,
    list_users,
    reset_password,
    unlock_user,
    update_user,
)
from services.totp_service import disable_2fa
from services.auth_bot_service import (
    list_user_sessions,
    publish_auth_event,
    revoke_user_sessions,
)
from services.auth_service import verify_jwt_token
from services.user_service import (
    ALL_TABS,
    PASSWORD_MIN_LENGTH,
    SESSION_TTL_DAYS_DEFAULT,
    SESSION_TTL_DAYS_MAX,
    SESSION_TTL_DAYS_MIN,
)
from services.workspace_access import (
    effective_access,
    is_global_admin,
    workspace_catalog,
    workspace_roles_of,
    workspaces_of,
)

router = APIRouter(prefix="/api/admin", tags=["admin"])
logger = logging.getLogger(__name__)


class AdminUserResponse(BaseModel):
    id: int
    username: str
    display_name: str
    role: str
    is_active: bool
    allowed_tabs: List[str]
    allowed_folders: List[int]
    forbidden_periods: List[Dict[str, str]]
    allowed_sources: List[int]
    can_toggle_sources: bool
    permissions_version: int
    force_2fa: bool
    totp_enabled: bool
    totp_confirmed_at: Optional[str] = None
    telegram_id: Optional[int] = None
    telegram_username: Optional[str] = None
    session_ttl_days: int
    access_until: Optional[str] = None
    access_note: Optional[str] = None
    failed_attempts: int
    locked_until: Optional[str] = None
    last_login_at: Optional[str] = None
    created_at: Optional[str] = None
    # CRM administration, kept apart from every workspace role.
    global_admin: bool = False
    # What was granted: workspace -> role. `workspaces` is what the Hub lists.
    workspace_roles: Dict[str, str] = Field(default_factory=dict)
    workspaces: List[str] = Field(default_factory=list)
    # What that comes to right now, given the state of the account.
    effective_access: Dict[str, Any] = Field(default_factory=dict)
    access_summary: List[str] = Field(default_factory=list)


class CreateUserRequest(BaseModel):
    username: str = Field(..., min_length=1, max_length=50)
    password: str = Field(..., min_length=1, max_length=500)
    role: str = Field(default="operator", pattern="^(admin|operator)$")
    display_name: Optional[str] = Field(default=None, max_length=100)
    is_active: bool = True
    allowed_tabs: List[str] = Field(default_factory=lambda: ["dashboard"])
    allowed_folders: List[int] = Field(default_factory=list)
    forbidden_periods: List[Dict[str, str]] = Field(default_factory=list)
    allowed_sources: List[int] = Field(default_factory=list)
    can_toggle_sources: bool = False
    force_2fa: bool = False
    session_ttl_days: int = Field(default=7, ge=1, le=90)
    telegram_id: Optional[int] = None
    telegram_username: Optional[str] = Field(default=None, max_length=255)
    # Never implied: an administrator is appointed by name.
    global_admin: bool = False
    # {workspace: role}, stating every grant. A client that predates workspaces
    # sends none and gets what it always got: a Checks account with `role`.
    workspaces: Optional[Dict[str, Optional[str]]] = None


class UpdateUserRequest(BaseModel):
    # The Checks role, as clients from before workspaces send it.
    role: Optional[str] = Field(default=None, pattern="^(admin|operator)$")
    display_name: Optional[str] = Field(default=None, max_length=100)
    is_active: Optional[bool] = None
    allowed_tabs: Optional[List[str]] = None
    allowed_folders: Optional[List[int]] = None
    forbidden_periods: Optional[List[Dict[str, str]]] = None
    allowed_sources: Optional[List[int]] = None
    can_toggle_sources: Optional[bool] = None
    force_2fa: Optional[bool] = None
    session_ttl_days: Optional[int] = Field(default=None, ge=1, le=90)
    # ISO date or datetime; an empty string clears it back to unlimited.
    access_until: Optional[str] = None
    access_note: Optional[str] = Field(default=None, max_length=200)
    global_admin: Optional[bool] = None
    # {workspace: role or null}: a role grants or changes it, null takes the
    # workspace away, a workspace that is not named stays as it is.
    workspaces: Optional[Dict[str, Optional[str]]] = None


class ResetPasswordRequest(BaseModel):
    password: str = Field(..., min_length=1, max_length=500)


class ForceTwoFactorRequest(BaseModel):
    force: bool = True


def _request_ip(request: Request) -> str:
    if request.client and request.client.host:
        return request.client.host
    return "unknown"


def _to_response(user: User) -> AdminUserResponse:
    snapshot = build_permissions_snapshot(user)
    effective = effective_access(user)
    return AdminUserResponse(
        id=int(user.id),
        username=user.username,
        display_name=str(snapshot.get("display_name") or user.username),
        role=str(snapshot.get("role") or "operator"),
        is_active=bool(user.is_active),
        allowed_tabs=[str(x) for x in (snapshot.get("allowed_tabs") or [])],
        allowed_folders=[int(x) for x in (snapshot.get("allowed_folders") or [])],
        forbidden_periods=list(snapshot.get("forbidden_periods") or []),
        allowed_sources=[int(x) for x in (snapshot.get("allowed_sources") or [])],
        can_toggle_sources=bool(snapshot.get("can_toggle_sources")),
        permissions_version=int(snapshot.get("permissions_version") or 1),
        force_2fa=bool(snapshot.get("force_2fa")),
        totp_enabled=bool(snapshot.get("totp_enabled")),
        totp_confirmed_at=snapshot.get("totp_confirmed_at"),
        telegram_id=int(user.telegram_id) if user.telegram_id else None,
        telegram_username=str(user.telegram_username) if user.telegram_username else None,
        session_ttl_days=int(snapshot.get("session_ttl_days") or 7),
        access_until=snapshot.get("access_until"),
        access_note=snapshot.get("access_note"),
        failed_attempts=int(user.failed_attempts or 0),
        locked_until=user.locked_until.isoformat() if user.locked_until else None,
        last_login_at=user.last_login_at.isoformat() if user.last_login_at else None,
        created_at=user.created_at.isoformat() if getattr(user, "created_at", None) else None,
        global_admin=is_global_admin(user),
        workspace_roles=workspace_roles_of(user),
        workspaces=workspaces_of(user),
        effective_access=effective,
        access_summary=list(effective["summary"]),
    )


def _access_state(user: User) -> Dict[str, Any]:
    """What an access change is compared and audited by. No secret is in it."""
    return {
        "is_active": bool(user.is_active),
        "global_admin": is_global_admin(user),
        "workspace_roles": workspace_roles_of(user),
    }


def _actor_id(current_user: Optional[dict]) -> int:
    try:
        return int((current_user or {}).get("id") or (current_user or {}).get("user_id") or 0)
    except (TypeError, ValueError):
        return 0


def _current_session_id(request: Request) -> Optional[str]:
    header = request.headers.get("authorization") or ""
    if not header.lower().startswith("bearer "):
        return None
    payload = verify_jwt_token(header[7:].strip()) or {}
    return str(payload.get("sid") or "") or None


def _refuse(exc: UserServiceError) -> HTTPException:
    code = str(exc)
    # Conflicts with the state of the accounts, as opposed to a malformed request.
    if code in {"last_admin", "username_exists", "telegram_id_already_linked"}:
        return HTTPException(status_code=409, detail=code)
    return HTTPException(status_code=400, detail=code)


async def _end_sessions(user_id: int, *, keep: Optional[str] = None) -> Optional[int]:
    """Ends the account's sessions; ``None`` when the session store did not answer."""
    try:
        return await revoke_user_sessions(int(user_id), except_session_id=keep)
    except Exception:
        logger.warning("Session store unavailable while ending sessions of user_id=%s", user_id, exc_info=True)
        return None


def _audit(
    db: Session,
    request: Request,
    current_user: Optional[dict],
    *,
    action: str,
    success: bool,
    details: Optional[Dict[str, Any]] = None,
) -> None:
    try:
        write_audit_log(
            db,
            action=action,
            success=success,
            user_id=int(current_user.get("id") or current_user.get("user_id") or 0) if current_user else None,
            ip_address=_request_ip(request),
            details=details or {},
        )
    except Exception:
        pass


@router.get("/users", response_model=List[AdminUserResponse])
async def get_users(
    include_inactive: bool = Query(True),
    db: Session = Depends(get_db_session),
    current_user: dict = Depends(require_global_admin),
):
    rows = list_users(db, include_inactive=include_inactive)
    return [_to_response(row) for row in rows]


@router.post("/users", response_model=AdminUserResponse)
async def create_user_endpoint(
    payload: CreateUserRequest,
    request: Request,
    db: Session = Depends(get_db_session),
    current_user: dict = Depends(require_global_admin),
):
    try:
        row = create_user(
            db,
            username=payload.username,
            password=payload.password,
            role=payload.role,
            display_name=payload.display_name,
            is_active=payload.is_active,
            allowed_tabs=payload.allowed_tabs,
            allowed_folders=payload.allowed_folders,
            forbidden_periods=payload.forbidden_periods,
            allowed_sources=payload.allowed_sources,
            can_toggle_sources=payload.can_toggle_sources,
            force_2fa=payload.force_2fa,
            session_ttl_days=payload.session_ttl_days,
            telegram_id=payload.telegram_id,
            telegram_username=payload.telegram_username,
            global_admin=payload.global_admin,
            workspaces=payload.workspaces,
        )
    except UserServiceError as exc:
        raise _refuse(exc)

    _audit(
        db,
        request,
        current_user,
        action="admin_user_create",
        success=True,
        details={"user_id": int(row.id), "username": row.username, "access": _access_state(row)},
    )
    return _to_response(row)


@router.patch("/users/{user_id}", response_model=AdminUserResponse)
async def update_user_endpoint(
    user_id: int,
    payload: UpdateUserRequest,
    request: Request,
    db: Session = Depends(get_db_session),
    current_user: dict = Depends(require_global_admin),
):
    row = get_user_by_id(db, user_id)
    if not row:
        raise HTTPException(status_code=404, detail="user_not_found")

    global_admin = payload.global_admin
    # Before CRM administration had a column of its own it was the Checks role
    # "admin", and a client from that time takes an administrator's rights
    # away by changing the role. It still does, and only by that change: a
    # role sent again unchanged, or a role named under `workspaces`, is about
    # Checks alone.
    if (
        global_admin is None
        and payload.role is not None
        and payload.role != "admin"
        and str(row.role or "").lower() == "admin"
        and is_global_admin(row)
    ):
        global_admin = False

    # An administrator does not take their own rights away by a slip of the
    # hand: another administrator does that.
    if int(row.id) == _actor_id(current_user):
        if payload.is_active is False:
            raise HTTPException(status_code=409, detail="cannot_disable_self")
        if global_admin is False:
            raise HTTPException(status_code=409, detail="cannot_demote_self")

    before = _access_state(row)
    try:
        row = update_user(
            db,
            row,
            role=payload.role,
            display_name=payload.display_name,
            is_active=payload.is_active,
            allowed_tabs=payload.allowed_tabs,
            allowed_folders=payload.allowed_folders,
            forbidden_periods=payload.forbidden_periods,
            allowed_sources=payload.allowed_sources,
            can_toggle_sources=payload.can_toggle_sources,
            force_2fa=payload.force_2fa,
            session_ttl_days=payload.session_ttl_days,
            access_until=payload.access_until,
            access_note=payload.access_note,
            global_admin=global_admin,
            workspaces=payload.workspaces,
        )
    except UserServiceError as exc:
        raise _refuse(exc)
    except ValueError as exc:
        raise HTTPException(status_code=400, detail=str(exc))

    after = _access_state(row)
    _audit(
        db,
        request,
        current_user,
        action="admin_user_update",
        success=True,
        details={"user_id": int(row.id), "username": row.username, "fields": sorted(payload.model_fields_set)},
    )
    if before != after:
        _audit(
            db,
            request,
            current_user,
            action="admin_user_access_change",
            success=True,
            details={"user_id": int(row.id), "username": row.username, "before": before, "after": after},
        )
    if before["is_active"] and not after["is_active"]:
        await _end_sessions(int(row.id))
    try:
        await publish_auth_event(
            "user_permissions_changed",
            {"user_id": int(row.id), "username": row.username, "admin_id": int(current_user.get("id") or 0)},
        )
    except Exception:
        logger.debug("Failed to publish user_permissions_changed for user_id=%s", int(row.id), exc_info=True)
    return _to_response(row)


@router.post("/users/{user_id}/reset-password", response_model=AdminUserResponse)
async def reset_user_password_endpoint(
    user_id: int,
    payload: ResetPasswordRequest,
    request: Request,
    db: Session = Depends(get_db_session),
    current_user: dict = Depends(require_global_admin),
):
    row = get_user_by_id(db, user_id)
    if not row:
        raise HTTPException(status_code=404, detail="user_not_found")

    try:
        row = reset_password(db, row, payload.password)
    except UserServiceError as exc:
        raise HTTPException(status_code=400, detail=str(exc))

    # The new permissions version already refuses every token issued before;
    # the registry entries go too, so the session list tells the truth.
    ended = await _end_sessions(int(row.id))
    _audit(
        db,
        request,
        current_user,
        action="admin_user_reset_password",
        success=True,
        details={"user_id": int(row.id), "username": row.username, "sessions_ended": ended},
    )
    return _to_response(row)


@router.post("/users/{user_id}/reset-2fa", response_model=AdminUserResponse)
async def reset_user_2fa_endpoint(
    user_id: int,
    request: Request,
    db: Session = Depends(get_db_session),
    current_user: dict = Depends(require_global_admin),
):
    row = get_user_by_id(db, user_id)
    if not row:
        raise HTTPException(status_code=404, detail="user_not_found")

    disable_2fa(db, row)
    _audit(
        db,
        request,
        current_user,
        action="admin_user_reset_2fa",
        success=True,
        details={"user_id": int(row.id), "username": row.username},
    )
    try:
        await publish_auth_event(
            "user_2fa_reset",
            {"user_id": int(row.id), "username": row.username, "admin_id": int(current_user.get("id") or 0)},
        )
    except Exception:
        logger.debug("Failed to publish user_2fa_reset for user_id=%s", int(row.id), exc_info=True)
    return _to_response(row)


@router.post("/users/{user_id}/force-2fa", response_model=AdminUserResponse)
async def force_user_2fa_endpoint(
    user_id: int,
    payload: ForceTwoFactorRequest,
    request: Request,
    db: Session = Depends(get_db_session),
    current_user: dict = Depends(require_global_admin),
):
    row = get_user_by_id(db, user_id)
    if not row:
        raise HTTPException(status_code=404, detail="user_not_found")

    row = update_user(db, row, force_2fa=bool(payload.force))
    _audit(
        db,
        request,
        current_user,
        action="admin_user_force_2fa",
        success=True,
        details={"user_id": int(row.id), "username": row.username, "force_2fa": bool(payload.force)},
    )
    try:
        await publish_auth_event(
            "user_2fa_forced",
            {
                "user_id": int(row.id),
                "username": row.username,
                "admin_id": int(current_user.get("id") or 0),
                "force_2fa": bool(payload.force),
            },
        )
    except Exception:
        logger.debug("Failed to publish user_2fa_forced for user_id=%s", int(row.id), exc_info=True)
    return _to_response(row)


@router.post("/users/{user_id}/unlock", response_model=AdminUserResponse)
async def unlock_user_endpoint(
    user_id: int,
    request: Request,
    db: Session = Depends(get_db_session),
    current_user: dict = Depends(require_global_admin),
):
    row = get_user_by_id(db, user_id)
    if not row:
        raise HTTPException(status_code=404, detail="user_not_found")

    row = unlock_user(db, row)
    _audit(
        db,
        request,
        current_user,
        action="admin_user_unlock",
        success=True,
        details={"user_id": int(row.id), "username": row.username},
    )
    return _to_response(row)


@router.delete("/users/{user_id}", response_model=AdminUserResponse)
async def deactivate_user_endpoint(
    user_id: int,
    request: Request,
    db: Session = Depends(get_db_session),
    current_user: dict = Depends(require_global_admin),
):
    row = get_user_by_id(db, user_id)
    if not row:
        raise HTTPException(status_code=404, detail="user_not_found")
    if int(row.id) == _actor_id(current_user):
        raise HTTPException(status_code=409, detail="cannot_disable_self")

    try:
        row = deactivate_user(db, row)
    except UserServiceError as exc:
        raise _refuse(exc)
    ended = await _end_sessions(int(row.id))
    _audit(
        db,
        request,
        current_user,
        action="admin_user_deactivate",
        success=True,
        details={"user_id": int(row.id), "username": row.username, "sessions_ended": ended},
    )
    return _to_response(row)


class LinkTelegramRequest(BaseModel):
    # A person's Telegram ID is positive; a negative one is a group or channel.
    telegram_id: int = Field(..., gt=0)
    telegram_username: Optional[str] = Field(default=None, max_length=255)


@router.post("/users/{user_id}/link-telegram", response_model=AdminUserResponse)
async def link_telegram_endpoint(
    user_id: int,
    payload: LinkTelegramRequest,
    request: Request,
    db: Session = Depends(get_db_session),
    current_user: dict = Depends(require_global_admin),
):
    row = get_user_by_id(db, user_id)
    if not row:
        raise HTTPException(status_code=404, detail="user_not_found")

    # Check that telegram_id is not already linked to another user
    existing = db.query(User).filter(
        User.telegram_id == payload.telegram_id,
        User.id != user_id,
    ).first()
    if existing:
        raise HTTPException(status_code=409, detail="telegram_id_already_linked")

    row.telegram_id = payload.telegram_id
    row.telegram_username = payload.telegram_username or None
    db.commit()
    db.refresh(row)

    _audit(
        db,
        request,
        current_user,
        action="admin_user_link_telegram",
        success=True,
        details={
            "user_id": int(row.id),
            "username": row.username,
            "telegram_id": payload.telegram_id,
            "telegram_username": payload.telegram_username,
        },
    )
    return _to_response(row)


@router.post("/users/{user_id}/unlink-telegram", response_model=AdminUserResponse)
async def unlink_telegram_endpoint(
    user_id: int,
    request: Request,
    db: Session = Depends(get_db_session),
    current_user: dict = Depends(require_global_admin),
):
    row = get_user_by_id(db, user_id)
    if not row:
        raise HTTPException(status_code=404, detail="user_not_found")

    old_tg_id = row.telegram_id
    row.telegram_id = None
    row.telegram_username = None
    db.commit()
    db.refresh(row)

    _audit(
        db,
        request,
        current_user,
        action="admin_user_unlink_telegram",
        success=True,
        details={"user_id": int(row.id), "username": row.username, "old_telegram_id": old_tg_id},
    )
    return _to_response(row)


# ---------------------------------------------------------------- sessions


@router.get("/users/{user_id}/sessions")
async def list_user_sessions_endpoint(
    user_id: int,
    request: Request,
    db: Session = Depends(get_db_session),
    current_user: dict = Depends(require_global_admin),
) -> Dict[str, Any]:
    row = get_user_by_id(db, user_id)
    if not row:
        raise HTTPException(status_code=404, detail="user_not_found")
    try:
        sessions = await list_user_sessions(int(row.id))
    except Exception as exc:
        logger.error("Session store unavailable while listing sessions of user_id=%s: %s", user_id, exc)
        raise HTTPException(status_code=503, detail="session_store_unavailable") from exc
    current = _current_session_id(request) if int(row.id) == _actor_id(current_user) else None
    for item in sessions:
        item["current"] = bool(current and item.get("session_id") == current)
    return {"user_id": int(row.id), "sessions": sessions}


@router.post("/users/{user_id}/revoke-sessions")
async def revoke_user_sessions_endpoint(
    user_id: int,
    request: Request,
    db: Session = Depends(get_db_session),
    current_user: dict = Depends(require_global_admin),
) -> Dict[str, Any]:
    """Signs the account out everywhere. Permissions and Excel connections stay as they are."""
    row = get_user_by_id(db, user_id)
    if not row:
        raise HTTPException(status_code=404, detail="user_not_found")
    # Ending one's own sessions keeps the one the request came on: the
    # administrator is not thrown out of the panel in the middle of the work.
    keep = _current_session_id(request) if int(row.id) == _actor_id(current_user) else None
    try:
        ended = await revoke_user_sessions(int(row.id), except_session_id=keep)
    except Exception as exc:
        logger.error("Session store unavailable while ending sessions of user_id=%s: %s", user_id, exc)
        raise HTTPException(status_code=503, detail="session_store_unavailable") from exc
    _audit(
        db,
        request,
        current_user,
        action="admin_user_revoke_sessions",
        success=True,
        details={"user_id": int(row.id), "username": row.username, "sessions_ended": ended},
    )
    return {"ok": True, "user_id": int(row.id), "revoked": ended, "kept_current": bool(keep)}


# ---------------------------------------------------------------- audit and reference data

# What the account administration writes, and the sign-ins it is read next to.
_AUDIT_EXACT = ("login_success", "login_failed")
_AUDIT_PREFIXES = ACCOUNT_ADMINISTRATION_AUDIT


@router.get("/audit")
async def account_audit(
    limit: int = Query(100, ge=1, le=500),
    user_id: Optional[int] = Query(None, ge=1),
    db: Session = Depends(get_db_session),
    current_user: dict = Depends(require_global_admin),
) -> Dict[str, Any]:
    """Who changed which account and when. Passwords and tokens are never written to it."""
    from sqlalchemy import or_

    query = db.query(AccessAuditLog).filter(
        or_(
            AccessAuditLog.action.in_(_AUDIT_EXACT),
            *[AccessAuditLog.action.like(pattern, escape="\\") for pattern in _AUDIT_PREFIXES],
        )
    )
    if user_id is not None:
        # The account acted on is in the details; the actor is the row's user.
        needle = f'"user_id": {int(user_id)}'
        query = query.filter(
            or_(
                AccessAuditLog.user_id == int(user_id),
                AccessAuditLog.details_json.like(f"%{needle},%"),
                AccessAuditLog.details_json.like(f"%{needle}}}%"),
            )
        )
    rows = query.order_by(AccessAuditLog.created_at.desc(), AccessAuditLog.id.desc()).limit(limit).all()

    actor_ids = {int(row.user_id) for row in rows if row.user_id}
    names = {
        int(user.id): user.username
        for user in (db.query(User).filter(User.id.in_(actor_ids)).all() if actor_ids else [])
    }
    items: List[Dict[str, Any]] = []
    for row in rows:
        details: Dict[str, Any] = {}
        if row.details_json:
            try:
                loaded = json.loads(row.details_json)
                details = loaded if isinstance(loaded, dict) else {"value": loaded}
            except Exception:
                details = {}
        items.append(
            {
                "id": int(row.id),
                "action": row.action,
                "success": bool(row.success),
                "actor_id": int(row.user_id) if row.user_id else None,
                "actor": names.get(int(row.user_id)) if row.user_id else None,
                "ip_address": row.ip_address,
                "details": details,
                "created_at": row.created_at.isoformat() if row.created_at else None,
            }
        )
    return {"items": items}


_TAB_TITLES = {
    "dashboard": "Транзакции",
    "reference": "Справочник",
    "automation": "Автоматизация",
    "userbot": "Telegram",
    "logs": "Логи",
    "admin": "Аудит",
}


@router.get("/meta")
async def admin_meta(
    db: Session = Depends(get_db_session),
    current_user: dict = Depends(require_global_admin),
) -> Dict[str, Any]:
    """What the administration panel needs to draw its forms. No account data.

    The workspaces come from the registry, so one that is added there appears
    in the panel with its roles and nothing here changes. `checks` is the
    reference data for the Checks data restrictions, the only workspace that
    has any beyond a role.
    """
    folders: Dict[int, Dict[str, Any]] = {}
    for scope in db.query(AccessScope).order_by(AccessScope.id.asc()).all():
        payload = scope_to_public_dict(scope)
        folders[int(payload["id"])] = payload
    for scope in get_config_scopes(active_only=False):
        payload = config_scope_to_public_dict(scope)
        folders.setdefault(int(payload.get("id") or 0), payload)

    return {
        "me": {
            "id": _actor_id(current_user),
            "username": current_user.get("username"),
            "display_name": current_user.get("display_name"),
        },
        "workspaces": workspace_catalog(),
        "checks": {
            "tabs": [{"id": tab, "title": _TAB_TITLES.get(tab, tab)} for tab in ALL_TABS if tab != "admin"],
            "folders": [
                {"id": key, "name": str(item.get("name") or key), "is_active": bool(item.get("is_active", True))}
                for key, item in sorted(folders.items())
            ],
        },
        "password": {"min_length": int(PASSWORD_MIN_LENGTH)},
        "session_ttl_days": {
            "min": int(SESSION_TTL_DAYS_MIN),
            "max": int(SESSION_TTL_DAYS_MAX),
            "default": int(SESSION_TTL_DAYS_DEFAULT),
        },
    }
