import json

from services.message_utils import (
    format_tdlib_message,
    is_telegram_noise,
    looks_like_receipt,
)


def test_format_tdlib_message_extracts_nested_content_text():
    raw = {
        "id": 123,
        "date": 1700000000,
        "content": {
            "@type": "messageText",
            "text": {"@type": "formattedText", "text": "💸 500000 UZS"},
        },
    }
    formatted = format_tdlib_message(raw)
    assert formatted is not None
    assert formatted["id"] == 123
    assert formatted["text"] == "💸 500000 UZS"


def test_format_tdlib_message_uses_caption_and_document_file():
    raw = {
        "id": 124,
        "date": 1700000000,
        "content": {
            "@type": "messageDocument",
            "caption": {"@type": "formattedText", "text": "Чек PDF"},
            "document": {
                "file_name": "check.pdf",
                "mime_type": "application/pdf",
                "document": {"id": 987654},
            },
        },
    }
    formatted = format_tdlib_message(raw)
    assert formatted is not None
    assert formatted["text"] == "Чек PDF"
    assert formatted["document"]["file_id"] == 987654
    assert formatted["document"]["mime_type"] == "application/pdf"


def test_looks_like_receipt_detects_new_keywords_and_emoji():
    assert looks_like_receipt("CardXabar summa: 250000 UZS karta: *6714", "") is True
    assert looks_like_receipt("🔴 random text without classic keywords", "") is True


def test_promotional_transaction_card_is_noise():
    text = """🎉 Поздравляем!
За транзакцию, совершённую в рамках акции "Сезон удачи 2.0", 561468******5982 на вашу карту начислено 2 шанса.
🕓 Время транзакции: 29.09.2026, 02:16
🔢 RRN: ***0726
🏆 Всего шансов: 61
🚀 Увеличьте свои шансы выиграть ценные призы!"""

    assert is_telegram_noise(text) is True
    assert looks_like_receipt(text, "") is False


def test_receipt_log_failure_card_is_noise():
    text = """❌ **#11926 · Чек не обработан**

**Не удалось обработать чек**
Нажмите «🔁 Повторно обработать», чтобы попробовать ещё раз.
📥 915326936

> 🎉 Поздравляем!
> За транзакцию, совершённую в рамках акции "Сезон удачи 2.0", 561468******5982 на вашу карту начислено 2 шанса.

*msg&#x20;*`915326936/417158135808`"""

    assert is_telegram_noise(text) is True
    assert looks_like_receipt(text, "") is False


def test_real_receipts_are_not_noise():
    assert is_telegram_noise(
        "Оплата HUMO: 250000 UZS, карта *6714, баланс 125000 UZS, RRN ***0726"
    ) is False
    assert looks_like_receipt(
        "Оплата HUMO: 250000 UZS, карта *6714, баланс 125000 UZS, RRN ***0726", ""
    ) is True


def test_looks_like_receipt_detects_pdf_message_from_raw_json():
    raw_json = json.dumps(
        {
            "content": {
                "@type": "messageDocument",
                "document": {
                    "mime_type": "application/pdf",
                    "document": {"id": 999},
                },
            }
        }
    )
    assert looks_like_receipt("", raw_json) is True


def test_register_receipt_incident_skips_noise_payload():
    """Noise payloads must not create incidents or raise errors."""
    from unittest.mock import MagicMock
    import sys, os
    sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))
    from workers.celery_worker import register_receipt_incident

    db = MagicMock()
    noise_text = (
        "\U0001f389 \u041f\u043e\u0437\u0434\u0440\u0430\u0432\u043b\u044f\u0435\u043c!\n"
        "\u0417\u0430 \u0442\u0440\u0430\u043d\u0437\u0430\u043a\u0446\u0438\u044e, "
        "\u0441\u043e\u0432\u0435\u0440\u0448\u0451\u043d\u043d\u0443\u044e \u0432 "
        "\u0440\u0430\u043c\u043a\u0430\u0445 \u0430\u043a\u0446\u0438\u0438 "
        '"\u0421\u0435\u0437\u043e\u043d \u0443\u0434\u0430\u0447\u0438 2.0"'
    )

    # Should return early without touching db or raising
    result = register_receipt_incident(
        db,
        chat_id=123,
        message_id=456,
        incident_type="failed_parse",
        payload={"raw_text": noise_text},
    )
    assert result is None
    # db.commit should NOT have been called since we returned early
    db.commit.assert_not_called()


# OCR of CardXabar's weekly picture, msg 915326936/418761408512 (2026-10-19).
SPENDING_REPORT_OCR = """Ваши расходы Pa Количество
12 Окт 2026 - 18 Окт 2026 СПЕрЕНАх
24 092 817 „
У м 46%
Продукты и
Прочее покупки
***8736
Переводы и P2P py
20 180 000 cym с [ М о
2.9% 2.4% 0.9%
83 ® 8 № Заправка Транспорт Онлайн-оплата
700 530 | 576 300 . — ДА 224999
® Аналитика
© Переводы и Р2Р 83.8%
На «Переводы и P2P» indi °
пришлось 83.8% всех @ Прочее 5.4%
расходов
© Продукты и покупки 4.6%
® Заправка 2.9%
® Транспорт 2.4%
@ Онлайн-оплата 0.9%"""


def test_weekly_spending_report_is_noise():
    from services.message_utils import is_spending_report

    assert is_spending_report(SPENDING_REPORT_OCR) is True
    assert is_telegram_noise("[Photo]\n\n" + SPENDING_REPORT_OCR) is True


def test_spending_report_survives_a_worse_ocr_read():
    """Lose the headline and the "всех расходов" line; the rest still gives it away."""
    from services.message_utils import is_spending_report

    garbled = "\n".join(
        line for line in SPENDING_REPORT_OCR.splitlines()
        if "Ваши расходы" not in line and "расходов" not in line
    )
    assert is_spending_report(garbled) is True


def test_real_receipts_are_not_spending_reports():
    from services.message_utils import is_spending_report

    for receipt in (
        "🔴 Pokupka ➖ 700 530.00 UZS 💳 ***5982 📍 METAN GAZ ISTIQBOL, Toshkent 🕓 18.10.26 16:56 💰 1 234 567.00 UZS",
        "💸 Операция ➖ 500.000,00 UZS 📍 MAVRID HUMO TO HUMO> 💳 HUMOCARD *8736 🕓 19.10.26 18:19 💵 2.345.000,00 UZS",
        "Онлайн-оплата Click 150 000 сум, карта *4483, комиссия 1%",
        "Чек: Переводы и P2P, 1 000 000 UZS, комиссия 0.5%",
    ):
        assert is_spending_report(receipt) is False, receipt
        assert is_telegram_noise(receipt) is False, receipt
