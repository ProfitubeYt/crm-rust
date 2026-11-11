"""Regression: GET /api/tg/chats used to 500 on mixed-awareness datetimes.

`TgChatMessage.message_date` is stored naive while the `MonitoredBotChat`
timestamps are timezone-aware. When a chat's latest message came from TDLib
(aware) and its processed date from the database (naive), the backlog
calculation raised "can't subtract offset-naive and offset-aware datetimes".
That took out the entire chat list, so the desktop client showed a connected
status and an empty list with no error the user could act on.
"""
from __future__ import annotations

from datetime import datetime, timedelta, timezone

import pytest

from api.routes.telegram_client import _as_utc, _build_monitor_backlog_payload


class _Monitor:
    """Stand-in for a MonitoredBotChat row."""

    def __init__(self, *, updated_at=None, last_realtime_seen_at=None,
                 processed_id=10, last_error=None):
        self.last_processed_message_id = processed_id
        self.last_realtime_seen_at = last_realtime_seen_at
        self.updated_at = updated_at
        self.last_error = last_error
        self.enabled = True


AWARE = datetime(2026, 9, 1, 12, 0, tzinfo=timezone.utc)
NAIVE = datetime(2026, 9, 1, 6, 0)


@pytest.mark.parametrize(
    "value,expected_tz",
    [(AWARE, timezone.utc), (NAIVE, timezone.utc), (None, None), ("nonsense", None)],
)
def test_as_utc_normalizes(value, expected_tz):
    result = _as_utc(value)
    if expected_tz is None:
        assert result is None
    else:
        assert result.tzinfo is not None
        assert result.utcoffset() == timedelta(0)


def test_naive_is_read_as_utc_not_shifted():
    """A naive value must keep its clock reading, only gaining a tzinfo."""
    assert _as_utc(NAIVE) == datetime(2026, 9, 1, 6, 0, tzinfo=timezone.utc)


def test_mixed_awareness_does_not_raise():
    """The exact combination seen in production."""
    payload = _build_monitor_backlog_payload(
        _Monitor(updated_at=AWARE, last_realtime_seen_at=NAIVE),
        latest_message_id=99,
        latest_message_date=AWARE,          # from TDLib: aware
        last_processed_message_date=NAIVE,  # from the database: naive
    )
    assert payload["backlog_lag_messages"] == 89
    assert payload["backlog_state"] in {"lagging", "stalled", "error"}


def test_all_naive_still_works():
    payload = _build_monitor_backlog_payload(
        _Monitor(updated_at=NAIVE, last_realtime_seen_at=NAIVE),
        latest_message_id=12,
        latest_message_date=NAIVE,
        last_processed_message_date=NAIVE,
    )
    assert payload["backlog_state"] == "lagging"


def test_all_aware_still_works():
    payload = _build_monitor_backlog_payload(
        _Monitor(updated_at=AWARE, last_realtime_seen_at=AWARE),
        latest_message_id=12,
        latest_message_date=AWARE,
        last_processed_message_date=AWARE,
    )
    assert payload["backlog_state"] == "lagging"


def test_stalled_is_still_detected_across_awareness():
    """The comparison must keep working, not just stop throwing."""
    old_processed = datetime(2026, 9, 1, 6, 0)              # naive, 6h behind
    fresh_latest = datetime(2026, 9, 1, 12, 0, tzinfo=timezone.utc)
    payload = _build_monitor_backlog_payload(
        _Monitor(updated_at=fresh_latest, last_realtime_seen_at=fresh_latest),
        latest_message_id=99,
        latest_message_date=fresh_latest,
        last_processed_message_date=old_processed,
    )
    # 6 hours of unprocessed backlog is well past the one-hour threshold.
    assert payload["backlog_state"] == "stalled"


def test_no_monitor_returns_empty_payload():
    payload = _build_monitor_backlog_payload(None, latest_message_id=5)
    assert payload["backlog_state"] is None
    assert payload["last_processed_message_id"] is None
