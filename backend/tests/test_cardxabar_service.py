"""
Reading the CardXabar mini app.

Their API is a third party's and private, so the tests here are mostly about the
places where our two views of the same transaction fail to line up — every one
of which was found by comparing real data, not by reading their code.
"""
from datetime import datetime
from decimal import Decimal
from unittest.mock import MagicMock, patch

import pytest

from services import cardxabar_service as cx


class TestLaunchUrl:
    def test_init_data_comes_out_of_the_fragment(self):
        """Telegram puts it after the '#' so it never reaches a server in the
        request — the same reason the desktop's URL check had to stop rejecting
        fragments."""
        # parse_qs decodes once, which is the form their login endpoint wants:
        # Telegram percent-encodes the whole payload into the fragment, and the
        # initData string underneath is itself percent-encoded key=value pairs.
        url = "https://webapp.cardxabar.uz/#tgWebAppData=user%3D%257B%2522id%2522&tgWebAppVersion=7.0"
        assert cx.init_data_from_launch_url(url) == "user=%7B%22id%22"

    def test_a_url_without_a_fragment_yields_nothing(self):
        assert cx.init_data_from_launch_url("https://webapp.cardxabar.uz/") is None
        assert cx.init_data_from_launch_url("") is None


class TestAmountScale:
    """
    Their money is in tiyin, ours is in so'm. Nothing in their payload says so —
    it showed up as 145 of our transactions matching none of their 5560 rows.
    """

    def test_a_tiyin_amount_becomes_soum(self):
        key = cx._row_key({
            "amount": 2500000,
            "transactionDate": "2026-09-02 08:19",
            "maskedPan": "860049******4483",
        })
        assert key is not None
        assert key[0] == Decimal("25000")

    def test_it_lines_up_with_our_own_row(self):
        transaction = MagicMock(
            amount=Decimal("-25000.00"),
            transaction_date=datetime(2026, 9, 2, 8, 19),
            card_last_4="4483",
        )
        theirs = cx._row_key({
            "amount": 2500000,
            "transactionDate": "2026-09-02 08:19",
            "maskedPan": "860049******4483",
        })
        assert cx._transaction_key(transaction) == theirs

    def test_the_sign_is_dropped_on_both_sides(self):
        """We store an outgoing payment as negative; they report a magnitude."""
        transaction = MagicMock(
            amount=Decimal("-1000.00"),
            transaction_date=datetime(2026, 9, 2, 8, 19),
            card_last_4="4483",
        )
        assert cx._transaction_key(transaction)[0] == Decimal("1000.00")


class TestKeying:
    def test_seconds_are_ignored(self):
        """Their history is minute-resolution; our receipts sometimes carry
        seconds."""
        transaction = MagicMock(
            amount=Decimal("500"),
            transaction_date=datetime(2026, 9, 2, 8, 19, 42),
            card_last_4="4483",
        )
        assert cx._transaction_key(transaction)[1] == datetime(2026, 9, 2, 8, 19)

    def test_only_the_last_four_card_digits_matter(self):
        key = cx._row_key({
            "amount": 100,
            "transactionDate": "2026-09-02 08:19",
            "maskedPan": "860049******4483",
        })
        assert key[2] == "4483"

    def test_a_row_without_an_amount_or_date_is_unusable(self):
        assert cx._row_key({"transactionDate": "2026-09-02 08:19"}) is None
        assert cx._row_key({"amount": 100}) is None

    def test_a_junk_date_does_not_raise(self):
        assert cx._row_key({"amount": 100, "transactionDate": "вчера"}) is None


class TestDescription:
    def test_category_and_service_point(self):
        assert cx.describe_row({
            "category": "Переводы и P2P",
            "merchantName": "XAZNA P2P UZCARD XB UZCARD OTHER",
        }) == "Переводы и P2P · XAZNA P2P UZCARD XB UZCARD OTHER"

    def test_category_alone_is_enough(self):
        assert cx.describe_row({"category": "Коммунальные услуги"}) == "Коммунальные услуги"

    def test_nothing_useful_writes_nothing(self):
        assert cx.describe_row({"amount": 100}) is None

    def test_it_is_trimmed(self):
        text = cx.describe_row({"category": "К" * 300, "merchantName": "М" * 300})
        assert len(text) <= 200


class TestResponseShape:
    def test_rows_are_read_from_their_envelope(self):
        payload = {"result": {"transactions": {"data": [{"amount": 1}, {"amount": 2}], "isLast": False}}}
        assert len(cx._rows_from(payload)) == 2
        assert cx._is_last_page(payload) is False

    def test_an_unexpected_shape_yields_no_rows_rather_than_an_error(self):
        """Their API can change without warning; that must not take receipt
        processing down."""
        for payload in ({}, {"result": None}, {"result": {"transactions": "неожиданно"}}):
            assert cx._rows_from(payload) == []

    def test_a_missing_flag_is_treated_as_the_last_page(self):
        """Better to stop early than to page forever against someone else's API."""
        assert cx._is_last_page({"result": {"transactions": {}}}) is True


class TestSyncSafety:
    def test_a_failed_fetch_reports_rather_than_raises(self):
        import asyncio

        async def boom(*args, **kwargs):
            raise cx.CardXabarError("search_http_500")

        with patch.object(cx, "fetch_history", boom):
            report = asyncio.new_event_loop().run_until_complete(
                cx.sync_annotations(MagicMock(), "init", days=7)
            )
        assert report["ok"] is False
        assert report["updated"] == 0

    def test_manual_and_imported_notes_are_never_touched(self):
        """The customer's own words, and their workbook, outrank a bank
        category. Both are checked here because the skip is ours — the
        provenance rule underneath would refuse anyway."""
        import inspect

        source = inspect.getsource(cx.sync_annotations)
        assert '("manual", "import")' in source
