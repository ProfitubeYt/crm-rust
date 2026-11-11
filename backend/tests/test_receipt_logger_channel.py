"""
How a receipt appears in the Telegram channel.

Three things were wrong and none of them raised an error, which is why they
survived: premium emoji were being silently dropped by Telegram, every receipt
produced two messages instead of one that updates, and a photo receipt showed
the literal "[Photo]" where its contents should be.
"""
import asyncio
from unittest.mock import AsyncMock, patch

import pytest

from services import receipt_logger
from services.receipt_logger import ReceiptEvent, _Renderer


def _run(coro):
    return asyncio.get_event_loop_policy().new_event_loop().run_until_complete(coro)


class TestPremiumEmoji:
    """
    Telegram documents `entities` as an alternative to `parse_mode`, not an
    addition: sending both means the entities are ignored. The renderer used to
    send both, so the premium emoji never actually reached anyone — the Unicode
    fallback is what showed. They now ride inside the HTML as <tg-emoji>.
    """

    def test_unicode_only_when_no_premium_map_configured(self):
        renderer = _Renderer()
        renderer.premium = {}
        text, entities = renderer.render(ReceiptEvent(kind="processed", transaction_id=1))
        assert "tg-emoji" not in text
        assert entities == []

    def test_premium_ids_are_wrapped_in_html(self):
        renderer = _Renderer()
        renderer.premium = {"success": "5237701891081234567"}
        text, entities = renderer.render(ReceiptEvent(kind="processed", transaction_id=7))
        assert '<tg-emoji emoji-id="5237701891081234567">✅</tg-emoji>' in text
        # Never alongside parse_mode — that is the bug being fixed.
        assert entities == []

    def test_a_slug_without_a_premium_id_keeps_its_unicode(self):
        renderer = _Renderer()
        renderer.premium = {"success": "123"}
        text, _ = renderer.render(ReceiptEvent(kind="failed", task_id=3, error_summary="x"))
        assert "❌" in text
        assert "tg-emoji" not in text


class TestMessageReuse:
    """
    "Принят в обработку" is a placeholder; the finished receipt should replace
    it. The id of the posted message was never captured, so it could not be.
    """

    def setup_method(self):
        receipt_logger._local_message_ids.clear()

    def test_received_posts_and_remembers(self):
        key = receipt_logger._message_key(-100, 55)
        with patch.object(receipt_logger._client, "send", AsyncMock(return_value=4242)) as send, \
             patch.object(receipt_logger, "_remember_message", AsyncMock()) as remember:
            _run(receipt_logger._publish("received", key, "текст", None))
        send.assert_awaited_once()
        remember.assert_awaited_once()
        assert remember.await_args.args[1] == 4242

    def test_processed_edits_the_existing_message(self):
        key = receipt_logger._message_key(-100, 56)
        with patch.object(receipt_logger, "_recall_message", AsyncMock(return_value=99)), \
             patch.object(receipt_logger._client, "edit", AsyncMock(return_value=True)) as edit, \
             patch.object(receipt_logger._client, "send", AsyncMock()) as send, \
             patch.object(receipt_logger, "_forget_message", AsyncMock()):
            _run(receipt_logger._publish("processed", key, "готово", None))
        edit.assert_awaited_once()
        assert edit.await_args.args[0] == 99
        send.assert_not_awaited()

    def test_a_failed_edit_falls_back_to_posting(self):
        """A deleted or expired message must not cost us the notification."""
        key = receipt_logger._message_key(-100, 57)
        with patch.object(receipt_logger, "_recall_message", AsyncMock(return_value=99)), \
             patch.object(receipt_logger._client, "edit", AsyncMock(return_value=False)), \
             patch.object(receipt_logger._client, "send", AsyncMock(return_value=100)) as send, \
             patch.object(receipt_logger, "_remember_message", AsyncMock()):
            _run(receipt_logger._publish("processed", key, "готово", None))
        send.assert_awaited_once()

    def test_no_remembered_message_means_a_plain_post(self):
        key = receipt_logger._message_key(-100, 58)
        with patch.object(receipt_logger, "_recall_message", AsyncMock(return_value=None)), \
             patch.object(receipt_logger._client, "edit", AsyncMock()) as edit, \
             patch.object(receipt_logger._client, "send", AsyncMock(return_value=7)) as send, \
             patch.object(receipt_logger, "_remember_message", AsyncMock()):
            _run(receipt_logger._publish("processed", key, "готово", None))
        edit.assert_not_awaited()
        send.assert_awaited_once()

    def test_a_failure_keeps_its_key_so_a_retry_can_rewrite_it(self):
        key = receipt_logger._message_key(-100, 59)
        with patch.object(receipt_logger, "_recall_message", AsyncMock(return_value=99)), \
             patch.object(receipt_logger._client, "edit", AsyncMock(return_value=True)), \
             patch.object(receipt_logger, "_forget_message", AsyncMock()) as forget:
            _run(receipt_logger._publish("failed", key, "ошибка", None))
        forget.assert_not_awaited()

    def test_key_is_none_without_a_source_message(self):
        assert receipt_logger._message_key(None, 5) is None
        assert receipt_logger._message_key(-100, None) is None


