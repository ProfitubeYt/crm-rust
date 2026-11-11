"""
The Claude Code session client.

The rule this file protects: the CLI is never load-bearing. It is an external
process on a small box — it can hang, hit a subscription limit, or change its
output shape on upgrade. Every call returns a result object rather than raising,
so callers can fall back to the existing provider.
"""
from unittest.mock import AsyncMock, patch

import pytest

from services import claude_cli_service as cc


def _run(coro):
    import asyncio

    return asyncio.get_event_loop_policy().new_event_loop().run_until_complete(coro)


class _Response:
    def __init__(self, payload):
        self._payload = payload

    def json(self):
        return self._payload


def _bridge(payload):
    """Patch httpx so the client sees `payload` come back from the bridge."""
    client = AsyncMock()
    client.post = AsyncMock(return_value=_Response(payload))
    ctx = AsyncMock()
    ctx.__aenter__ = AsyncMock(return_value=client)
    ctx.__aexit__ = AsyncMock(return_value=False)
    return patch("httpx.AsyncClient", return_value=ctx), client


class TestConfiguration:
    def test_unconfigured_bridge_is_reported_not_raised(self):
        with patch.object(cc, "BRIDGE_TOKEN", ""):
            result = _run(cc.ask("привет"))
        assert result.ok is False
        assert result.error == "bridge_not_configured"

    def test_empty_prompt_is_refused_before_any_call(self):
        with patch.object(cc, "BRIDGE_TOKEN", "t"):
            assert _run(cc.ask("   ")).error == "empty_prompt"

    def test_result_is_falsy_on_failure(self):
        assert not cc.ClaudeResult(ok=False)
        assert cc.ClaudeResult(ok=True, text="x")


class TestAsking:
    def test_a_successful_answer_carries_the_session(self):
        ctx, _ = _bridge({
            "ok": True, "text": "готово", "session_id": "abc-123",
            "cost_usd": 0.01, "duration_ms": 900,
        })
        with patch.object(cc, "BRIDGE_TOKEN", "t"), ctx:
            result = _run(cc.ask("сделай"))
        assert result.ok is True
        assert result.text == "готово"
        assert result.session_id == "abc-123"
        assert result.cost_usd == 0.01

    def test_session_id_is_forwarded_so_context_survives(self):
        ctx, client = _bridge({"ok": True, "text": "7"})
        with patch.object(cc, "BRIDGE_TOKEN", "t"), ctx:
            _run(cc.ask("какое число?", session_id="prev-session"))
        assert client.post.call_args.kwargs["json"]["session_id"] == "prev-session"

    def test_writes_are_off_unless_asked_for(self):
        ctx, client = _bridge({"ok": True, "text": "ok"})
        with patch.object(cc, "BRIDGE_TOKEN", "t"), ctx:
            _run(cc.ask("посмотри"))
        assert client.post.call_args.kwargs["json"]["allow_writes"] is False

    def test_writes_can_be_requested_explicitly(self):
        ctx, client = _bridge({"ok": True, "text": "ok"})
        with patch.object(cc, "BRIDGE_TOKEN", "t"), ctx:
            _run(cc.ask("поправь", allow_writes=True))
        assert client.post.call_args.kwargs["json"]["allow_writes"] is True

    def test_the_conversation_model_is_the_default(self):
        ctx, client = _bridge({"ok": True, "text": "ok"})
        with patch.object(cc, "BRIDGE_TOKEN", "t"), ctx:
            _run(cc.ask("вопрос"))
        assert client.post.call_args.kwargs["json"]["model"] == cc.MODEL_CONVERSATION

    def test_the_token_is_sent_as_a_header_not_in_the_body(self):
        ctx, client = _bridge({"ok": True, "text": "ok"})
        with patch.object(cc, "BRIDGE_TOKEN", "secret-token"), ctx:
            _run(cc.ask("вопрос"))
        assert client.post.call_args.kwargs["headers"]["X-Bridge-Token"] == "secret-token"
        assert "secret-token" not in str(client.post.call_args.kwargs["json"])


class TestFailureIsNeverFatal:
    """Whatever the CLI does, the caller gets a result and can fall back."""

    def test_an_unreachable_bridge_returns_retryable(self):
        ctx = patch("httpx.AsyncClient", side_effect=RuntimeError("connection refused"))
        with patch.object(cc, "BRIDGE_TOKEN", "t"), ctx:
            result = _run(cc.ask("вопрос"))
        assert result.ok is False
        assert result.retryable is True

    def test_a_bridge_error_is_passed_through(self):
        ctx, _ = _bridge({"ok": False, "error": "timeout", "retryable": True})
        with patch.object(cc, "BRIDGE_TOKEN", "t"), ctx:
            result = _run(cc.ask("вопрос"))
        assert result.ok is False
        assert result.error == "timeout"
        assert result.retryable is True

    def test_a_busy_bridge_is_retryable_rather_than_fatal(self):
        ctx, _ = _bridge({"ok": False, "error": "busy", "retryable": True})
        with patch.object(cc, "BRIDGE_TOKEN", "t"), ctx:
            assert _run(cc.ask("вопрос")).retryable is True

    def test_a_nonsense_response_does_not_crash(self):
        ctx, _ = _bridge(["not", "a", "dict"])
        with patch.object(cc, "BRIDGE_TOKEN", "t"), ctx:
            result = _run(cc.ask("вопрос"))
        assert result.ok is False

    def test_health_reports_rather_than_raising(self):
        ctx = patch("httpx.AsyncClient", side_effect=RuntimeError("down"))
        with patch.object(cc, "BRIDGE_TOKEN", "t"), ctx:
            assert _run(cc.health())["ok"] is False


class TestModelSplit:
    def test_conversation_and_worker_models_are_separate(self):
        """Opus for the conversation, Sonnet for the mechanical work under it —
        so the subscription is spent where the customer sees it."""
        assert "opus" in cc.MODEL_CONVERSATION
        assert "sonnet" in cc.MODEL_WORKER
