"""
The agent, reachable from Telegram.

Before this the traffic was one-way: the bot could post at you and there was no
path back. Grepping the bot for the orchestrator returned nothing at all.

These cover the parts worth protecting — who may talk to it, that a conversation
keeps its thread, and that a stale session is recovered rather than surfaced as
an error to the person typing.
"""
from unittest.mock import AsyncMock, MagicMock, patch

import pytest

from services import auth_bot_handler as bot


def _run(coro):
    import asyncio

    return asyncio.get_event_loop_policy().new_event_loop().run_until_complete(coro)


def _message(text: str, user_id: int = 555, admin: bool = True):
    message = MagicMock()
    message.text = text
    message.from_user = MagicMock(id=user_id, username="tester")
    message.answer = AsyncMock(return_value=MagicMock(edit_text=AsyncMock()))
    return message


class TestWhoMayTalkToIt:
    def test_a_stranger_is_ignored(self):
        message = _message("покажи транзакции", user_id=999)
        with patch.object(bot, "_is_admin", return_value=False):
            _run(bot.on_agent_message(message))
        message.answer.assert_not_awaited()

    def test_commands_fall_through_to_their_own_handlers(self):
        """The catch-all is registered last; it must not swallow /status."""
        message = _message("/status")
        with patch.object(bot, "_is_admin", return_value=True):
            _run(bot.on_agent_message(message))
        message.answer.assert_not_awaited()

    def test_empty_text_does_nothing(self):
        message = _message("   ")
        with patch.object(bot, "_is_admin", return_value=True):
            _run(bot.on_agent_message(message))
        message.answer.assert_not_awaited()


class TestAnswering:
    def test_an_answer_replaces_the_placeholder(self):
        """One message that updates, not a placeholder plus a reply."""
        from services import claude_cli_service

        placeholder = MagicMock(edit_text=AsyncMock())
        message = _message("сколько транзакций?")
        message.answer = AsyncMock(return_value=placeholder)

        result = claude_cli_service.ClaudeResult(ok=True, text="1477", session_id="s-1")
        with patch.object(bot, "_is_admin", return_value=True), \
             patch.object(claude_cli_service, "is_configured", return_value=True), \
             patch.object(claude_cli_service, "ask", AsyncMock(return_value=result)), \
             patch.object(bot, "_agent_session_id", AsyncMock(return_value=None)), \
             patch.object(bot, "_agent_model", AsyncMock(return_value=None)), \
             patch.object(bot, "_remember_agent_session", AsyncMock()) as remember:
            _run(bot.on_agent_message(message))

        message.answer.assert_awaited_once()
        placeholder.edit_text.assert_awaited_once()
        assert "1477" in placeholder.edit_text.await_args.args[0]
        remember.assert_awaited_once()

    def test_the_conversation_is_continued(self):
        from services import claude_cli_service

        message = _message("а за август?")
        result = claude_cli_service.ClaudeResult(ok=True, text="191", session_id="s-1")
        with patch.object(bot, "_is_admin", return_value=True), \
             patch.object(claude_cli_service, "is_configured", return_value=True), \
             patch.object(claude_cli_service, "ask", AsyncMock(return_value=result)) as ask, \
             patch.object(bot, "_agent_session_id", AsyncMock(return_value="s-1")), \
             patch.object(bot, "_agent_model", AsyncMock(return_value=None)), \
             patch.object(bot, "_remember_agent_session", AsyncMock()):
            _run(bot.on_agent_message(message))

        assert ask.await_args.kwargs["session_id"] == "s-1"
        # The system prompt belongs to the first turn; the session already has it.
        assert ask.await_args.kwargs["system_prompt"] is None

    def test_a_stale_session_is_retried_fresh(self):
        """After a restart the stored id no longer resolves. That is our problem
        to recover from, not something to show the person typing."""
        from services import claude_cli_service

        placeholder = MagicMock(edit_text=AsyncMock())
        message = _message("привет")
        message.answer = AsyncMock(return_value=placeholder)

        failed = claude_cli_service.ClaudeResult(ok=False, error="no such session")
        recovered = claude_cli_service.ClaudeResult(ok=True, text="здравствуйте", session_id="s-2")
        with patch.object(bot, "_is_admin", return_value=True), \
             patch.object(claude_cli_service, "is_configured", return_value=True), \
             patch.object(claude_cli_service, "ask",
                          AsyncMock(side_effect=[failed, recovered])) as ask, \
             patch.object(bot, "_agent_session_id", AsyncMock(return_value="dead-session")), \
             patch.object(bot, "_agent_model", AsyncMock(return_value=None)), \
             patch.object(bot, "_forget_agent_session", AsyncMock()) as forget, \
             patch.object(bot, "_remember_agent_session", AsyncMock()):
            _run(bot.on_agent_message(message))

        assert ask.await_count == 2
        forget.assert_awaited_once()
        assert "здравствуйте" in placeholder.edit_text.await_args.args[0]

    def test_a_real_failure_is_reported_not_swallowed(self):
        from services import claude_cli_service

        placeholder = MagicMock(edit_text=AsyncMock())
        message = _message("вопрос")
        message.answer = AsyncMock(return_value=placeholder)

        failed = claude_cli_service.ClaudeResult(ok=False, error="busy")
        with patch.object(bot, "_is_admin", return_value=True), \
             patch.object(claude_cli_service, "is_configured", return_value=True), \
             patch.object(claude_cli_service, "ask", AsyncMock(return_value=failed)), \
             patch.object(bot, "_agent_session_id", AsyncMock(return_value=None)), \
             patch.object(bot, "_agent_model", AsyncMock(return_value=None)):
            _run(bot.on_agent_message(message))

        assert "busy" in placeholder.edit_text.await_args.args[0]

    def test_an_unconfigured_bridge_says_so_plainly(self):
        from services import claude_cli_service

        message = _message("вопрос")
        with patch.object(bot, "_is_admin", return_value=True), \
             patch.object(claude_cli_service, "is_configured", return_value=False):
            _run(bot.on_agent_message(message))
        assert "недоступен" in message.answer.await_args.args[0]

    def test_a_long_answer_is_trimmed_to_telegram_limits(self):
        from services import claude_cli_service

        placeholder = MagicMock(edit_text=AsyncMock())
        message = _message("расскажи всё")
        message.answer = AsyncMock(return_value=placeholder)

        result = claude_cli_service.ClaudeResult(ok=True, text="а" * 9000, session_id="s")
        with patch.object(bot, "_is_admin", return_value=True), \
             patch.object(claude_cli_service, "is_configured", return_value=True), \
             patch.object(claude_cli_service, "ask", AsyncMock(return_value=result)), \
             patch.object(bot, "_agent_session_id", AsyncMock(return_value=None)), \
             patch.object(bot, "_agent_model", AsyncMock(return_value=None)), \
             patch.object(bot, "_remember_agent_session", AsyncMock()):
            _run(bot.on_agent_message(message))

        assert len(placeholder.edit_text.await_args.args[0]) < 4096


