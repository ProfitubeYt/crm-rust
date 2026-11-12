"""The Time workspace behind the same accounts as Checks.

Time («Тайм-лист») is its own program with its own database and no accounts.
It stays that way: this module is the only door to it. A request passes here
only with a live session, the Time workspace granted and a role that covers
the exact route; then it is forwarded over the internal network with the
server's own token. Time is never published to the internet and never sees a
user's credential.

The other direction is the bot. It runs inside Time and asks
``/internal/time/telegram/{id}`` whether the sender is an account that may mark
attendance. That path is deliberately outside ``/api``: the public proxy routes
only ``/api/*`` to this service, so it cannot be reached from the internet.
"""
from __future__ import annotations

import logging
import os
import re
import secrets
from typing import Any, Dict, List, Optional, Tuple
from urllib.parse import quote

import httpx
from fastapi import APIRouter, Depends, HTTPException, Request, Response
from sqlalchemy.orm import Session

from api.dependencies import get_current_app_user
from database.connection import get_db_session
from database.models import User
from services.access_control_service import write_audit_log
from services.internal_api_key_service import _PROXY_HEADERS, _ip_in_allowlist
from services.workspace_access import (
    TIME_ROLE_MANAGER,
    TIME_ROLE_VIEWER,
    WORKSPACE_TIME,
    snapshot_workspace_role,
    time_bot_decision,
    time_role_allows,
)

logger = logging.getLogger(__name__)

router = APIRouter(prefix="/api/time", tags=["Time"])
internal_router = APIRouter(prefix="/internal/time", tags=["TimeInternal"], include_in_schema=False)

_YM = r"\d{4}-\d{2}"
_DATE = r"\d{4}-\d{2}-\d{2}"
_ID = r"\d{1,18}"

# Every route of Time's API that the workspace uses, with the weakest role that
# may call it. Anything not listed is refused: Time's own /login and /logout
# (its shared password is not used here) and /events (the page polls /meta).
_ROUTES: Tuple[Tuple[str, str, str], ...] = (
    ("GET", r"meta", TIME_ROLE_VIEWER),
    ("GET", rf"month/{_YM}", TIME_ROLE_VIEWER),
    ("GET", rf"day/{_ID}/{_DATE}", TIME_ROLE_VIEWER),
    ("PUT", rf"marks/{_ID}/{_DATE}", TIME_ROLE_MANAGER),
    ("POST", rf"months/{_YM}/lock", TIME_ROLE_MANAGER),
    ("DELETE", rf"months/{_YM}/lock", TIME_ROLE_MANAGER),
    ("PUT", rf"months/{_YM}/limit", TIME_ROLE_MANAGER),
    ("GET", r"messages", TIME_ROLE_VIEWER),
    ("POST", r"messages", TIME_ROLE_MANAGER),
    ("POST", r"messages/import", TIME_ROLE_MANAGER),
    ("PATCH", rf"messages/{_ID}", TIME_ROLE_MANAGER),
    ("DELETE", rf"messages/{_ID}", TIME_ROLE_MANAGER),
    # Shows how a phrase would be read; writes nothing.
    ("POST", r"parse", TIME_ROLE_VIEWER),
    ("GET", r"employees", TIME_ROLE_VIEWER),
    ("POST", r"employees", TIME_ROLE_MANAGER),
    ("PUT", rf"employees/{_ID}", TIME_ROLE_MANAGER),
    ("DELETE", rf"employees/{_ID}", TIME_ROLE_MANAGER),
    ("GET", r"codes", TIME_ROLE_VIEWER),
    ("PUT", r"codes/[^/.\\]{1,8}", TIME_ROLE_MANAGER),
    ("GET", r"settings", TIME_ROLE_VIEWER),
    ("PUT", r"settings", TIME_ROLE_MANAGER),
    ("GET", r"calendar/\d{4}", TIME_ROLE_VIEWER),
    ("PUT", r"calendar/\d{4}/norms/\d{1,2}", TIME_ROLE_MANAGER),
    ("DELETE", r"calendar/\d{4}/norms/\d{1,2}", TIME_ROLE_MANAGER),
    ("PUT", rf"calendar/days/{_DATE}", TIME_ROLE_MANAGER),
    ("DELETE", rf"calendar/days/{_DATE}", TIME_ROLE_MANAGER),
    ("GET", r"periods", TIME_ROLE_VIEWER),
    ("POST", r"periods", TIME_ROLE_MANAGER),
    ("DELETE", rf"periods/{_ID}", TIME_ROLE_MANAGER),
    ("GET", r"tg/users", TIME_ROLE_VIEWER),
    ("POST", rf"tg/users/{_ID}/link", TIME_ROLE_MANAGER),
    ("GET", r"audit", TIME_ROLE_VIEWER),
    ("GET", rf"export/{_YM}(?:\.xlsx)?", TIME_ROLE_VIEWER),
    ("POST", r"import/xlsx", TIME_ROLE_MANAGER),
)
_COMPILED = tuple((method, re.compile(pattern), role) for method, pattern, role in _ROUTES)