class TestRetryButtonSurvives:
    def test_failures_still_carry_the_reprocess_button(self):
        markup = receipt_logger._build_reply_markup(
            ReceiptEvent(kind="failed", task_id=1, chat_id=-100, message_id=42)
        )
        assert markup is not None
        button = markup["inline_keyboard"][0][0]
        assert button["callback_data"] == "rcpt:repro:-100:42"

    def test_success_carries_no_button(self):
        assert receipt_logger._build_reply_markup(
            ReceiptEvent(kind="processed", transaction_id=1, chat_id=-100, message_id=42)
        ) is None


class TestNoiseSuppression:
    def test_failure_cards_for_promotional_noise_are_suppressed(self):
        """Old noise already queued before filtering must not recreate log spam."""
        with patch.object(receipt_logger._client, "token", "bot-token"), \
             patch.object(receipt_logger._client, "chat_id", -100), \
             patch.object(receipt_logger._client, "send", AsyncMock(return_value=123)) as send:
            receipt_logger.log_failed(
                task_id=17,
                chat_id=915326936,
                message_id=417158135808,
                rejection_reason="parser_returned_none",
                error_summary="Parsing returned None: parser_returned_none",
                ocr_preview='🎉 Поздравляем! За транзакцию в акции "Сезон удачи 2.0" начислено 2 шанса.',
            )
            send.assert_not_called()


class TestVisionReceiptText:
    """
    TDLib substitutes "[Photo]" for a caption-less picture, and that string was
    what the channel showed and the transaction stored. The model now returns a
    one-line summary of what it read.
    """

    def test_summary_is_returned_alongside_the_fields(self):
        from decimal import Decimal
        from unittest.mock import MagicMock

        from parsers import vision_receipt_parser as vrp

        payload = {
            "amount": 40000, "currency": "UZS",
            "transaction_date_iso": "2026-01-04T11:24:00",
            "card_last_4": "6714", "operator_raw": "KORZINKA",
            "transaction_type": "DEBIT", "balance_after": None,
            "receiver_name": None, "receiver_card": None, "confidence": 0.9,
            "description": "Оплата в супермаркете KORZINKA",
        }
        provider = MagicMock()
        provider.complete_vision_json_sync.return_value = payload

        with patch.object(vrp, "AI_VISION_ENABLED", True), \
             patch.object(vrp, "get_vision_ai_provider", return_value=provider):
            result = vrp.parse_receipt_images(images_b64=["x"], mime_type="image/png")

        assert result["_vision_description"] == "Оплата в супермаркете KORZINKA"
        assert result["amount"] == Decimal("40000")

    def test_a_missing_summary_is_simply_absent(self):
        from unittest.mock import MagicMock

        from parsers import vision_receipt_parser as vrp

        provider = MagicMock()
        provider.complete_vision_json_sync.return_value = {
            "amount": 100, "currency": "UZS",
            "transaction_date_iso": "2026-01-04T11:24:00",
            "transaction_type": "DEBIT", "confidence": 0.5,
        }
        with patch.object(vrp, "AI_VISION_ENABLED", True), \
             patch.object(vrp, "get_vision_ai_provider", return_value=provider):
            result = vrp.parse_receipt_images(images_b64=["x"])

        assert "_vision_description" not in result

    def test_the_summary_is_not_a_transaction_column(self):
        """It is underscored precisely so it is copied deliberately, never by
        splatting the parse result into the model."""
        from database.models import Transaction

        assert not hasattr(Transaction, "_vision_description")


