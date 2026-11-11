"""
Reading the CardXabar mini app.

The bot posts a receipt into the monitored chat, and that is all we see. Its
mini app holds more about the same transactions — the bot's own category
("переводы P2P", "коммунальные услуги", "продукты"), the service point, the
payment type — and the customer reads that detail when deciding what a payment
was for. Bringing it across means their "Подробности" column starts from
something real instead of blank.

This is the customer's own account and their own transactions; the account is
already logged in through TDLib, and the launch URL Telegram signs for it is
what authorises the read.

It is also a third party's private API, so it is treated as one:

  * Nothing here is load-bearing. Every failure returns a report instead of
    raising, because the mini app can change shape without warning and that must
    not take receipt processing down with it.
  * Matching reuses the same fingerprint the workbook import uses, rather than
    trusting their ids to mean anything on our side.
  * The category is written with `src='agent'` — a starting point the customer's
    own note silently overrides.

Endpoints, read off the app's own bundle:
    POST /api/webapp/auth/login                  {initData} -> accessToken
    POST /api/webapp/notification-history/search {fromDate, toDate, ...}
"""
from __future__ import annotations

import logging
import os
import urllib.parse
from datetime import date, datetime, timedelta
from decimal import Decimal
from typing import Any, Dict, List, Optional

import httpx
from sqlalchemy.orm import Session

from database.models import Transaction
from services import annotation_service

logger = logging.getLogger(__name__)

API_BASE = os.getenv("CARDXABAR_API_BASE", "https://cardxabar.uz").rstrip("/")
BOT_CHAT_ID = int(os.getenv("CARDXABAR_CHAT_ID", "915326936") or 915326936)
ENABLED = (os.getenv("CARDXABAR_SYNC_ENABLED") or "").strip().lower() in {"1", "true", "yes", "on"}
TIMEOUT = float(os.getenv("CARDXABAR_TIMEOUT", "45"))
PAGE_SIZE = 100
#: Their API reports money in tiyin; ours is in so'm.
AMOUNT_SCALE = Decimal("100")


class CardXabarError(RuntimeError):
    """Something in their API did not go as expected. Always caught by callers."""


def init_data_from_launch_url(launch_url: str) -> Optional[str]:
    """
    Pull the signed launch payload out of the URL fragment.

    Telegram puts it after the `#` on purpose so it never travels to a server in
    an HTTP request. It is also why the desktop's URL check had to stop
    rejecting fragments — every mini app carries one.
    """
    if not launch_url or "#" not in launch_url:
        return None
    fragment = launch_url.split("#", 1)[1]
    values = urllib.parse.parse_qs(fragment)
    return (values.get("tgWebAppData") or [None])[0]


async def _login(client: httpx.AsyncClient, init_data: str) -> str:
    response = await client.post(
        f"{API_BASE}/api/webapp/auth/login",
        json={"initData": init_data},
        headers={"Accept": "application/json", "Language": "ru"},
    )
    if response.status_code >= 400:
        raise CardXabarError(f"login_http_{response.status_code}")
    payload = response.json() or {}
    token = ((payload.get("result") or {}).get("accessToken")) or ""
    if not token:
        raise CardXabarError("login_no_token")
    return token


async def _search(
    client: httpx.AsyncClient,
    token: str,
    *,
    date_from: date,
    date_to: date,
    page: int,
) -> Dict[str, Any]:
    response = await client.post(
        f"{API_BASE}/api/webapp/notification-history/search",
        json={
            "cardId": None,
            "maskedPan": None,
            "fromDate": date_from.isoformat(),
            "toDate": date_to.isoformat(),
            "transactions": {"pageNumber": page, "pageSize": PAGE_SIZE},
        },
        headers={
            "Authorization": f"Bearer {token}",
            "Accept": "application/json",
            "Language": "ru",
        },
    )
    if response.status_code >= 400:
        raise CardXabarError(f"search_http_{response.status_code}")
    return response.json() or {}


def _rows_from(payload: Dict[str, Any]) -> List[Dict[str, Any]]:
    """Their envelope nests the page under result.transactions."""
    result = payload.get("result") if isinstance(payload.get("result"), dict) else payload
    transactions = (result or {}).get("transactions")
    if not isinstance(transactions, dict):
        return []
    rows = transactions.get("data")
    return [row for row in rows if isinstance(row, dict)] if isinstance(rows, list) else []


def _is_last_page(payload: Dict[str, Any]) -> bool:
    result = payload.get("result") if isinstance(payload.get("result"), dict) else payload
    transactions = (result or {}).get("transactions")
    if not isinstance(transactions, dict):
        # An unreadable page is the end of the road: better to stop than to keep
        # paging against someone else's API hoping it recovers.
        return True
    return bool(transactions.get("isLast", True))


def _first(row: Dict[str, Any], *names: str) -> Any:
    """Their field naming is not ours and is not guaranteed; try what we know."""
    for name in names:
        if row.get(name) not in (None, ""):
            return row[name]
    return None


