"""Workspaces of the CRM Hub and who may open them.

One account directory serves the whole CRM. Three things are decided about an
account, each on the server and each on its own:

1. The account itself: active, not expired, session not revoked.
2. CRM administration (``users.global_admin``): creating accounts and granting
   access. It belongs to no workspace and opens none; a role inside a
   workspace, however senior, does not give it.
3. Workspace access: a role per workspace. ``WORKSPACES`` below is the list.

Everything here reads the current user row, so a change made by an
administrator is seen by the next request of an already running client.

Adding a workspace is an entry in ``WORKSPACES`` and a nullable column for its
role. Sign-in, sessions, the administration API and the panel are driven by
the registry and need no change.

The path check lives in one place on purpose. ``required_workspace_for_path``
maps a request path to the workspace it belongs to and defaults to Checks, so
a new Checks endpoint is protected without anyone remembering to protect it.
"""
from __future__ import annotations

import os
from dataclasses import dataclass
from datetime import datetime, timezone
from typing import Any, Dict, FrozenSet, Iterable, List, Optional, Tuple

WORKSPACE_CHECKS = "checks"
WORKSPACE_TIME = "time"

CHECKS_ROLE_OPERATOR = "operator"
CHECKS_ROLE_ADMIN = "admin"

TIME_ROLE_EMPLOYEE = "employee"
TIME_ROLE_VIEWER = "viewer"
TIME_ROLE_MANAGER = "manager"
# Weakest first: a role may do what every role before it may.
TIME_ROLES = (TIME_ROLE_EMPLOYEE, TIME_ROLE_VIEWER, TIME_ROLE_MANAGER)
# Roles that open the Time workspace on screen. An employee only marks
# attendance through the bot and sees nobody's timesheet.
TIME_UI_ROLES = frozenset({TIME_ROLE_VIEWER, TIME_ROLE_MANAGER})


class WorkspaceError(ValueError):
    pass


@dataclass(frozen=True)
class WorkspaceRole:
    id: str
    title: str  # shown to the administrator
    hint: str


@dataclass(frozen=True)
class Workspace:
    id: str
    title: str
    description: str
    roles: Tuple[WorkspaceRole, ...]  # weakest first
    default_role: str
    # Roles that open the workspace on screen; the others act some other way.
    hub_roles: FrozenSet[str]
    # Request paths that belong to it. Empty for the default workspace, which
    # owns every path nobody else claims.
    api_prefixes: Tuple[str, ...]
    # Where the role is kept on the user row.
    role_column: str
    # A switch that takes access away without forgetting the role. Only Checks
    # has one: its role and data restrictions predate workspaces and stay put.
    grant_column: Optional[str] = None
    # Server settings the workspace cannot run without. While one is missing
    # the administrator is told that granting it will not open anything yet.
    requires_env: Tuple[str, ...] = ()

    def role_ids(self) -> Tuple[str, ...]:
        return tuple(role.id for role in self.roles)

    def role(self, role_id: Optional[str]) -> Optional[WorkspaceRole]:
        for role in self.roles:
            if role.id == role_id:
                return role
        return None

    def available(self) -> bool:
        return all(str(os.getenv(name, "")).strip() for name in self.requires_env)


CHECKS = Workspace(
    id=WORKSPACE_CHECKS,
    title="Чеки",
    description="Чеки, справочники, автоматизация и синхронизация с Excel.",
    roles=(
        WorkspaceRole(
            CHECKS_ROLE_OPERATOR,
            "Оператор",
            "Работает в пределах выданных вкладок, папок, источников и периодов.",
        ),
        WorkspaceRole(
            CHECKS_ROLE_ADMIN,
            "Администратор Чеков",
            "Все данные и настройки Чеков. Учётными записями CRM не управляет.",
        ),
    ),
    default_role=CHECKS_ROLE_OPERATOR,
    hub_roles=frozenset({CHECKS_ROLE_OPERATOR, CHECKS_ROLE_ADMIN}),
    api_prefixes=(),
    role_column="role",
    grant_column="checks_access",
)

TIME = Workspace(
    id=WORKSPACE_TIME,
    title="Учёт времени",
    description="Табель, отметки посещаемости и бот учёта времени.",
    roles=(
        WorkspaceRole(
            TIME_ROLE_EMPLOYEE,
            "Сотрудник",
            "Только отмечается через бота. Раздел в Hub не открывается.",
        ),
        WorkspaceRole(TIME_ROLE_VIEWER, "Просмотр", "Видит табель и отчёты, ничего не меняет."),
        WorkspaceRole(
            TIME_ROLE_MANAGER,
            "Руководитель",
            "Ведёт табель: отметки, сотрудники, календарь, закрытие месяца.",
        ),
    ),
    default_role=TIME_ROLE_EMPLOYEE,
    hub_roles=TIME_UI_ROLES,
    api_prefixes=("/api/time",),
    role_column="time_role",
    requires_env=("TIMELIST_INTERNAL_URL", "TIMELIST_API_TOKEN"),
)