class TestMediaPlaceholders:
    """
    "[Photo]" is what TDLib substitutes for a caption-less picture. Treating it
    as real text is what put it on the transaction and in the channel — it beat
    the model's summary simply by being a non-empty string.
    """

    def test_placeholders_count_as_empty(self):
        from services.receipt_processor import _is_placeholder

        for value in ("", "   ", "[Photo]", "[photo]", "[IMAGE]", "[PDF] receipt.pdf", None):
            assert _is_placeholder(value) is True, value

    def test_real_captions_are_kept(self):
        from services.receipt_processor import _is_placeholder

        for value in ("оплата за парковку", "Перевод от Максима", "40000 UZS"):
            assert _is_placeholder(value) is False, value


class TestRedisClientOwnership:
    """
    The logger owns its Redis client instead of borrowing auth_bot_service's.

    That one is a module-global bound to whichever event loop first touched it,
    while this module runs its own loop on a dedicated thread — so every call
    raised "got Future attached to a different loop". The error was caught and
    logged, leaving only the in-process dict, which is useless when "received"
    and "processed" land in different Celery worker processes. The placeholder
    messages stayed in the channel for exactly that reason.
    """

    def test_client_is_cached_per_loop(self):
        import services.receipt_logger as rl

        rl._redis_client = None
        rl._redis_loop = None

        fake = object()

        async def first(loop_holder):
            with patch("redis.asyncio.from_url", return_value=fake):
                client = await rl._get_redis()
            loop_holder.append(rl._redis_loop)
            return client

        holder: list = []
        loop_a = asyncio.new_event_loop()
        try:
            client_a = loop_a.run_until_complete(first(holder))
        finally:
            loop_a.close()
        assert client_a is fake
        assert holder[0] is loop_a

        # A different loop must not reuse the client bound to the first one.
        second = object()

        async def again():
            with patch("redis.asyncio.from_url", return_value=second):
                return await rl._get_redis()

        loop_b = asyncio.new_event_loop()
        try:
            client_b = loop_b.run_until_complete(again())
        finally:
            loop_b.close()
        assert client_b is second

        rl._redis_client = None
        rl._redis_loop = None

    def test_a_redis_outage_still_leaves_the_in_process_copy(self):
        import services.receipt_logger as rl

        rl._local_message_ids.clear()
        key = rl._message_key(-5, 5)
        with patch.object(rl, "_get_redis", AsyncMock(side_effect=RuntimeError("down"))):
            _run(rl._remember_message(key, 4242))
            assert _run(rl._recall_message(key)) == 4242


class TestSkippedRetractsPlaceholder:
    def test_skipped_message_deletes_its_received_card(self):
        import asyncio

        key = receipt_logger._message_key(915326936, 418761408512)
        with patch.object(receipt_logger._client, "token", "bot-token"), \
             patch.object(receipt_logger._client, "chat_id", -100), \
             patch.object(receipt_logger, "_recall_message", AsyncMock(return_value=555)), \
             patch.object(receipt_logger, "_forget_message", AsyncMock()) as forget, \
             patch.object(receipt_logger._client, "delete", AsyncMock(return_value=True)) as delete:
            asyncio.run(receipt_logger._retract(key))
            delete.assert_awaited_once_with(555)
            forget.assert_awaited_once_with(key)
