"""
Reading a receipt straight off its image.

This is the fallback for receipts the text cascade cannot touch at all: a photo
of a paper slip, a screenshot, a PDF that is one big scan. Those arrive with no
extractable text, OCR returns noise or nothing, and every text parser downstream
is then working from an empty string — which is exactly how they ended up in the
dead-letter queue with "Parsing returned None".

Validation is deliberately not reimplemented here. The model's answer goes
through the same `TransactionSchema` and the same `_convert_schema` as the text
path, so a receipt read from a photo lands in the database in precisely the
shape one read from text does.
"""
import logging
from datetime import datetime
from typing import Any, Dict, List, Optional

from config.ai_models import AI_VISION_ENABLED, AI_VISION_MAX_PAGES
from parsers.ai_receipt_parser import ReceiptAiParser, TransactionSchema
from services.ai_provider import get_vision_ai_provider

logger = logging.getLogger(__name__)

SYSTEM_PROMPT = (
    "You read bank and payment receipts from Uzbekistan and return structured data. "
    "You are given the receipt as an image. Read every visible field carefully, "
    "including text in Uzbek, Russian and English. Do not guess values that are not "
    "visible in the image."
)

USER_PROMPT = """Read this payment receipt image and return a JSON object.

Context you can rely on:
- Amounts are in UZS unless another currency is printed. "so'm", "сум", "UZS" all mean UZS.
- Dates appear as DD.MM.YYYY or YYYY-MM-DD, usually with HH:MM.
- The operator is the merchant or payment gateway (Payme, Click, Paynet, Uzum, NBU, HUMO...).
- Card numbers are shown masked; keep only the last 4 digits.
- Transaction types:
  * DEBIT — payment, purchase, withdrawal (Оплата, To'lov, Pokupka, Spisanie)
  * CREDIT — top-up, refund (Пополнение, Popolnenie)
  * CONVERSION — currency exchange (Конверсия)
  * REVERSAL — cancellation (Отмена, OTMENA)

Return ONLY a JSON object with EXACTLY these keys, using null where the receipt
does not show a value (never omit a key):
- amount (number, required)
- currency (string, default "UZS")
- transaction_date_iso (string ISO 8601 YYYY-MM-DDTHH:MM:SS, required)
- card_last_4 (string|null)
- operator_raw (string|null)
- transaction_type (string: DEBIT, CREDIT, CONVERSION or REVERSAL, required)
- balance_after (number|null)
- receiver_name (string|null)
- receiver_card (string|null)
- confidence (number 0..1, required — how clearly the image showed these fields)
- description (string, required — one short Russian sentence naming what the
  receipt is: the merchant or service, and what was paid for. This is shown to a
  human instead of the picture, so write what they would need to recognise it.)

If the image is not a payment receipt at all, return {"amount": null, "confidence": 0}."""


def parse_receipt_images(
    *,
    images_b64: List[str],
    mime_type: str = "image/png",
    caption: str = "",
    fallback_datetime: Optional[datetime] = None,
) -> Optional[Dict[str, Any]]:
    """
    Return parsed receipt fields, or None if vision is off, the image is not a
    receipt, or the model could not read an amount.

    Raising is reserved for genuine transport failures, so the caller can tell a
    "this is not a receipt" from a "the gateway is down" — the first must not be
    retried, the second must.
    """
    if not AI_VISION_ENABLED:
        logger.info("Vision fallback disabled (AI_VISION_ENABLED is off)")
        return None
    if not images_b64:
        return None

    # A receipt is one page. The cap keeps a long PDF from being billed per page.
    pages = images_b64[:AI_VISION_MAX_PAGES]

    prompt = USER_PROMPT
    if caption and caption.strip():
        # The message caption often carries the operator or a note the image
        # itself does not show.
        prompt = f"{USER_PROMPT}\n\nMessage caption accompanying the receipt:\n{caption.strip()[:800]}"

    provider = get_vision_ai_provider()
    raw = provider.complete_vision_json_sync(
        images_b64=pages,
        prompt=prompt,
        system_prompt=SYSTEM_PROMPT,
        mime_type=mime_type,
    )

    if not isinstance(raw, dict) or raw.get("amount") in (None, ""):
        logger.info("Vision: no amount found in image (not a receipt?)")
        return None

    # Same validation and normalisation as the text path.
    helper = ReceiptAiParser(allow_without_api_key=True)
    enriched, date_source = helper._with_source_datetime(raw, fallback_datetime)

    try:
        parsed = TransactionSchema(**enriched)
    except Exception as exc:  # noqa: BLE001
        logger.warning("Vision output failed schema validation: %s", exc)
        return None

    result = helper._convert_schema(parsed, date_source=date_source)
    # Distinct from the text path's 'GPT' so vision-read receipts stay auditable.
    result["parsing_method"] = "GPT_VISION"

    # A photo receipt carries no text of its own, so everything downstream — the
    # stored raw_message, the Telegram channel card — had nothing to show but the
    # literal "[Photo]". The model's own one-line summary is what a human
    # actually needs there. Underscored: it is not a Transaction column, and the
    # worker copies it across explicitly.
    summary = str(raw.get("description") or "").strip()
    if summary:
        result["_vision_description"] = summary[:500]
    return result
