"""User service for RBAC login and admin CRUD."""
from __future__ import annotations

import json
import logging
import os
from datetime import datetime, timedelta, timezone
from typing import Any, Dict, List, Optional, Tuple

from sqlalchemy import func
from sqlalchemy.orm import Session

from database.models import User
from services.access_control_service import hash_password, verify_password
from services.root_access_config_service import hash_password_argon2, verify_secret
from services.workspace_access import (
    WORKSPACE_CHECKS,
    WorkspaceError,
    apply_workspace_role,
    is_global_admin,
    normalize_workspace_roles,
    workspace_roles_of,
    workspaces_of,
)

LOGIN_MAX_ATTEMPTS = int(os.getenv("USER_LOGIN_MAX_ATTEMPTS", "5"))
LOGIN_LOCKOUT_MINUTES = int(os.getenv("USER_LOGIN_LOCKOUT_MINUTES", "15"))
PASSWORD_MIN_LENGTH = int(os.getenv("PASSWORD_MIN_LENGTH", "8"))
SESSION_TTL_DAYS_DEFAULT = int(os.getenv("USER_SESSION_TTL_DAYS_DEFAULT", "7"))
SESSION_TTL_DAYS_MIN = int(os.getenv("USER_SESSION_TTL_DAYS_MIN", "1"))
SESSION_TTL_DAYS_MAX = int(os.getenv("USER_SESSION_TTL_DAYS_MAX", "90"))

ROLE_ADMIN = "admin"
ROLE_OPERATOR = "operator"
VALID_ROLES = {ROLE_ADMIN, ROLE_OPERATOR}
ALL_TABS = ["dashboard", "reference", "automation", "userbot", "logs", "admin"]
DEFAULT_OPERATOR_TABS = ["dashboard"]

logger = logging.getLogger(__name__)


class UserServiceError(ValueError):
    pass


def _utcnow() -> datetime:
    return datetime.utcnow()


def _naive_utc(moment: datetime) -> datetime:
    """A stored time stamp as naive UTC, the form `_utcnow()` has.

    PostgreSQL returns `locked_until` with a time zone and SQLite without one;
    Python refuses to compare the two kinds.
    """
    if moment.tzinfo is None:
        return moment
    return moment.astimezone(timezone.utc).replace(tzinfo=None)


def _json_list(raw: Optional[str], *, default: Optional[List[Any]] = None) -> List[Any]:
    if not raw:
        return list(default or [])
    try:
        payload = json.loads(raw)
    except Exception:
        return list(default or [])
    if isinstance(payload, list):
        return payload
    return list(default or [])


def _normalize_tabs(tabs: Optional[List[Any]], *, role: str) -> List[str]:
    if role == ROLE_ADMIN:
        return list(ALL_TABS)
    raw = tabs if tabs is not None else list(DEFAULT_OPERATOR_TABS)
    normalized: List[str] = []
    for item in raw:
        value = str(item or "").strip().lower()
        if not value:
            continue
        if value == "transactions":
            value = "dashboard"
        if value in ALL_TABS and value != "admin":
            normalized.append(value)
    if "dashboard" not in normalized:
        normalized.insert(0, "dashboard")
    return sorted(set(normalized), key=lambda x: ALL_TABS.index(x) if x in ALL_TABS else 999)


def _normalize_int_list(values: Optional[List[Any]]) -> List[int]:
    normalized: List[int] = []
    for item in values or []:
        try:
            value = int(item)
        except Exception:
            continue
        normalized.append(value)
    return sorted(set(normalized))


def _normalize_forbidden_periods(periods: Optional[List[Dict[str, Any]]]) -> List[Dict[str, str]]:
    result: List[Dict[str, str]] = []
    for period in periods or []:
        if not isinstance(period, dict):
            continue
        date_from = str(period.get("from") or "").strip()
        date_to = str(period.get("to") or "").strip()
        if not date_from or not date_to:
            continue
        result.append({"from": date_from, "to": date_to})
    return result


def _validate_role(role: str) -> str:
    value = (role or ROLE_OPERATOR).strip().lower()
    if value not in VALID_ROLES:
        raise UserServiceError("invalid_role")
    return value


def _validate_username(username: str) -> str:
    value = (username or "").strip()
    if not value:
        raise UserServiceError("invalid_username")
    if len(value) > 50:
        raise UserServiceError("invalid_username")
    return value


