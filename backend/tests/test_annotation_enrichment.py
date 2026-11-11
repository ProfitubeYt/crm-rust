"""
Filling in "Подробности" for receipts nobody has annotated.

The note this writes is a starting point for the customer, not a replacement for
their judgement — which is why it lands with `src='agent'`, the provenance tier
their own edit silently overrides and that never overwrites one. Most of these
tests exist to keep that true.
"""
from unittest.mock import AsyncMock, MagicMock, patch

import pytest

from services import annotation_enrichment_service as enrichment


def _run(coro):
    import asyncio

    return asyncio.get_event_loop_policy().new_event_loop().run_until_complete(coro)


class TestAnswerCleaning:
    def test_a_plain_answer_is_kept(self):
        assert enrichment._clean("супермаркет") == "супермаркет"

    def test_quotes_and_trailing_stops_are_removed(self):
        assert enrichment._clean('"мобильная связь".') == "мобильная связь"

    def test_only_the_first_line_survives(self):
        assert enrichment._clean("АЗС\nЭто заправочная станция") == "АЗС"

    def test_an_admission_of_ignorance_writes_nothing(self):
        """An empty cell is better than a confident guess: the customer reads
        this column to decide what a payment was for."""
        for answer in ("неизвестно", "Неизвестно.", "unknown", "n/a", "-", "", "   "):
            assert enrichment._clean(answer) is None, answer

    def test_a_long_answer_is_trimmed(self):
        cleaned = enrichment._clean("сеть супермаркетов " * 20)
        assert cleaned is not None
        assert len(cleaned) <= enrichment.MAX_DETAILS_LENGTH


class TestDescribeOperator:
    def test_blank_operator_short_circuits(self):
        assert _run(enrichment.describe_operator("   ")) is None

    def test_a_session_answer_is_used(self):
        # `from services import claude_cli_service` resolves through the package
        # attribute once `services` is imported, so patching sys.modules only
        # works if nothing has imported it yet — which is true alone and false in
        # a full run. Patch the real module's own functions instead.
        from services import claude_cli_service

        result = claude_cli_service.ClaudeResult(ok=True, text="супермаркет")
        with patch.object(enrichment, "USE_SESSIONS", True), \
             patch.object(claude_cli_service, "is_configured", return_value=True), \
             patch.object(claude_cli_service, "ask", AsyncMock(return_value=result)), \
             patch.object(enrichment, "_describe_via_snippets",
                          AsyncMock(return_value=None)) as fallback:
            assert _run(enrichment.describe_operator("KORZINKA")) == "супермаркет"
        # The session answered, so nothing should have reached the network.
        fallback.assert_not_awaited()

    def test_an_unavailable_session_falls_back_to_snippets(self):
        """The CLI is an external process; it must never be the only path."""
        from services import claude_cli_service

        busy = claude_cli_service.ClaudeResult(ok=False, error="busy", retryable=True)
        with patch.object(enrichment, "USE_SESSIONS", True), \
             patch.object(claude_cli_service, "is_configured", return_value=True), \
             patch.object(claude_cli_service, "ask", AsyncMock(return_value=busy)), \
             patch.object(enrichment, "_describe_via_snippets",
                          AsyncMock(return_value="платёжный сервис")) as fallback:
            assert _run(enrichment.describe_operator("PAYNET")) == "платёжный сервис"
        fallback.assert_awaited_once()

    def test_everything_failing_returns_none_not_an_error(self):
        with patch.object(enrichment, "USE_SESSIONS", False), \
             patch.object(enrichment, "_describe_via_snippets", AsyncMock(return_value=None)):
            assert _run(enrichment.describe_operator("ЧТО-ТО")) is None