# The largest body each kind of request may carry; Time's own limit for a
# workbook is the same 32 MiB.
_BODY_LIMITS = {"import/xlsx": 32 * 1024 * 1024, "messages/import": 4 * 1024 * 1024}
_DEFAULT_BODY_LIMIT = 256 * 1024
_FORWARDED_RESPONSE_HEADERS = ("content-type", "content-disposition")

_client: Optional[httpx.AsyncClient] = None


def _upstream_url() -> str:
    return str(os.getenv("TIMELIST_INTERNAL_URL", "")).strip().rstrip("/")


def _upstream_token() -> str:
    return str(os.getenv("TIMELIST_API_TOKEN", "")).strip()


def is_configured() -> bool:
    return bool(_upstream_url()) and bool(_upstream_token())


def _get_client() -> httpx.AsyncClient:
    global _client
    if _client is None:
        # trust_env off: a proxy variable in the container must not send the
        # server's token for Time somewhere else.
        _client = httpx.AsyncClient(
            timeout=httpx.Timeout(60.0, connect=5.0),
            follow_redirects=False,
            trust_env=False,
        )
    return _client


async def close_client() -> None:
    global _client
    if _client is not None:
        client, _client = _client, None
        await client.aclose()


def required_role(method: str, path: str) -> Optional[str]:
    """The weakest Time role for a route, or ``None`` when the route is not offered."""
    if any(mark in path for mark in ("..", "\\", "\x00")) or path != path.strip():
        return None
    for route_method, pattern, role in _COMPILED:
        if route_method == method and pattern.fullmatch(path):
            return role
    return None


def _journal_name(user: Dict[str, Any]) -> str:
    """The name Time writes into its change journal. Set here, never taken from the client."""
    username = str(user.get("username") or "").strip()
    display = str(user.get("display_name") or "").strip()
    name = username if not display or display == username else f"{display} ({username})"
    return quote(name[:60], safe="")


async def _read_body(request: Request, limit: int) -> bytes:
    declared = request.headers.get("content-length")
    if declared and declared.isdigit() and int(declared) > limit:
        raise HTTPException(status_code=413, detail={"error": "time_request_too_large", "limit": limit})
    chunks: List[bytes] = []
    size = 0
    async for chunk in request.stream():
        size += len(chunk)
        if size > limit:
            raise HTTPException(status_code=413, detail={"error": "time_request_too_large", "limit": limit})
        chunks.append(chunk)
    return b"".join(chunks)


def _unavailable(code: str, status: int = 502) -> HTTPException:
    return HTTPException(status_code=status, detail={"error": code})


