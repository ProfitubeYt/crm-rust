"""Shared message utilities.

Functions extracted from ``api.routes.telegram_client`` so they can be
reused by the reconciliation agent and other services.
"""
from __future__ import annotations

import json
import os
import re
from datetime import datetime, timezone
from typing import Any, Dict, Optional, Set
from zoneinfo import ZoneInfo


# ── Receipt detection constants ──────────────────────────────────────────────

RECEIPT_KEYWORDS_LOWER: Set[str] = {
    # Currency and money context
    "uzs",
    "usd",
    "sum",
    "сум",
    "сўм",
    "balans",
    "баланс",
    "balance",
    "dostupno",
    "остаток",
    # Card/bank markers
    "humo",
    "humocard",
    "uzcard",
    "cardxabar",
    "nbu card",
    # Receipt operation words (RU)
    "оплата",
    "пополнение",
    "перевод",
    "конверсия",
    "снятие наличных",
    # Receipt operation words (LAT)
    "oplata",
    "popolnenie",
    "pokupka",
    "spisanie",
    "transfer",
    "konversiya",
    # P2P markers
    "sender",
    "receiver",
    "отправител",
    "получател",
    # Common labels
    "receipt",
    "chek",
    "чек",
    "summa:",
    "karta:",
    "magazin:",
    # Payment systems
    "payme",
    "click",
    "apelsin",
    "terminal",
}
RECEIPT_EMOJI: Set[str] = {
    "💸",
    "💳",
    "📍",
    "🏪",
    "🕓",
    "🕘",
    "📅",
    "➖",
    "➕",
    "💰",
    "💵",
    "🔴",
    "🟢",
}
MIN_RECEIPT_TEXT_LEN: int = 20
MIN_TEXT_SPAM_THRESHOLD: int = 400
DEFAULT_METADATA_ONLY_CHAT_IDS: Set[int] = set()
ALWAYS_PROCESS_CHAT_IDS = {-1003547724919}
TASHKENT_TZ = ZoneInfo("Asia/Tashkent")

# Exact/substring markers for Telegram bot noise that superficially looks like
# a receipt (transaction/card/date words) but is promotional or a service card.
_SPAM_EXACT_SUBSTRINGS: Set[str] = {
    "не удалось обработать чек",
    "нажмите «🔁 повторно обработать»",
    "нажмите «🔁 повторно обработать», чтобы попробовать ещё раз",
    "поздравляем!",
    "сезон удачи",
    "увеличьте свои шансы выиграть",
    "msg&#x20;",
    "msg\u0020",
}
_SPAM_REGEXPS = (
    # "Сезон удачи 2.0", "акции "Сезон удачи 2.0""
    re.compile(r"акц\w+\s+«?\s*сезон\s+удачи", re.IGNORECASE),
    # "начислено 2 шанса", "Всего шансов: 61"
    re.compile(r"(?:начислено|всего)\s+\d+\s+шанс", re.IGNORECASE),
    # "561468******5982" — card-like runs followed by promotion markers
    re.compile(r"\d{6}\s*\*{4,}\s*\d{4}", re.IGNORECASE),
)


def receipt_min_datetime(now: Optional[datetime] = None) -> datetime:
    """Oldest receipt/source datetime accepted by Telegram ingestion.

    The default rolls forward automatically on January 1. Operators may pin an
    explicit ISO date with ``RECEIPT_MIN_DATE`` when a controlled backfill is
    required.
    """
    configured = os.getenv("RECEIPT_MIN_DATE", "").strip()
    if configured:
        try:
            parsed = datetime.fromisoformat(configured.replace("Z", "+00:00"))
        except ValueError:
            parsed = None
        if parsed is not None:
            if parsed.tzinfo is not None:
                parsed = parsed.astimezone(TASHKENT_TZ).replace(tzinfo=None)
            return parsed

    current = now or datetime.now(TASHKENT_TZ)
    if current.tzinfo is not None:
        current = current.astimezone(TASHKENT_TZ)
    return datetime(current.year, 1, 1)