def validate_password_strength(password: str) -> None:
    raw = str(password or "")
    if len(raw) < PASSWORD_MIN_LENGTH:
        raise UserServiceError("weak_password")
    has_upper = any(ch.isalpha() and ch.isupper() for ch in raw)
    has_lower = any(ch.isalpha() and ch.islower() for ch in raw)
    has_digit = any(ch.isdigit() for ch in raw)
    has_symbol = any(not ch.isalnum() for ch in raw)
    if not (has_upper and has_lower and has_digit and has_symbol):
        raise UserServiceError("weak_password")


def _normalize_session_ttl_days(value: Optional[Any]) -> int:
    try:
        ttl = int(value if value is not None else SESSION_TTL_DAYS_DEFAULT)
    except Exception:
        ttl = SESSION_TTL_DAYS_DEFAULT
    ttl = max(int(SESSION_TTL_DAYS_MIN), min(int(SESSION_TTL_DAYS_MAX), ttl))
    return ttl


def build_permissions_snapshot(user: User) -> Dict[str, Any]:
    role = (user.role or ROLE_OPERATOR).strip().lower()
    is_admin = role == ROLE_ADMIN

    allowed_tabs = _normalize_tabs(_json_list(user.allowed_tabs, default=DEFAULT_OPERATOR_TABS), role=role)
    allowed_folders = [] if is_admin else _normalize_int_list(_json_list(user.allowed_folders, default=[]))
    allowed_sources = [] if is_admin else _normalize_int_list(_json_list(user.allowed_sources, default=[]))
    forbidden_periods = [] if is_admin else _normalize_forbidden_periods(_json_list(user.forbidden_periods, default=[]))

    return {
        "id": int(user.id),
        "user_id": int(user.id),
        "username": user.username,
        "display_name": user.display_name or user.username,
        "role": role,
        "is_active": bool(user.is_active),
        "allowed_tabs": allowed_tabs,
        "allowed_folders": allowed_folders,
        "forbidden_periods": forbidden_periods,
        "allowed_sources": allowed_sources,
        "can_toggle_sources": True if is_admin else bool(user.can_toggle_sources),
        "permissions_version": int(user.permissions_version or 1),
        "totp_enabled": bool(user.totp_enabled),
        "totp_confirmed_at": user.totp_confirmed_at.isoformat() if user.totp_confirmed_at else None,
        "force_2fa": bool(user.force_2fa),
        "session_ttl_days": _normalize_session_ttl_days(user.session_ttl_days),
        # Entitlement, not lockout: how long this account may use the app at all.
        "access_until": user.access_until.isoformat() if getattr(user, "access_until", None) else None,
        "access_note": getattr(user, "access_note", None),
        "last_login_at": user.last_login_at.isoformat() if user.last_login_at else None,
        # CRM administration and workspace access. Read from the row on every
        # request, so a grant or a revocation needs no new token. `workspaces`
        # is what the Hub lists; `workspace_roles` has every role held.
        "global_admin": is_global_admin(user),
        "workspaces": workspaces_of(user),
        "workspace_roles": workspace_roles_of(user),
    }


def ensure_global_admin_remains(
    db: Session,
    user: User,
    *,
    global_admin: Optional[bool] = None,
    is_active: Optional[bool] = None,
) -> None:
    """Refuse a change that would leave the CRM without an active administrator.

    Accounts are administered only by CRM administrators, so losing the last
    one closes account management to everybody for good. Asked by every path
    that can take the right away or disable an account, the bot's commands
    included. Workspace roles are not part of it: an administrator restores
    any of them.
    """
    if not (is_global_admin(user) and bool(user.is_active)):
        return
    stays_admin = is_global_admin(user) if global_admin is None else bool(global_admin)
    stays_active = bool(user.is_active) if is_active is None else bool(is_active)
    if stays_admin and stays_active:
        return
    # Row-locked, so two administrators removing each other at the same moment
    # cannot both find the other one still there.
    others = (
        db.query(User)
        .filter(User.global_admin.is_(True), User.is_active.is_(True), User.id != user.id)
        .with_for_update()
        .all()
    )
    if not others:
        raise UserServiceError("last_admin")


def _requested_workspaces(workspaces: Optional[Dict[str, Any]]) -> Dict[str, Optional[str]]:
    try:
        return normalize_workspace_roles(workspaces)
    except WorkspaceError as exc:
        raise UserServiceError(str(exc)) from exc