@router.api_route("/{path:path}", methods=["GET", "POST", "PUT", "PATCH", "DELETE"])
async def time_gateway(
    path: str,
    request: Request,
    current_user: dict = Depends(get_current_app_user),
) -> Response:
    # `get_current_user` has already refused an account without the Time
    # workspace: this path is classified as Time in services.workspace_access.
    method = request.method.upper()
    needed = required_role(method, path)
    if needed is None:
        raise HTTPException(status_code=404, detail={"error": "time_route_not_found"})
    role = snapshot_workspace_role(current_user, WORKSPACE_TIME)
    if not time_role_allows(role, needed):
        raise HTTPException(status_code=403, detail={"error": "time_role_forbidden", "required": needed})
    if not is_configured():
        raise _unavailable("time_workspace_unavailable", 503)

    body = b""
    headers = {
        "Authorization": f"Bearer {_upstream_token()}",
        "X-Timelist-User": _journal_name(current_user),
        "Accept": "application/json",
    }
    if method in {"POST", "PUT", "PATCH"}:
        body = await _read_body(request, _BODY_LIMITS.get(path, _DEFAULT_BODY_LIMIT))
        headers["Content-Type"] = request.headers.get("content-type") or "application/json"

    url = f"{_upstream_url()}/api/{quote(path, safe='/')}"
    try:
        upstream = await _get_client().request(
            method,
            url,
            params=list(request.query_params.multi_items()),
            content=body if body else None,
            headers=headers,
        )
    except httpx.HTTPError as exc:
        logger.warning("Time is not answering (%s %s): %s", method, path, type(exc).__name__)
        raise _unavailable("time_upstream_unavailable") from exc

    # A refusal of the server's own token is a deployment fault. Passed on as a
    # 401 it would read as "your session ended" and sign the person out.
    if upstream.status_code in (401, 403):
        logger.error("Time refused the gateway token (%s %s): %s", method, path, upstream.status_code)
        raise _unavailable("time_upstream_refused")

    content = upstream.content
    if path == "meta" and upstream.status_code == 200:
        try:
            payload = upstream.json()
        except ValueError:
            payload = None
        if isinstance(payload, dict):
            # Who the page is working as; the page shows it and hides what the role cannot do.
            payload["hub"] = {
                "username": current_user.get("username"),
                "display_name": current_user.get("display_name"),
                "role": role,
                "can_edit": time_role_allows(role, TIME_ROLE_MANAGER),
            }
            return Response(
                content=_json_bytes(payload),
                status_code=200,
                media_type="application/json",
                headers={"Cache-Control": "no-store"},
            )

    out_headers = {"Cache-Control": "no-store"}
    for name in _FORWARDED_RESPONSE_HEADERS:
        value = upstream.headers.get(name)
        if value:
            out_headers[name] = value
    return Response(content=content, status_code=upstream.status_code, headers=out_headers)


def _json_bytes(payload: Any) -> bytes:
    import json

    return json.dumps(payload, ensure_ascii=False).encode("utf-8")


# ---------------------------------------------------------------- the bot's question


def _hub_key() -> str:
    return str(os.getenv("TIME_HUB_KEY", "")).strip()


def _require_time_bot(request: Request, db: Session) -> None:
    """Only Time itself, from inside the internal network, with its own key."""
    configured = _hub_key()
    if len(configured) < 24:
        raise HTTPException(status_code=503, detail={"error": "time_hub_key_not_configured"})
    given = str(request.headers.get("x-time-hub-key") or "").strip()
    direct = not any(request.headers.get(name) for name in _PROXY_HEADERS)
    host = request.client.host if request.client else None
    if given and secrets.compare_digest(given, configured) and direct and _ip_in_allowlist(host):
        return
    try:
        write_audit_log(
            db,
            action="time_hub_key_refused",
            success=False,
            ip_address=host or "unknown",
            details={"path": str(request.url.path), "direct": direct},
        )
    except Exception:
        pass
    raise HTTPException(status_code=403, detail={"error": "time_hub_key_refused"})


@internal_router.get("/telegram/{telegram_id}")
async def time_bot_identity(
    telegram_id: int,
    request: Request,
    db: Session = Depends(get_db_session),
) -> Dict[str, Any]:
    """May this Telegram sender act through the Time bot right now?

    Read from the account row at the moment of asking and never cached, so an
    account that is disabled or loses Time stops being accepted at once.
    """
    _require_time_bot(request, db)
    row = db.query(User).filter(User.telegram_id == int(telegram_id)).first() if telegram_id > 0 else None
    return time_bot_decision(row)