def parse_message_datetime(value: Any) -> Optional[datetime]:
    """Normalize a TDLib timestamp/ISO datetime to naive Asia/Tashkent time."""
    if value in (None, ""):
        return None
    try:
        if isinstance(value, (int, float)) or (
            isinstance(value, str) and value.strip().isdigit()
        ):
            return (
                datetime.fromtimestamp(float(value), tz=timezone.utc)
                .astimezone(TASHKENT_TZ)
                .replace(tzinfo=None)
            )
        if isinstance(value, datetime):
            parsed = value
        else:
            parsed = datetime.fromisoformat(str(value).strip().replace("Z", "+00:00"))
        if parsed.tzinfo is not None:
            parsed = parsed.astimezone(TASHKENT_TZ).replace(tzinfo=None)
        return parsed
    except (TypeError, ValueError, OSError, OverflowError):
        return None


def is_receipt_datetime_allowed(value: Any) -> bool:
    parsed = parse_message_datetime(value)
    return parsed is not None and parsed >= receipt_min_datetime()


def metadata_only_chat_ids() -> Set[int]:
    raw = os.getenv("TG_METADATA_ONLY_CHAT_IDS", "")
    result = set(DEFAULT_METADATA_ONLY_CHAT_IDS)
    for token in raw.split(","):
        try:
            result.add(int(token.strip()))
        except (TypeError, ValueError):
            continue
    result.difference_update(ALWAYS_PROCESS_CHAT_IDS)
    return result


def is_metadata_only_chat(chat_id: Any) -> bool:
    try:
        return int(chat_id) in metadata_only_chat_ids()
    except (TypeError, ValueError):
        return False


# ── Receipt heuristic ────────────────────────────────────────────────────────

# The bank's weekly "Ваши расходы" infographic: a picture with no caption, so
# intake only sees "[Photo]" and queues it. After OCR it reads as category
# names, percentages and a few category totals (e.g. "Заправка 700 530") — no
# operation, card balance or counterparty. The parser rightly rejects it, and
# every Monday that used to post "Чек не обработан" to the log channel.
_SPENDING_REPORT_MARKERS = (
    "ваши расходы",
    "всех расходов",
    "аналитика",
    "переводы и p2p",
    "переводы и р2р",  # Cyrillic "р" as OCR sometimes reads it
    "продукты и покупки",
    "онлайн-оплата",
)


def is_spending_report(text: str) -> bool:
    """Detect the bank's periodic spending-analytics picture after OCR."""
    if not text:
        return False
    normalized = text.lower()
    if "ваши расходы" in normalized or "всех расходов" in normalized:
        return True
    hits = sum(1 for marker in _SPENDING_REPORT_MARKERS if marker in normalized)
    return hits >= 2 and normalized.count("%") >= 3


def is_telegram_noise(text: str) -> bool:
    """Detect bot promotional/service cards that look receipt-like.

    The sample that forced this filter advertises a bank promotion: it contains
    "транзакция", "карта" and a date, but no amount/currency/balance. Our broad
    emoji/keyword heuristics accept all three and send it to the receipt parser,
    which then posts "Чек не обработан" to the log channel. A second copy of the
    same text is quoted in that failure card, so old cards can re-enter parsing
    when a user presses "Повторно обработать".
    """
    if not text:
        return False
    normalized = text.strip().lower()
    if any(marker in normalized for marker in _SPAM_EXACT_SUBSTRINGS):
        return True
    if any(regex.search(normalized) for regex in _SPAM_REGEXPS):
        return True
    if is_spending_report(normalized):
        return True
    # Receipt-log failure cards. Long multi-part cards are not ordinary bank
    # receipts; match their structural footer and their retry CTA.
    if "повторно обработать" in normalized:
        return True
    if len(normalized) >= MIN_TEXT_SPAM_THRESHOLD and (
        "чек не обработан" in normalized or re.search(r"\bmsg\s+[-‑–—]?\s*\d+/\d+\b", normalized)
    ):
        return True
    return False