class TestWritingTheNote:
    def test_disabled_by_default_writes_nothing(self):
        transaction = MagicMock(annotation_details=None, operator_raw="KORZINKA")
        with patch.object(enrichment, "ENABLED", False):
            assert _run(enrichment.enrich_transaction(MagicMock(), transaction)) is False

    def test_an_existing_note_is_left_alone(self):
        """Not merely unchanged — not even looked up, so no cost is incurred."""
        transaction = MagicMock(annotation_details="вп маг", operator_raw="KORZINKA")
        with patch.object(enrichment, "ENABLED", True), \
             patch.object(enrichment, "describe_operator", AsyncMock()) as describe:
            assert _run(enrichment.enrich_transaction(MagicMock(), transaction)) is False
        describe.assert_not_awaited()

    def test_the_note_is_written_as_agent(self, db_session):
        """`src='agent'` is what lets the customer overwrite it without a fight."""
        from datetime import datetime
        from decimal import Decimal

        from database.models import Transaction

        transaction = Transaction(
            raw_message="enrich-test",
            source_type="AUTO",
            source_chat_id=-900,
            source_message_id=9001,
            transaction_date=datetime(2026, 3, 1, 10, 0),
            amount=Decimal("-1000"),
            currency="UZS",
            operator_raw="KORZINKA TASHKENT",
            transaction_type="DEBIT",
            fingerprint="enrich-fp-1",
        )
        db_session.add(transaction)
        db_session.commit()

        with patch.object(enrichment, "ENABLED", True), \
             patch.object(enrichment, "describe_operator", AsyncMock(return_value="супермаркет")):
            changed = _run(enrichment.enrich_transaction(db_session, transaction))
        db_session.commit()

        assert changed is True
        assert transaction.annotation_details == "супермаркет"
        assert transaction.annotation_details_src == "agent"

    def test_it_cannot_overwrite_what_a_person_wrote(self, db_session):
        from datetime import datetime
        from decimal import Decimal

        from database.models import Transaction
        from services import annotation_service

        transaction = Transaction(
            raw_message="enrich-test-2",
            source_type="AUTO",
            source_chat_id=-900,
            source_message_id=9002,
            transaction_date=datetime(2026, 3, 2, 10, 0),
            amount=Decimal("-2000"),
            currency="UZS",
            operator_raw="KORZINKA TASHKENT",
            transaction_type="DEBIT",
            fingerprint="enrich-fp-2",
        )
        db_session.add(transaction)
        db_session.commit()

        annotation_service.apply_annotation_patch(
            db_session, transaction, {"details": "вп маг"}, source="manual",
        )
        db_session.commit()

        # Even called directly, past the has-a-note guard, the provenance rule holds.
        result = annotation_service.apply_annotation_patch(
            db_session, transaction, {"details": "супермаркет"}, source="agent",
        )
        db_session.commit()

        assert transaction.annotation_details == "вп маг"
        assert transaction.annotation_details_src == "manual"
        assert "details" in (result.get("skipped") or {})


class TestSweep:
    def test_disabled_sweep_does_nothing(self):
        with patch.object(enrichment, "ENABLED", False):
            assert _run(enrichment.sweep_unannotated(MagicMock()))["updated"] == 0

    def test_each_merchant_is_looked_up_once(self, db_session):
        """The answer for KORZINKA is the same on every receipt carrying it."""
        from datetime import datetime
        from decimal import Decimal

        from database.models import Transaction

        for index in range(3):
            db_session.add(Transaction(
                raw_message=f"sweep-{index}",
                source_type="AUTO",
                source_chat_id=-900,
                source_message_id=9100 + index,
                transaction_date=datetime(2026, 4, 1 + index, 12, 0),
                amount=Decimal("-100"),
                currency="UZS",
                operator_raw="KORZINKA TASHKENT",
                transaction_type="DEBIT",
                fingerprint=f"sweep-fp-{index}",
            ))
        db_session.commit()

        with patch.object(enrichment, "ENABLED", True), \
             patch.object(enrichment, "describe_operator",
                          AsyncMock(return_value="супермаркет")) as describe:
            summary = _run(enrichment.sweep_unannotated(db_session, limit=50))

        assert describe.await_count == 1
        assert summary["updated"] == 3
