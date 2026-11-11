"""
Tests for the vision fallback: reading a receipt off its image.

The cases that matter here are the ones that were silently broken before this
path existed — a picture receipt producing no text, and therefore no parse — and
the ones that would silently break it again: sending base64 through the JSON
keyword helper, or letting a vision result reach the database in a different
shape than a text result.
"""
import base64
from datetime import datetime
from unittest.mock import MagicMock, patch

import pytest

from parsers import vision_receipt_parser as vrp
from services.ai_provider import OpenAICompatibleTextProvider

PNG_B64 = base64.b64encode(b"\x89PNG\r\n\x1a\n fake").decode("ascii")

GOOD_RESPONSE = {
    "amount": 40000,
    "currency": "UZS",
    "transaction_date_iso": "2026-01-04T11:24:00",
    "card_last_4": "6714",
    "operator_raw": "KORZINKA TASHKENT",
    "transaction_type": "DEBIT",
    "balance_after": 1250000,
    "receiver_name": None,
    "receiver_card": None,
    "confidence": 0.93,
}


def _provider_returning(payload):
    provider = MagicMock()
    provider.complete_vision_json_sync.return_value = payload
    return provider


class TestVisionParser:
    def test_disabled_by_default_returns_none(self):
        """Vision costs money per receipt; it must not run unless switched on."""
        with patch.object(vrp, "AI_VISION_ENABLED", False):
            assert vrp.parse_receipt_images(images_b64=[PNG_B64]) is None

    def test_reads_a_receipt(self):
        with patch.object(vrp, "AI_VISION_ENABLED", True), \
             patch.object(vrp, "get_vision_ai_provider", return_value=_provider_returning(GOOD_RESPONSE)):
            result = vrp.parse_receipt_images(images_b64=[PNG_B64])

        assert result is not None
        assert result["amount"] == 40000
        assert result["operator_raw"] == "KORZINKA TASHKENT"
        assert result["card_last_4"] == "6714"
        assert result["transaction_type"] == "DEBIT"
        # Distinct from the text path so vision-read receipts stay auditable.
        assert result["parsing_method"] == "GPT_VISION"

    def test_result_shape_matches_the_text_path(self):
        """
        A receipt read from a photo must land in the same shape as one read from
        text — otherwise only the rarer path is ever exercised and it drifts.
        """
        with patch.object(vrp, "AI_VISION_ENABLED", True), \
             patch.object(vrp, "get_vision_ai_provider", return_value=_provider_returning(GOOD_RESPONSE)):
            result = vrp.parse_receipt_images(images_b64=[PNG_B64])

        for field in (
            "amount", "currency", "transaction_type", "card_last_4", "operator_raw",
            "transaction_date", "balance_after", "parsing_method", "parsing_confidence",
            "date_source",
        ):
            assert field in result, f"missing {field}"
        assert isinstance(result["transaction_date"], datetime)
        assert result["transaction_date"].tzinfo is None, "naive local time expected"

    def test_not_a_receipt_returns_none(self):
        with patch.object(vrp, "AI_VISION_ENABLED", True), \
             patch.object(vrp, "get_vision_ai_provider",
                          return_value=_provider_returning({"amount": None, "confidence": 0})):
            assert vrp.parse_receipt_images(images_b64=[PNG_B64]) is None

    def test_malformed_model_output_returns_none_not_raises(self):
        """Garbage from the model is a parse failure, not a crash."""
        bad = dict(GOOD_RESPONSE, transaction_type="СОВЕРШЕННО НЕ ТИП", amount="abc")
        with patch.object(vrp, "AI_VISION_ENABLED", True), \
             patch.object(vrp, "get_vision_ai_provider", return_value=_provider_returning(bad)):
            assert vrp.parse_receipt_images(images_b64=[PNG_B64]) is None

    def test_transport_failure_propagates(self):
        """
        A gateway outage must be distinguishable from 'not a receipt': the first
        has to be retried, the second must not.
        """
        provider = MagicMock()
        provider.complete_vision_json_sync.side_effect = RuntimeError("gateway down")
        with patch.object(vrp, "AI_VISION_ENABLED", True), \
             patch.object(vrp, "get_vision_ai_provider", return_value=provider):
            with pytest.raises(RuntimeError):
                vrp.parse_receipt_images(images_b64=[PNG_B64])

    def test_page_cap_is_enforced(self):
        """A 200-page PDF must not be rendered and billed page by page."""
        provider = _provider_returning(GOOD_RESPONSE)
        with patch.object(vrp, "AI_VISION_ENABLED", True), \
             patch.object(vrp, "AI_VISION_MAX_PAGES", 2), \
             patch.object(vrp, "get_vision_ai_provider", return_value=provider):
            vrp.parse_receipt_images(images_b64=[PNG_B64] * 50)

        sent = provider.complete_vision_json_sync.call_args.kwargs["images_b64"]
        assert len(sent) == 2

    def test_caption_is_passed_to_the_model(self):
        """The caption often names the operator the image itself does not show."""
        provider = _provider_returning(GOOD_RESPONSE)
        with patch.object(vrp, "AI_VISION_ENABLED", True), \
             patch.object(vrp, "get_vision_ai_provider", return_value=provider):
            vrp.parse_receipt_images(images_b64=[PNG_B64], caption="оплата за парковку")

        assert "оплата за парковку" in provider.complete_vision_json_sync.call_args.kwargs["prompt"]

    def test_fallback_datetime_used_when_receipt_has_no_date(self):
        no_date = dict(GOOD_RESPONSE, transaction_date_iso=None)
        received = datetime(2026, 3, 15, 9, 30, 0)
        with patch.object(vrp, "AI_VISION_ENABLED", True), \
             patch.object(vrp, "get_vision_ai_provider", return_value=_provider_returning(no_date)):
            result = vrp.parse_receipt_images(images_b64=[PNG_B64], fallback_datetime=received)

        assert result is not None
        assert result["date_source"] == "source_received_at"