class TestModelSwitch:
    def test_switching_starts_a_new_conversation(self):
        """A session is bound to the model it began with; carrying it over would
        mix two models in one thread."""
        message = _message("/model sonnet")
        with patch.object(bot, "_is_admin", return_value=True), \
             patch.object(bot, "_set_agent_model", AsyncMock()) as set_model, \
             patch.object(bot, "_forget_agent_session", AsyncMock()) as forget:
            _run(bot.cmd_agent_model(message))

        assert set_model.await_args.args[1] == "cc/claude-sonnet-5"
        forget.assert_awaited_once()

    def test_a_full_model_name_is_accepted(self):
        message = _message("/model cc/claude-haiku-4-5")
        with patch.object(bot, "_is_admin", return_value=True), \
             patch.object(bot, "_set_agent_model", AsyncMock()) as set_model, \
             patch.object(bot, "_forget_agent_session", AsyncMock()):
            _run(bot.cmd_agent_model(message))
        assert set_model.await_args.args[1] == "cc/claude-haiku-4-5"

    def test_bare_command_shows_the_current_model(self):
        message = _message("/model")
        with patch.object(bot, "_is_admin", return_value=True), \
             patch.object(bot, "_agent_model", AsyncMock(return_value="cc/claude-opus-5")), \
             patch.object(bot, "_set_agent_model", AsyncMock()) as set_model:
            _run(bot.cmd_agent_model(message))
        set_model.assert_not_awaited()
        assert "opus" in message.answer.await_args.args[0]

    def test_reset_forgets_the_thread(self):
        message = _message("/reset")
        with patch.object(bot, "_is_admin", return_value=True), \
             patch.object(bot, "_forget_agent_session", AsyncMock()) as forget:
            _run(bot.cmd_agent_reset(message))
        forget.assert_awaited_once()