def looks_like_receipt(text: str, raw_json: str) -> bool:
    """Quick heuristic check if a history message might contain a receipt."""
    if raw_json:
        try:
            msg = json.loads(raw_json)
            content = msg.get("content") or {}
            content_type = content.get("@type", "")
            if content_type == "messageDocument":
                doc = content.get("document") or {}
                doc_file = doc.get("document") if isinstance(doc.get("document"), dict) else {}
                mime = str(doc.get("mime_type") or doc_file.get("mime_type") or "").lower()
                if mime == "application/pdf" or mime.startswith("image/"):
                    return True
            if content_type == "messagePhoto":
                return True
        except Exception:
            pass

    if not text or len(text) < MIN_RECEIPT_TEXT_LEN:
        return False
    if is_telegram_noise(text):
        return False

    if any(emoji in text for emoji in RECEIPT_EMOJI):
        return True

    text_lower = text.lower()
    return any(kw in text_lower for kw in RECEIPT_KEYWORDS_LOWER)


# ── Reply markup (inline keyboards / mini-app buttons) ───────────────────────

_INLINE_BUTTON_TYPE_MAP: Dict[str, str] = {
    "inlineKeyboardButtonTypeWebApp": "web_app",
    "inlineKeyboardButtonTypeUrl": "url",
    "inlineKeyboardButtonTypeLoginUrl": "login_url",
    "inlineKeyboardButtonTypeCallback": "callback",
    "inlineKeyboardButtonTypeCallbackWithPassword": "callback",
    "inlineKeyboardButtonTypeCallbackGame": "callback",
    "inlineKeyboardButtonTypeSwitchInline": "switch_inline",
    "inlineKeyboardButtonTypeBuy": "buy",
    "inlineKeyboardButtonTypeCopyText": "copy_text",
}

_KEYBOARD_BUTTON_TYPE_MAP: Dict[str, str] = {
    "keyboardButtonTypeWebApp": "web_app",
}


def _parse_reply_markup(rm: Optional[Dict[str, Any]]) -> Optional[Dict[str, Any]]:
    """Convert a raw TDLib ``ReplyMarkup`` object into the shared contract shape.

    Returns ``None`` when *rm* is falsy or not an inline/reply keyboard
    (``replyMarkupRemoveKeyboard``, ``replyMarkupForceReply``, etc. are ignored).
    """
    if not rm:
        return None
    rm_type = rm.get("@type")

    if rm_type == "replyMarkupInlineKeyboard":
        rows = []
        for raw_row in rm.get("rows") or []:
            out_row = []
            for button in raw_row or []:
                btn_type = (button.get("type") or {}).get("@type", "")
                kind = _INLINE_BUTTON_TYPE_MAP.get(btn_type, "other")
                url = None
                if kind in ("web_app", "url", "login_url"):
                    url = (button.get("type") or {}).get("url")
                out_row.append({
                    "text": button.get("text"),
                    "type": kind,
                    "url": url,
                })
            rows.append(out_row)
        return {"kind": "inline", "rows": rows}

    if rm_type == "replyMarkupShowKeyboard":
        rows = []
        for raw_row in rm.get("rows") or []:
            out_row = []
            for button in raw_row or []:
                btn_type = (button.get("type") or {}).get("@type", "")
                kind = _KEYBOARD_BUTTON_TYPE_MAP.get(btn_type, "other")
                url = None
                if kind == "web_app":
                    url = (button.get("type") or {}).get("url")
                out_row.append({
                    "text": button.get("text"),
                    "type": kind,
                    "url": url,
                })
            rows.append(out_row)
        return {"kind": "keyboard", "rows": rows}

    return None


# ── TDLib message formatting ─────────────────────────────────────────────────