def increment_permissions_version(user: User) -> None:
    current = int(user.permissions_version or 1)
    user.permissions_version = current + 1


def _verify_user_password(user: User, password_raw: str) -> bool:
    method = str(user.hash_method or "pbkdf2_sha256").strip().lower()
    if method in {"argon2", "argon2id"}:
        return verify_secret(
            password_raw,
            {
                "kind": "argon2id",
                "hash": str(user.password_hash or ""),
                "active": True,
            },
        )
    return verify_password(password_raw, str(user.password_hash or ""), str(user.salt or ""))


def _hash_user_password(password_raw: str) -> Tuple[str, str, str]:
    try:
        argon_hash = hash_password_argon2(password_raw)
        return argon_hash, "", "argon2id"
    except Exception:
        logger.debug("argon2 unavailable, fallback to pbkdf2 for user password hashing", exc_info=True)
        password_hash, salt = hash_password(password_raw)
        return password_hash, salt, "pbkdf2_sha256"


def _try_upgrade_user_password_hash(user: User, password_raw: str) -> bool:
    method = str(user.hash_method or "pbkdf2_sha256").strip().lower()
    if method not in {"pbkdf2", "pbkdf2_sha256", "legacy_pbkdf2"}:
        return False
    try:
        argon_hash = hash_password_argon2(password_raw)
    except Exception:
        logger.debug("argon2 upgrade skipped for user_id=%s", getattr(user, "id", None), exc_info=True)
        return False
    user.password_hash = argon_hash
    user.salt = ""
    user.hash_method = "argon2id"
    return True


def authenticate(db: Session, username: str, password: str) -> Tuple[Optional[User], Dict[str, Any]]:
    username_norm = (username or "").strip().lower()
    password_raw = password or ""
    if not username_norm or not password_raw:
        return None, {"error": "invalid_credentials"}

    user = (
        db.query(User)
        .filter(func.lower(User.username) == username_norm)
        .first()
    )
    if not user:
        return None, {"error": "invalid_credentials"}

    if not user.is_active:
        return None, {"error": "inactive_user"}

    now = _utcnow()
    locked_until = _naive_utc(user.locked_until) if user.locked_until else None
    if locked_until and locked_until > now:
        locked_seconds = max(1, int((locked_until - now).total_seconds()))
        return None, {"error": "locked", "locked_seconds": locked_seconds}

    if not _verify_user_password(user, password_raw):
        user.failed_attempts = int(user.failed_attempts or 0) + 1
        attempts_left = max(0, int(LOGIN_MAX_ATTEMPTS) - int(user.failed_attempts))
        if attempts_left <= 0:
            user.failed_attempts = int(LOGIN_MAX_ATTEMPTS)
            user.locked_until = now + timedelta(minutes=max(1, int(LOGIN_LOCKOUT_MINUTES)))
            db.commit()
            return None, {
                "error": "locked",
                "locked_seconds": max(1, int(LOGIN_LOCKOUT_MINUTES) * 60),
                "attempts_left": 0,
            }
        db.commit()
        return None, {"error": "invalid_credentials", "attempts_left": attempts_left}

    user.failed_attempts = 0
    user.locked_until = None
    user.last_login_at = now
    _try_upgrade_user_password_hash(user, password_raw)
    db.commit()
    db.refresh(user)
    return user, {"ok": True}


def list_users(db: Session, include_inactive: bool = True) -> List[User]:
    query = db.query(User)
    if not include_inactive:
        query = query.filter(User.is_active.is_(True))
    return query.order_by(User.id.asc()).all()


def get_user_by_id(db: Session, user_id: int) -> Optional[User]:
    return db.get(User, int(user_id))


def get_user_by_username(db: Session, username: str) -> Optional[User]:
    username_norm = (username or "").strip().lower()
    if not username_norm:
        return None
    return db.query(User).filter(func.lower(User.username) == username_norm).first()


