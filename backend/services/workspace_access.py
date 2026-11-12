"""Workspace access: which Hub workspaces an account may open.

One account, one password, one session; the workspaces are a layer above the
existing role and data restrictions. Everything here reads the current user
row, so a change made by an administrator is seen by the next request of an
already running client.

The check lives in one place on purpose. ``required_workspace_for_path`` maps a
request path to the workspace it belongs to and defaults to Checks, so a new
Checks endpoint is protected without anyone remembering to protect it.
"""
from __future__ import annotations

from datetime import datetime, timezone
from typing import Any, Dict, Iterable, List, Optional

WORKSPACE_CHECKS = "checks"
WORKSPACE_TIME = "time"
ALL_WORKSPACES = (WORKSPACE_CHECKS, WORKSPACE_TIME)

TIME_ROLE_EMPLOYEE = "employee"
TIME_ROLE_VIEWER = "viewer"
TIME_ROLE_MANAGER = "manager"
# Weakest first: a role may do what every role before it may.
TIME_ROLES = (TIME_ROLE_EMPLOYEE, TIME_ROLE_VIEWER, TIME_ROLE_MANAGER)
# Roles that open the Time workspace on screen. An employee only marks
# attendance through the bot and sees nobody's timesheet.
TIME_UI_ROLES = frozenset({TIME_ROLE_VIEWER, TIME_ROLE_MANAGER})

# Requests that belong to the account itself or to its administration, and so
# to no workspace: signing in, the session, own 2FA, the launch gate, and the
# account administration an administrator must reach even with no workspace.
_HUB_PREFIXES = (
    "/api/auth",
    "/api/2fa",
    "/api/admin",
    "/api/security/app",
    "/api/security/system-status",
    "/api/security/sessions",
)
_TIME_PREFIX = "/api/time"


class WorkspaceError(ValueError):
    pass


def _under(path: str, prefix: str) -> bool:
    return path == prefix or path.startswith(prefix + "/")


def normalize_time_role(value: Any) -> Optional[str]:
    """``None``/empty/"none" mean no Time access; anything unknown is refused."""
    if value is None:
        return None
    text = str(value).strip().lower()
    if text in {"", "none", "null"}:
        return None
    if text not in TIME_ROLES:
        raise WorkspaceError("invalid_time_role")
    return text


def time_role_of(user: Any) -> Optional[str]:
    """The stored Time role of a user row; a value the code does not know is no role."""
    raw = getattr(user, "time_role", None)
    text = str(raw).strip().lower() if raw else ""
    return text if text in TIME_ROLES else None


def checks_access_of(user: Any) -> bool:
    """Checks access of a user row. A row from before the column is a Checks account."""
    raw = getattr(user, "checks_access", True)
    return True if raw is None else bool(raw)


def workspaces_of(user: Any) -> List[str]:
    """Workspaces the Hub lists for this row, in the order it lists them."""
    result: List[str] = []
    if checks_access_of(user):
        result.append(WORKSPACE_CHECKS)
    if time_role_of(user) in TIME_UI_ROLES:
        result.append(WORKSPACE_TIME)
    return result


def snapshot_workspaces(snapshot: Optional[Dict[str, Any]]) -> frozenset:
    """Workspaces of a permissions snapshot.

    A snapshot without the field was built by something that predates
    workspaces (a hand-made local-admin or internal one); such an account is a
    Checks account, exactly as it was before the field existed.
    """
    if not isinstance(snapshot, dict):
        return frozenset()
    raw = snapshot.get("workspaces")
    if raw is None:
        return frozenset({WORKSPACE_CHECKS})
    if not isinstance(raw, (list, tuple, set, frozenset)):
        return frozenset()
    return frozenset(str(item).strip().lower() for item in raw if str(item).strip())


def has_workspace(snapshot: Optional[Dict[str, Any]], workspace: str) -> bool:
    return workspace in snapshot_workspaces(snapshot)


def required_workspace_for_path(path: str) -> Optional[str]:
    """The workspace a request path belongs to; ``None`` for account-level paths."""
    normalized = "/" + str(path or "").lstrip("/")
    if any(_under(normalized, prefix) for prefix in _HUB_PREFIXES):
        return None
    if _under(normalized, _TIME_PREFIX):
        return WORKSPACE_TIME
    return WORKSPACE_CHECKS


def workspace_denied_detail(workspace: str) -> Dict[str, str]:
    # Clients key on `error`; the word "scope" must not appear here, the
    # desktop reads it as an expired period scope.
    return {"error": "workspace_forbidden", "workspace": workspace}


def time_role_allows(role: Optional[str], required: str) -> bool:
    if role not in TIME_ROLES or required not in TIME_ROLES:
        return False
    return TIME_ROLES.index(role) >= TIME_ROLES.index(required)


def _access_expired(user: Any, now: Optional[datetime] = None) -> bool:
    moment = getattr(user, "access_until", None)
    if moment is None:
        return False
    if moment.tzinfo is None:
        moment = moment.replace(tzinfo=timezone.utc)
    return moment <= (now or datetime.now(timezone.utc))


def time_bot_decision(user: Any, now: Optional[datetime] = None) -> Dict[str, Any]:
    """Whether the account a Telegram identity is linked to may act through the Time bot.

    ``user`` is the row found by Telegram ID, or ``None``. The answer carries a
    reason the bot can put into words and nothing that identifies anyone else.
    """
    if user is None:
        return {"allowed": False, "reason": "not_linked"}
    base = {
        "user_id": int(user.id),
        "username": str(user.username),
        "display_name": str(user.display_name or user.username),
    }
    if not bool(user.is_active):
        return {"allowed": False, "reason": "inactive", **base}
    role = time_role_of(user)
    if role is None:
        return {"allowed": False, "reason": "no_time_access", **base}
    # The same rule the API applies: an administrator is never locked out by an
    # expiry date, everyone else is.
    if _access_expired(user, now) and str(user.role or "").lower() != "admin":
        return {"allowed": False, "reason": "access_expired", **base}
    return {"allowed": True, "reason": "ok", "time_role": role, **base}


def describe_access(user: Any) -> List[str]:
    """Effective access in words, for the administrator's list (Russian UI text)."""
    lines: List[str] = []
    if not bool(getattr(user, "is_active", True)):
        lines.append("Учётная запись отключена")
    if checks_access_of(user):
        role = "администратор" if str(getattr(user, "role", "") or "").lower() == "admin" else "оператор"
        lines.append(f"Чеки: {role}")
    role = time_role_of(user)
    if role:
        lines.append("Учёт времени: " + TIME_ROLE_TITLES[role])
    if not checks_access_of(user) and not role:
        lines.append("Нет доступа к рабочим пространствам")
    return lines


TIME_ROLE_TITLES = {
    TIME_ROLE_EMPLOYEE: "сотрудник (только отметки через бота)",
    TIME_ROLE_VIEWER: "просмотр",
    TIME_ROLE_MANAGER: "руководитель",
}


def changed_workspaces(before: Iterable[str], after: Iterable[str]) -> Dict[str, List[str]]:
    old, new = set(before), set(after)
    return {"granted": sorted(new - old), "revoked": sorted(old - new)}