# In the order the Hub lists them. The one without prefixes is the default.
WORKSPACES: Tuple[Workspace, ...] = (CHECKS, TIME)
DEFAULT_WORKSPACE = WORKSPACE_CHECKS

# Requests that belong to the account itself or to CRM administration, and so
# to no workspace: signing in, the session, own 2FA, the launch gate, account
# administration and the system-wide security settings.
_HUB_PREFIXES = (
    "/api/auth",
    "/api/2fa",
    "/api/admin",
    "/api/security/app",
    "/api/security/system-status",
    "/api/security/sessions",
    "/api/system-settings",
)


def _under(path: str, prefix: str) -> bool:
    return path == prefix or path.startswith(prefix + "/")


def _field(account: Any, name: str, default: Any = None) -> Any:
    """A field of a user row or of a permissions snapshot."""
    if isinstance(account, dict):
        return account.get(name, default)
    return getattr(account, name, default)


def _text(value: Any) -> str:
    return str(value).strip().lower() if value else ""


def workspace_ids() -> Tuple[str, ...]:
    """Every registered workspace, in the Hub's order."""
    return tuple(item.id for item in WORKSPACES)


def get_workspace(workspace_id: Any) -> Workspace:
    wanted = _text(workspace_id)
    for item in WORKSPACES:
        if item.id == wanted:
            return item
    raise WorkspaceError("unknown_workspace")


# ---------------------------------------------------------------- CRM administration


def is_global_admin(account: Any) -> bool:
    """Whether a user row or a snapshot is a CRM administrator. Absent means no."""
    return bool(_field(account, "global_admin", False))


def exempt_from_expiry(account: Any) -> bool:
    """Administrators are never locked out by an expiry date.

    Someone has to be able to extend everyone else's, and that includes their
    own. True for a CRM administrator and, as it always was, for a Checks
    administrator.
    """
    return is_global_admin(account) or _text(_field(account, "role")) == CHECKS_ROLE_ADMIN


# ---------------------------------------------------------------- roles


def normalize_workspace_role(workspace_id: Any, value: Any) -> Optional[str]:
    """``None``/empty/"none" mean no access; a role the workspace lacks is refused."""
    workspace = get_workspace(workspace_id)
    text = _text(value)
    if text in {"", "none", "null"}:
        return None
    if text not in workspace.role_ids():
        raise WorkspaceError("invalid_workspace_role")
    return text


def normalize_workspace_roles(values: Optional[Dict[Any, Any]]) -> Dict[str, Optional[str]]:
    """A ``{workspace: role or None}`` request, validated against the registry."""
    if values is None:
        return {}
    if not isinstance(values, dict):
        raise WorkspaceError("invalid_workspaces")
    return {
        get_workspace(key).id: normalize_workspace_role(key, value) for key, value in values.items()
    }


def workspace_role_of(user: Any, workspace_id: Any) -> Optional[str]:
    """The role a user row holds in a workspace; ``None`` when it has no access.

    A stored value the code does not know is no role, except in Checks, where
    anything that is not an administrator has always been an operator.
    """
    workspace = get_workspace(workspace_id)
    if workspace.grant_column:
        granted = getattr(user, workspace.grant_column, True)
        # A row from before the column is a Checks account.
        if granted is not None and not bool(granted):
            return None
    role = _text(getattr(user, workspace.role_column, None))
    if role in workspace.role_ids():
        return role
    return workspace.default_role if workspace.grant_column else None


def workspace_roles_of(user: Any) -> Dict[str, str]:
    """Every workspace the row has a role in, whether or not the Hub shows it."""
    result: Dict[str, str] = {}
    for workspace in WORKSPACES:
        role = workspace_role_of(user, workspace.id)
        if role:
            result[workspace.id] = role
    return result


def apply_workspace_role(user: Any, workspace_id: Any, role: Any) -> None:
    """Grant a role in a workspace or, with ``None``, take the workspace away."""
    workspace = get_workspace(workspace_id)
    normalized = normalize_workspace_role(workspace.id, role)
    if normalized is None:
        if workspace.grant_column:
            setattr(user, workspace.grant_column, False)
        else:
            setattr(user, workspace.role_column, None)
        return
    setattr(user, workspace.role_column, normalized)
    if workspace.grant_column:
        setattr(user, workspace.grant_column, True)


def time_role_of(user: Any) -> Optional[str]:
    return workspace_role_of(user, WORKSPACE_TIME)


def checks_access_of(user: Any) -> bool:
    return workspace_role_of(user, WORKSPACE_CHECKS) is not None


def workspaces_of(user: Any) -> List[str]:
    """Workspaces the Hub lists for this row, in the order it lists them."""
    roles = workspace_roles_of(user)
    return [item.id for item in WORKSPACES if roles.get(item.id) in item.hub_roles]


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


def snapshot_workspace_role(snapshot: Optional[Dict[str, Any]], workspace_id: str) -> Optional[str]:
    """The role a snapshot holds in a workspace, as the row had it when it was read."""
    roles = (snapshot or {}).get("workspace_roles")
    if not isinstance(roles, dict):
        return None
    return _text(roles.get(workspace_id)) or None