def describe_row(row: Dict[str, Any]) -> Optional[str]:
    """
    The one line worth keeping: their category, and the service point if named.

    Everything else in their payload — amount, card, time — we already hold, and
    ours came from the receipt itself rather than a second-hand summary.
    """
    # Field names confirmed against a live response: category, merchantName,
    # merchantAddress, amount, balanceAfter, maskedPan, transactionDate,
    # isCredit, currency, smsMessageId.
    category = _first(row, "category", "categoryName")
    merchant = _first(row, "merchantName", "servicePoint", "terminalName")

    parts = [str(value).strip() for value in (category, merchant) if value]
    text = " · ".join(part for part in parts if part)
    return text[:200] or None


def _row_key(row: Dict[str, Any]) -> Optional[tuple]:
    """
    Match on what the receipt itself carried: amount, the minute, and the card.

    Their transaction ids mean nothing on our side, and the workbook import
    already proved this triple is enough to line the two sources up.
    """
    amount = _first(row, "amount", "sum")
    moment = _first(row, "transactionDate", "date")
    card = _first(row, "maskedPan", "cardMask")
    if amount is None or not moment:
        return None
    try:
        # Their amounts are in tiyin: `balanceAfter: 959943862` is 9 599 438.62
        # so'm. Ours are in so'm, straight off the receipt.
        value = abs(Decimal(str(amount))) / AMOUNT_SCALE
    except Exception:  # noqa: BLE001
        return None
    try:
        parsed = datetime.fromisoformat(str(moment).replace("Z", "+00:00")).replace(tzinfo=None)
    except (TypeError, ValueError):
        return None
    digits = "".join(ch for ch in str(card or "") if ch.isdigit())
    return (value, parsed.replace(second=0, microsecond=0), digits[-4:] if digits else "")


def _transaction_key(transaction: Transaction) -> Optional[tuple]:
    if transaction.amount is None or transaction.transaction_date is None:
        return None
    try:
        value = abs(Decimal(str(transaction.amount)))
    except Exception:  # noqa: BLE001
        return None
    card = "".join(ch for ch in str(transaction.card_last_4 or "") if ch.isdigit())
    return (
        value,
        transaction.transaction_date.replace(second=0, microsecond=0),
        card[-4:] if card else "",
    )


async def fetch_history(
    init_data: str,
    *,
    date_from: date,
    date_to: date,
    max_pages: int = 40,
) -> List[Dict[str, Any]]:
    """Every operation the mini app knows about in the window."""
    collected: List[Dict[str, Any]] = []
    async with httpx.AsyncClient(timeout=httpx.Timeout(TIMEOUT, connect=10.0)) as client:
        token = await _login(client, init_data)
        for page in range(1, max_pages + 1):
            payload = await _search(client, token, date_from=date_from, date_to=date_to, page=page)
            rows = _rows_from(payload)
            collected.extend(rows)
            if not rows or _is_last_page(payload):
                break
    return collected


async def sync_annotations(
    db: Session,
    init_data: str,
    *,
    days: int = 30,
    dry_run: bool = True,
    actor_user_id: Optional[int] = None,
) -> Dict[str, Any]:
    """
    Bring their categories across onto our receipts.

    Only fills an empty note. A category is a useful first guess at what a
    payment was for; the customer's own words are the answer, and the provenance
    rule keeps those safe.
    """
    date_to = date.today()
    date_from = date_to - timedelta(days=max(1, days))

    try:
        rows = await fetch_history(init_data, date_from=date_from, date_to=date_to)
    except Exception as exc:  # noqa: BLE001 - a third party's API is never fatal here
        logger.warning("CardXabar history fetch failed: %s", exc)
        return {"ok": False, "error": str(exc)[:200], "matched": 0, "updated": 0}

    by_key: Dict[tuple, Dict[str, Any]] = {}
    for row in rows:
        key = _row_key(row)
        if key:
            by_key.setdefault(key, row)

    candidates = (
        db.query(Transaction)
        .filter(
            Transaction.source_chat_id == BOT_CHAT_ID,
            Transaction.transaction_date >= datetime.combine(date_from, datetime.min.time()),
        )
        .all()
    )

    matched = 0
    updated = 0
    unmatched_rows = len(by_key)
    for transaction in candidates:
        key = _transaction_key(transaction)
        row = by_key.get(key) if key else None
        if row is None:
            continue
        matched += 1
        unmatched_rows -= 1
        # Let the provenance rule decide rather than skipping on any existing
        # value. The bank's own category for this exact transaction beats a note
        # the web lookup guessed from the merchant name, and both are 'agent'.
        # Anything a person wrote, or the customer's workbook supplied, is
        # untouchable either way.
        if (transaction.annotation_details_src or "") in ("manual", "import"):
            continue
        description = describe_row(row)
        if not description:
            continue
        if dry_run:
            updated += 1
            continue
        outcome = annotation_service.apply_annotation_patch(
            db, transaction, {"details": description},
            source="agent", actor_user_id=actor_user_id,
        )
        if outcome.get("changed"):
            updated += 1

    if not dry_run:
        db.commit()

    return {
        "ok": True,
        "dry_run": dry_run,
        "period": [date_from.isoformat(), date_to.isoformat()],
        "rows_from_mini_app": len(rows),
        "our_transactions": len(candidates),
        "matched": matched,
        "unmatched_in_mini_app": max(unmatched_rows, 0),
        "updated": updated,
    }