def format_tdlib_message(data: Optional[Dict[str, Any]]) -> Optional[Dict[str, Any]]:
    """Convert raw TDLib message dict into a flat ChatMessage-like dict.

    Returns a dict with keys: id, date, is_outgoing, sender_id, text,
    document, photo, reply_markup.  Returns ``None`` when *data* is falsy.
    """
    if not data:
        return None
    iso_date: Optional[str] = None
    timestamp = data.get("date")
    if timestamp:
        try:
            iso_date = datetime.fromtimestamp(timestamp, tz=timezone.utc).isoformat().replace("+00:00", "Z")
        except Exception:
            iso_date = None

    content = data.get("content") or {}
    content_type = content.get("@type", "")

    # TDLib text may be in root "text", root formattedText object, or content.text.text.
    text: Optional[str] = data.get("text")
    if isinstance(text, dict):
        text = text.get("text")
    if not isinstance(text, str) or not text.strip():
        content_text = content.get("text")
        if isinstance(content_text, dict):
            text = content_text.get("text")
        elif isinstance(content_text, str):
            text = content_text

    # Caption fallback for document/photo messages.
    caption = content.get("caption")
    if (not text or not str(text).strip()) and caption:
        if isinstance(caption, dict):
            text = caption.get("text")
        elif isinstance(caption, str):
            text = caption

    document = data.get("document")
    if not document and content_type == "messageDocument":
        doc = content.get("document") or {}
        doc_file = doc.get("document") if isinstance(doc.get("document"), dict) else {}
        document = {
            "file_id": doc_file.get("id") or doc_file.get("file_id") or doc.get("id") or doc.get("file_id"),
            "file_name": doc.get("file_name"),
            "mime_type": doc.get("mime_type") or doc_file.get("mime_type"),
        }

    photo = data.get("photo")
    if not photo and content_type == "messagePhoto":
        photo_content = content.get("photo") or {}
        sizes = photo_content.get("sizes") or []
        if sizes:
            biggest = max(
                sizes,
                key=lambda size: (size.get("width", 0) * size.get("height", 0)),
            )
            photo_file = biggest.get("photo") if isinstance(biggest.get("photo"), dict) else biggest
            photo = {
                "file_id": photo_file.get("id") or photo_file.get("file_id"),
                "file_name": "photo.jpg",
                "mime_type": "image/jpeg",
            }

    sender_id: Any = data.get("sender_id")
    if isinstance(sender_id, dict):
        sender_id = (
            sender_id.get("user_id")
            or sender_id.get("chat_id")
            or sender_id.get("id")
        )

    return {
        "id": data.get("id"),
        "date": iso_date,
        "is_outgoing": data.get("is_outgoing", False),
        "sender_id": sender_id,
        "text": text,
        "document": document,
        "photo": photo,
        "reply_markup": _parse_reply_markup(data.get("reply_markup")),
    }


# ── Task payload builder ─────────────────────────────────────────────────────

def build_task_data_from_message(
    chat_id: int,
    message_id: int,
    message: Dict[str, Any],
) -> Dict[str, Any]:
    """Convert formatted TDLib message into task payload for Celery."""
    raw_text = message.get("text") or ""
    task_data: Dict[str, Any] = {
        "source_type": "AUTO",
        "source_chat_id": chat_id,
        "source_message_id": message_id,
        "source_received_at": message.get("date"),
        "raw_text": raw_text,
        "added_via": "tdlib",
    }
    document = message.get("document")
    if document and document.get("mime_type") == "application/pdf":
        task_data["document"] = {
            "file_id": document.get("file_id"),
            "file_name": document.get("file_name"),
            "mime_type": document.get("mime_type"),
            "caption": raw_text,
        }
    elif document and str(document.get("mime_type") or "").lower().startswith("image/"):
        task_data["image"] = {
            "file_id": document.get("file_id"),
            "file_name": document.get("file_name"),
            "mime_type": document.get("mime_type"),
            "caption": raw_text,
        }
    photo = message.get("photo")
    if photo and photo.get("file_id"):
        task_data["image"] = {
            "file_id": photo.get("file_id"),
            "file_name": photo.get("file_name") or "photo.jpg",
            "mime_type": photo.get("mime_type") or "image/jpeg",
            "caption": raw_text,
        }
    return task_data