class TestVisionProviderRequest:
    """
    The request itself. `_ensure_json_keyword` json.dumps-es any non-string
    content, so routing image messages through it would serialise the whole
    base64 payload into the prompt text — expensive and useless.
    """

    def _capture(self, provider):
        captured = {}

        def fake_create(**kwargs):
            captured.update(kwargs)
            message = MagicMock()
            message.content = '{"amount": 1, "confidence": 0.5}'
            response = MagicMock()
            response.choices = [MagicMock(message=message)]
            response.usage = None
            return response

        client = MagicMock()
        client.chat.completions.create = fake_create
        with patch.object(provider, "_get_sync_client", return_value=client):
            provider.complete_vision_json_sync(
                images_b64=[PNG_B64], prompt="read this receipt as json",
            )
        return captured

    def test_image_travels_as_a_data_uri(self):
        captured = self._capture(OpenAICompatibleTextProvider(api_key="k"))
        content = captured["messages"][-1]["content"]

        assert isinstance(content, list), "content must stay a list of parts"
        image_parts = [part for part in content if part.get("type") == "image_url"]
        assert len(image_parts) == 1
        assert image_parts[0]["image_url"]["url"] == f"data:image/png;base64,{PNG_B64}"

    def test_base64_never_leaks_into_the_prompt_text(self):
        captured = self._capture(OpenAICompatibleTextProvider(api_key="k"))
        text_parts = [
            part["text"] for part in captured["messages"][-1]["content"]
            if part.get("type") == "text"
        ]
        assert text_parts
        for text in text_parts:
            assert PNG_B64 not in text

    def test_json_keyword_is_guaranteed(self):
        """`response_format=json_object` requires the word in the prompt."""
        provider = OpenAICompatibleTextProvider(api_key="k")
        captured = {}

        def fake_create(**kwargs):
            captured.update(kwargs)
            message = MagicMock()
            message.content = '{"amount": 1}'
            response = MagicMock()
            response.choices = [MagicMock(message=message)]
            response.usage = None
            return response

        client = MagicMock()
        client.chat.completions.create = fake_create
        with patch.object(provider, "_get_sync_client", return_value=client):
            provider.complete_vision_json_sync(images_b64=[PNG_B64], prompt="read this")

        text = captured["messages"][-1]["content"][0]["text"]
        assert "json" in text.lower()

    def test_empty_image_list_is_rejected(self):
        provider = OpenAICompatibleTextProvider(api_key="k")
        with pytest.raises(ValueError):
            provider.complete_vision_json_sync(images_b64=[], prompt="x")
