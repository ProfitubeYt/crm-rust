"""
Filling in "Подробности" for a receipt nobody has annotated yet.

The customer's own note for a transaction says what it was *for* — "вп маг",
"ст парковка", "офисн.инет". Half of that is predictable from the merchant and
is what the rule engine already covers; the rest needs to know what the merchant
actually is, which is not in our data at all.

So: look the operator up, and write one short line saying what it is. The
resulting note is a starting point for the human, not a replacement — which is
why it is written with `src='agent'`, the provenance tier that a person's own
edit silently overrides and that never overwrites one.

Two paths, deliberately:

  * A Claude Code session, which searches the web itself and reasons about what
    it finds. Better answers, one session per operator.
  * The existing keyless DuckDuckGo scrape plus a text completion, for sweeping
    the backlog — a thousand sessions to label a thousand old operators would be
    absurd.

Both are keyed by operator, not by transaction: the answer for KORZINKA is the
same on every receipt, so it is looked up once and reused.
"""
from __future__ import annotations

import logging
import os
from typing import Any, Dict, List, Optional

from sqlalchemy.orm import Session

from database.models import Transaction
from services import annotation_service

logger = logging.getLogger(__name__)

ENABLED = (os.getenv("ANNOTATION_ENRICHMENT_ENABLED") or "").strip().lower() in {"1", "true", "yes", "on"}
# Whether to spend a full agent session per operator, or use the cheap sweep.
USE_SESSIONS = (os.getenv("ANNOTATION_ENRICHMENT_USE_CLI") or "true").strip().lower() in {"1", "true", "yes", "on"}
MAX_DETAILS_LENGTH = 60

_PROMPT = (
    "Определи, что это за продавец или сервис в Узбекистане, по названию из "
    "банковского чека: «{operator}».\n"
    "Ответь ОДНОЙ короткой строкой на русском — чем он занимается, максимум 5 слов. "
    "Например: «супермаркет», «мобильная связь», «доставка еды», «АЗС», "
    "«коммунальные платежи».\n"
    "Не добавляй пояснений, кавычек и точки в конце. "
    "Если по названию понять невозможно — ответь ровно: неизвестно"
)


def _clean(answer: str) -> Optional[str]:
    """One short line, or nothing. A vague answer is worse than an empty cell."""
    text = (answer or "").strip()
    if not text:
        return None
    text = text.splitlines()[0].strip()
    # Quotes and a trailing stop can nest either way round (`"смысл".`), so peel
    # until nothing more comes off rather than assuming an order.
    previous = None
    while text != previous:
        previous = text
        text = text.strip().strip('"').strip("'").strip("«»").rstrip(".")
    if not text or text.lower() in {"неизвестно", "unknown", "n/a", "-"}:
        return None
    if len(text) > MAX_DETAILS_LENGTH:
        text = text[:MAX_DETAILS_LENGTH].rstrip()
    return text


async def describe_operator(operator_raw: str) -> Optional[str]:
    """
    What this merchant is, in a few words. None when it cannot be established.

    Tries a Claude Code session first — it searches the web itself — and falls
    back to the existing snippet-plus-completion path when the session layer is
    unavailable. The fallback is also what the bulk sweep uses directly.
    """
    operator = (operator_raw or "").strip()
    if not operator:
        return None

    if USE_SESSIONS:
        try:
            from services import claude_cli_service

            if claude_cli_service.is_configured():
                result = await claude_cli_service.ask(
                    _PROMPT.format(operator=operator),
                    model=claude_cli_service.MODEL_WORKER,
                    timeout=90,
                )
                if result.ok:
                    cleaned = _clean(result.text)
                    if cleaned:
                        return cleaned
                else:
                    logger.info("Enrichment session unavailable (%s); falling back", result.error)
        except Exception:  # noqa: BLE001 - enrichment is never worth an exception
            logger.debug("Claude session enrichment failed", exc_info=True)

    return await _describe_via_snippets(operator)


async def _describe_via_snippets(operator: str) -> Optional[str]:
    """The cheap path: keyless web snippets summarised by the text model."""
    try:
        from services.ai_provider import get_text_ai_provider
        from services.web_search import search_operator_snippets

        snippets = await search_operator_snippets(operator)
        provider = get_text_ai_provider()
        if not provider.enabled:
            return None

        context = "\n".join(snippets[:4])[:1200]
        prompt = _PROMPT.format(operator=operator)
        if context:
            prompt += f"\n\nНайдено в интернете:\n{context}"

        answer = await provider.complete_text(
            messages=[{"role": "user", "content": prompt}],
            temperature=0.0,
            max_tokens=60,
        )
        return _clean(answer)
    except Exception:  # noqa: BLE001
        logger.debug("Snippet enrichment failed for %s", operator, exc_info=True)
        return None


async def enrich_transaction(db: Session, transaction: Transaction, *, actor_user_id: Optional[int] = None) -> bool:
    """
    Give one receipt a starting note, if it has none. True when something was written.

    Safe to call on anything: the provenance rule in `apply_annotation_patch`
    refuses to touch a field a human filled in, so this can never overwrite the
    customer's own bookkeeping.
    """
    if not ENABLED:
        return False
    if getattr(transaction, "annotation_details", None):
        return False

    description = await describe_operator(getattr(transaction, "operator_raw", "") or "")
    if not description:
        return False

    result = annotation_service.apply_annotation_patch(
        db,
        transaction,
        {"details": description},
        source="agent",
        actor_user_id=actor_user_id,
    )
    return bool(result.get("changed"))


async def sweep_unannotated(
    db: Session,
    *,
    limit: int = 200,
    actor_user_id: Optional[int] = None,
) -> Dict[str, Any]:
    """
    Work through receipts that have no note yet, newest first.

    Grouped by operator so each merchant is looked up once however many receipts
    carry it — the answer for KORZINKA does not change between rows.
    """
    if not ENABLED:
        return {"enabled": False, "updated": 0}

    rows: List[Transaction] = (
        db.query(Transaction)
        .filter(
            Transaction.annotation_details.is_(None),
            Transaction.operator_raw.isnot(None),
            Transaction.operator_raw != "",
        )
        .order_by(Transaction.transaction_date.desc())
        .limit(max(1, min(int(limit), 5000)))
        .all()
    )

    by_operator: Dict[str, List[Transaction]] = {}
    for row in rows:
        key = annotation_service.normalize_code(row.operator_raw) or (row.operator_raw or "").strip().upper()
        by_operator.setdefault(key, []).append(row)

    updated = 0
    described = 0
    for operator_key, group in by_operator.items():
        description = await describe_operator(group[0].operator_raw or "")
        if not description:
            continue
        described += 1
        for row in group:
            outcome = annotation_service.apply_annotation_patch(
                db, row, {"details": description}, source="agent", actor_user_id=actor_user_id,
            )
            if outcome.get("changed"):
                updated += 1

    db.commit()
    return {
        "enabled": True,
        "scanned": len(rows),
        "operators": len(by_operator),
        "described": described,
        "updated": updated,
    }