def create_user(
    db: Session,
    *,
    username: str,
    password: str,
    role: str = ROLE_OPERATOR,
    display_name: Optional[str] = None,
    allowed_tabs: Optional[List[Any]] = None,
    allowed_folders: Optional[List[Any]] = None,
    forbidden_periods: Optional[List[Dict[str, Any]]] = None,
    allowed_sources: Optional[List[Any]] = None,
    can_toggle_sources: bool = False,
    force_2fa: bool = False,
    session_ttl_days: Optional[int] = None,
    is_active: bool = True,
    telegram_id: Optional[int] = None,
    telegram_username: Optional[str] = None,
    global_admin: bool = False,
    workspaces: Optional[Dict[str, Any]] = None,
) -> User:
    """Create an account.

    ``workspaces`` is ``{workspace: role}`` and states every grant: a
    workspace it does not name is not granted. Left out altogether, the
    account is what every account was before workspaces: a Checks account
    with ``role``. No other workspace and no CRM administration is implied.
    """
    username_norm = _validate_username(username)
    role_norm = _validate_role(role)
    grants = _requested_workspaces(workspaces)
    checks_granted = True
    if workspaces is not None:
        checks_role = grants.get(WORKSPACE_CHECKS)
        checks_granted = checks_role is not None
        if checks_role is not None:
            role_norm = checks_role

    if get_user_by_username(db, username_norm):
        raise UserServiceError("username_exists")

    if not password:
        raise UserServiceError("invalid_password")
    validate_password_strength(password)

    # Check telegram_id uniqueness if provided
    if telegram_id is not None:
        existing_tg = db.query(User).filter(User.telegram_id == telegram_id).first()
        if existing_tg:
            raise UserServiceError("telegram_id_already_linked")

    password_hash, salt, hash_method = _hash_user_password(password)
    user = User(
        username=username_norm,
        password_hash=password_hash,
        salt=salt,
        hash_method=hash_method,
        role=role_norm,
        display_name=(display_name or username_norm).strip()[:100],
        is_active=bool(is_active),
        allowed_tabs=json.dumps(_normalize_tabs(allowed_tabs, role=role_norm), ensure_ascii=False),
        allowed_folders=json.dumps(_normalize_int_list(allowed_folders), ensure_ascii=False),
        forbidden_periods=json.dumps(_normalize_forbidden_periods(forbidden_periods), ensure_ascii=False),
        allowed_sources=json.dumps(_normalize_int_list(allowed_sources), ensure_ascii=False),
        can_toggle_sources=bool(can_toggle_sources),
        force_2fa=bool(force_2fa),
        session_ttl_days=_normalize_session_ttl_days(session_ttl_days),
        failed_attempts=0,
        locked_until=None,
        permissions_version=1,
        telegram_id=telegram_id,
        telegram_username=(telegram_username or "").strip()[:255] or None,
        checks_access=checks_granted,
        global_admin=bool(global_admin),
    )
    for workspace_id, workspace_role in grants.items():
        if workspace_id != WORKSPACE_CHECKS:
            apply_workspace_role(user, workspace_id, workspace_role)
    db.add(user)
    db.commit()
    db.refresh(user)
    return user


def _parse_access_until(value: Any) -> Optional[datetime]:
    """
    Read an entitlement date. Empty means unlimited.

    A date with no time is taken as the end of that day rather than its start:
    "до 31 декабря" means the customer works through the 31st, and starting at
    midnight would cut a day off every licence.
    """
    if value in (None, "", "null"):
        return None
    if isinstance(value, datetime):
        moment = value
    else:
        text = str(value).strip()
        try:
            moment = datetime.fromisoformat(text.replace("Z", "+00:00"))
        except ValueError:
            raise ValueError("access_until must be an ISO 8601 date or datetime")
        if len(text) == 10:
            moment = moment.replace(hour=23, minute=59, second=59)
    if moment.tzinfo is None:
        moment = moment.replace(tzinfo=timezone.utc)
    return moment