def has_workspace(snapshot: Optional[Dict[str, Any]], workspace: str) -> bool:
    return workspace in snapshot_workspaces(snapshot)


def required_workspace_for_path(path: str) -> Optional[str]:
    """The workspace a request path belongs to; ``None`` for account-level paths."""
    normalized = "/" + str(path or "").lstrip("/")
    if any(_under(normalized, prefix) for prefix in _HUB_PREFIXES):
        return None
    for workspace in WORKSPACES:
        if any(_under(normalized, prefix) for prefix in workspace.api_prefixes):
            return workspace.id
    return DEFAULT_WORKSPACE


def workspace_denied_detail(workspace: str) -> Dict[str, str]:
    # Clients key on `error`; the word "scope" must not appear here, the
    # desktop reads it as an expired period scope.
    return {"error": "workspace_forbidden", "workspace": workspace}


def time_role_allows(role: Optional[str], required: str) -> bool:
    if role not in TIME_ROLES or required not in TIME_ROLES:
        return False
    return TIME_ROLES.index(role) >= TIME_ROLES.index(required)


# ---------------------------------------------------------------- effective access


def _access_expired(user: Any, now: Optional[datetime] = None) -> bool:
    moment = getattr(user, "access_until", None)
    if moment is None:
        return False
    if moment.tzinfo is None:
        moment = moment.replace(tzinfo=timezone.utc)
    return moment <= (now or datetime.now(timezone.utc))


def _locked(user: Any, now: Optional[datetime] = None) -> bool:
    moment = getattr(user, "locked_until", None)
    if moment is None:
        return False
    if moment.tzinfo is None:
        moment = moment.replace(tzinfo=timezone.utc)
    return moment > (now or datetime.now(timezone.utc))


def account_state(user: Any, now: Optional[datetime] = None) -> str:
    """``active``, ``disabled`` or ``expired``: whether the account may be used at all."""
    if not bool(getattr(user, "is_active", True)):
        return "disabled"
    if _access_expired(user, now) and not exempt_from_expiry(user):
        return "expired"
    return "active"


def effective_access(user: Any, now: Optional[datetime] = None) -> Dict[str, Any]:
    """What the account can actually do right now, for the administrator.

    Grants are what was given; this is what they come to once the state of the
    account is taken into account. The same rules the requests are judged by.
    """
    state = account_state(user, now)
    usable = state == "active"
    administrator = is_global_admin(user)
    roles = workspace_roles_of(user)

    summary: List[str] = []
    if state == "disabled":
        summary.append("Учётная запись отключена — войти нельзя")
    elif state == "expired":
        summary.append("Срок доступа истёк — войти нельзя")
    elif _locked(user, now):
        summary.append("Временно заблокирована после неудачных попыток входа")
    if administrator:
        summary.append("Администратор CRM: учётные записи и доступ")

    workspaces: List[Dict[str, Any]] = []
    for workspace in WORKSPACES:
        role = workspace.role(roles.get(workspace.id))
        workspaces.append(
            {
                "id": workspace.id,
                "title": workspace.title,
                "granted": role is not None,
                "role": role.id if role else None,
                "role_title": role.title if role else None,
                # Usable now, and listed in the Hub (some roles act elsewhere).
                "effective": bool(role) and usable,
                "in_hub": bool(role) and usable and role.id in workspace.hub_roles,
            }
        )
        if role:
            summary.append(f"{workspace.title}: {role.title[0].lower()}{role.title[1:]}")
    if not roles:
        summary.append("Нет доступа к рабочим пространствам")

    return {
        "state": state,
        "locked": _locked(user, now),
        "can_sign_in": usable and not _locked(user, now),
        "global_admin": administrator and usable,
        "workspaces": workspaces,
        "summary": summary,
    }


def describe_access(user: Any) -> List[str]:
    """Effective access in words, for the administrator's list."""
    return list(effective_access(user)["summary"])


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
    # The same rule the API applies to an expiry date.
    if _access_expired(user, now) and not exempt_from_expiry(user):
        return {"allowed": False, "reason": "access_expired", **base}
    return {"allowed": True, "reason": "ok", "time_role": role, **base}


def workspace_catalog() -> List[Dict[str, Any]]:
    """The registry as the administration panel draws it. No account data."""
    return [
        {
            "id": workspace.id,
            "title": workspace.title,
            "description": workspace.description,
            "default_role": workspace.default_role,
            "available": workspace.available(),
            "roles": [
                {
                    "id": role.id,
                    "title": role.title,
                    "hint": role.hint,
                    "in_hub": role.id in workspace.hub_roles,
                }
                for role in workspace.roles
            ],
        }
        for workspace in WORKSPACES
    ]


def changed_workspaces(before: Iterable[str], after: Iterable[str]) -> Dict[str, List[str]]:
    old, new = set(before), set(after)
    return {"granted": sorted(new - old), "revoked": sorted(old - new)}
