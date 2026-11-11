"""
The weekly audit: one publisher, one window, one message.

The customer was receiving the same seven-day figures nine times a week — a
built-in routine running `7 13 * * *`, plus a Sunday publish, plus a Monday
report that mirrored itself into the same channel. Nothing recorded that a
window had already been published, so manual triggers and restarts added more.
"""
from datetime import datetime
from unittest.mock import AsyncMock, patch

import pytest

from services.ai_agent import weekly_publisher


def _run(coro):
    import asyncio

    return asyncio.get_event_loop_policy().new_event_loop().run_until_complete(coro)


class TestSchedule:
    def test_only_the_sunday_publish_remains(self):
        """The Monday report computed the same window and reached the same
        channel through publish_team_notification."""
        import inspect

        from services.ai_agent import scheduler_service

        source = inspect.getsource(scheduler_service.AgentSchedulerService.start)
        assert 'id="agent-weekly-publish"' in source
        assert 'id="agent-weekly-report"' not in source

    def test_it_fires_sunday_at_twenty(self):
        import inspect

        from services.ai_agent import scheduler_service

        source = inspect.getsource(scheduler_service.AgentSchedulerService.start)
        assert 'day_of_week="sun", hour=20, minute=0' in source


class TestWindowClaim:
    def test_the_week_key_is_the_iso_week(self):
        key = weekly_publisher._week_key({})
        year, week, _ = datetime.utcnow().isocalendar()
        assert key == f"{year}-W{week:02d}"

    def test_an_explicit_period_gets_its_own_key(self):
        """Re-running a named period on purpose must stay possible."""
        assert weekly_publisher._week_key({"date_from": "2026-08-01T00:00:00"}) == "explicit:2026-08-01"

    def test_second_run_in_the_same_week_is_refused(self):
        config = weekly_publisher.WeeklyPublishConfig(bot_token="t", chat_id=-100)
        with patch.object(weekly_publisher, "_claim_week", AsyncMock(return_value=False)), \
             patch.object(weekly_publisher, "weekly_health_check", AsyncMock()) as health:
            result = _run(weekly_publisher.publish_weekly_report(config=config))
        assert result["published"] is False
        assert result["reason"] == "already_published_this_week"
        # Refused before doing the work, not after.
        health.assert_not_awaited()

    def test_force_overrides_the_claim(self):
        config = weekly_publisher.WeeklyPublishConfig(bot_token="t", chat_id=-100)
        with patch.object(weekly_publisher, "_claim_week", AsyncMock(return_value=False)), \
             patch.object(weekly_publisher, "weekly_health_check",
                          AsyncMock(return_value={"data": {"has_issues": False}})), \
             patch("httpx.AsyncClient") as client:
            client.return_value.__aenter__.return_value.post = AsyncMock(
                return_value=type("R", (), {"status_code": 200, "text": "ok"})()
            )
            result = _run(weekly_publisher.publish_weekly_report(
                config=config, arguments={"force": True},
            ))
        assert result["published"] is True

    def test_a_redis_outage_publishes_rather_than_going_silent(self):
        """A missed audit is worse than a duplicated one."""
        with patch("redis.asyncio.from_url", side_effect=RuntimeError("down")):
            assert _run(weekly_publisher._claim_week("2026-W01")) is True

    def test_a_failed_send_hands_the_week_back(self):
        config = weekly_publisher.WeeklyPublishConfig(bot_token="t", chat_id=-100)
        with patch.object(weekly_publisher, "_claim_week", AsyncMock(return_value=True)), \
             patch.object(weekly_publisher, "_release_week", AsyncMock()) as release, \
             patch.object(weekly_publisher, "weekly_health_check",
                          AsyncMock(return_value={"data": {"has_issues": False}})), \
             patch("httpx.AsyncClient") as client:
            client.return_value.__aenter__.return_value.post = AsyncMock(
                side_effect=RuntimeError("telegram down")
            )
            result = _run(weekly_publisher.publish_weekly_report(config=config))
        assert result["published"] is False
        release.assert_awaited_once()


class TestBuiltinRoutineRetired:
    def test_the_daily_routine_is_disabled_not_reseeded(self, db_session):
        from database.models import AgentRoutine
        from services.ai_agent import routine_service

        routine = routine_service.create_routine(
            db_session,
            name=routine_service._BUILTIN_RECEIPT_AUDIT_NAME,
            task_prompt="аудит",
            cron="7 13 * * *",
            kind="receipt_audit",
            deliver_to_channel=True,
            config={"builtin": routine_service._BUILTIN_RECEIPT_AUDIT_MARKER},
        )
        assert routine.enabled is True

        assert routine_service.ensure_builtin_routines(db_session) == 1
        db_session.refresh(routine)
        assert routine.enabled is False

        # Idempotent, and it never re-creates the routine.
        assert routine_service.ensure_builtin_routines(db_session) == 0
        assert db_session.query(AgentRoutine).filter(
            AgentRoutine.kind == "receipt_audit"
        ).count() == 1

    def test_a_renamed_builtin_is_still_recognised(self, db_session):
        """The old check matched on name, so renaming it in the UI made the next
        restart seed a second copy — both enabled, both posting."""
        from services.ai_agent import routine_service

        routine = routine_service.create_routine(
            db_session,
            name="Мой аудит",
            task_prompt="аудит",
            cron="7 13 * * *",
            kind="receipt_audit",
            config={"builtin": routine_service._BUILTIN_RECEIPT_AUDIT_MARKER},
        )
        assert routine_service.ensure_builtin_routines(db_session) == 1
        db_session.refresh(routine)
        assert routine.enabled is False

    def test_a_users_own_audit_routine_is_left_alone(self, db_session):
        from services.ai_agent import routine_service

        mine = routine_service.create_routine(
            db_session,
            name="Мой личный аудит",
            task_prompt="аудит",
            cron="0 9 * * 1",
            kind="receipt_audit",
            config={},
        )
        assert routine_service.ensure_builtin_routines(db_session) == 0
        db_session.refresh(mine)
        assert mine.enabled is True


class TestForumTopic:
    def test_routine_reports_go_to_the_receipt_topic(self):
        """The receipt logger and the weekly publisher both pass the topic; this
        one did not, so on a forum group it landed in General."""
        import inspect

        from services.ai_agent import routine_service

        source = inspect.getsource(routine_service._deliver_to_channel)
        assert "message_thread_id" in source
        assert "RECEIPT_LOG_TOPIC_ID" in source


class TestAutofixButton:
    def test_the_bot_routes_the_callback(self):
        """The button was published on every problem audit and did nothing: the
        publisher's docstring said the bot handled it, and no branch existed."""
        import inspect

        from services import auth_bot_handler

        source = inspect.getsource(auth_bot_handler._handle_callback)
        assert 'data.startswith("autofix:")' in source
        assert hasattr(auth_bot_handler, "_cb_weekly_autofix")