def update_user(
    db: Session,
    user: User,
    *,
    role: Optional[str] = None,
    display_name: Optional[str] = None,
    is_active: Optional[bool] = None,
    allowed_tabs: Optional[List[Any]] = None,
    allowed_folders: Optional[List[Any]] = None,
    forbidden_periods: Optional[List[Dict[str, Any]]] = None,
    allowed_sources: Optional[List[Any]] = None,
    can_toggle_sources: Optional[bool] = None,
    force_2fa: Optional[bool] = None,
    session_ttl_days: Optional[int] = None,
    access_until: Optional[Any] = None,
    access_note: Optional[str] = None,
    password: Optional[str] = None,
    global_admin: Optional[bool] = None,
    workspaces: Optional[Dict[str, Any]] = None,
) -> User:
    """Change an account. ``None`` everywhere means "not mentioned".

    ``workspaces`` is ``{workspace: role or None}``: a role grants or changes
    it, ``None`` takes the workspace away, a workspace it does not name is left
    as it is. The Checks role is the ``role`` column, so naming it here is the
    same as passing ``role``; a Checks role that is taken away is remembered.
    """
    grants = _requested_workspaces(workspaces)
    checks_access: Optional[bool] = None
    if WORKSPACE_CHECKS in grants:
        checks_role = grants[WORKSPACE_CHECKS]
        checks_access = checks_role is not None
        if checks_role is not None:
            # Named again unchanged, the role is not a change: a new
            # permissions version would sign the person out for nothing.
            current_role = str(user.role or ROLE_OPERATOR).strip().lower()
            role = checks_role if checks_role != current_role else None

    new_role = _validate_role(role) if role is not None else (user.role or ROLE_OPERATOR)
    ensure_global_admin_remains(db, user, global_admin=global_admin, is_active=is_active)

    permissions_changed = False

    if role is not None and user.role != new_role:
        user.role = new_role
        permissions_changed = True

    if display_name is not None:
        user.display_name = display_name.strip()[:100] if str(display_name).strip() else user.username

    if is_active is not None and bool(user.is_active) != bool(is_active):
        user.is_active = bool(is_active)
        permissions_changed = True

    if can_toggle_sources is not None and bool(user.can_toggle_sources) != bool(can_toggle_sources):
        user.can_toggle_sources = bool(can_toggle_sources)
        permissions_changed = True

    if force_2fa is not None and bool(user.force_2fa) != bool(force_2fa):
        user.force_2fa = bool(force_2fa)
        permissions_changed = True

    if session_ttl_days is not None:
        normalized_ttl = _normalize_session_ttl_days(session_ttl_days)
        if int(user.session_ttl_days or SESSION_TTL_DAYS_DEFAULT) != normalized_ttl:
            user.session_ttl_days = normalized_ttl
            permissions_changed = True

    if access_until is not None:
        # An empty string clears the date, which is how an admin grants
        # unlimited access again — distinct from "not mentioned in this patch".
        user.access_until = _parse_access_until(access_until)
        permissions_changed = True

    if access_note is not None:
        user.access_note = (str(access_note).strip()[:200] or None)

    # CRM administration and workspace grants deliberately leave
    # permissions_version alone. The row is read on every request, so the
    # change is already in force for a running client, and bumping the version
    # would sign the person out of the other workspace and cut their Excel
    # connections for no gain.
    if global_admin is not None:
        user.global_admin = bool(global_admin)

    if checks_access is not None:
        user.checks_access = checks_access

    for workspace_id, workspace_role in grants.items():
        if workspace_id != WORKSPACE_CHECKS:
            apply_workspace_role(user, workspace_id, workspace_role)

    if any(value is not None for value in (allowed_tabs, role)):
        tabs = _normalize_tabs(allowed_tabs if allowed_tabs is not None else _json_list(user.allowed_tabs), role=new_role)
        user.allowed_tabs = json.dumps(tabs, ensure_ascii=False)
        permissions_changed = True

    if allowed_folders is not None:
        user.allowed_folders = json.dumps(_normalize_int_list(allowed_folders), ensure_ascii=False)
        permissions_changed = True

    if forbidden_periods is not None:
        user.forbidden_periods = json.dumps(_normalize_forbidden_periods(forbidden_periods), ensure_ascii=False)
        permissions_changed = True

    if allowed_sources is not None:
        user.allowed_sources = json.dumps(_normalize_int_list(allowed_sources), ensure_ascii=False)
        permissions_changed = True

    if password is not None:
        if not password:
            raise UserServiceError("invalid_password")
        validate_password_strength(password)
        password_hash, salt, hash_method = _hash_user_password(password)
        user.password_hash = password_hash
        user.salt = salt
        user.hash_method = hash_method
        user.failed_attempts = 0
        user.locked_until = None
        permissions_changed = True

    if permissions_changed:
        increment_permissions_version(user)

    db.commit()
    db.refresh(user)
    return user


def reset_password(db: Session, user: User, password: str) -> User:
    return update_user(db, user, password=password)


def unlock_user(db: Session, user: User) -> User:
    user.failed_attempts = 0
    user.locked_until = None
    db.commit()
    db.refresh(user)
    return user


def deactivate_user(db: Session, user: User) -> User:
    if not user.is_active:
        return user
    ensure_global_admin_remains(db, user, is_active=False)
    user.is_active = False
    increment_permissions_version(user)
    db.commit()
    db.refresh(user)
    return user
