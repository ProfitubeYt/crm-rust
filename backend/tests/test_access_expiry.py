"""
Account expiry: how long a person may keep using the desktop app.

The tricky part is what *not* to conflate. `locked_until` already existed and
looks like it would serve — it is the failed-login lockout, a security response
measured in minutes, not an entitlement measured in months. Overloading it would
have made a lockout look like an expired licence and an expiry look like a
break-in.
"""
from datetime import datetime, timedelta, timezone

import pytest

from api.dependencies import access_expired_at


class TestExpiryCheck:
    def test_no_date_means_unlimited(self):
        """Every account created before this feature has no date, and none of
        them should suddenly be locked out."""
        assert access_expired_at({"access_until": None}) is None
        assert access_expired_at({}) is None

    def test_a_future_date_is_not_expired(self):
        future = (datetime.now(timezone.utc) + timedelta(days=30)).isoformat()
        assert access_expired_at({"access_until": future}) is None

    def test_a_past_date_is_expired(self):
        past = (datetime.now(timezone.utc) - timedelta(days=1)).isoformat()
        assert access_expired_at({"access_until": past}) == past

    def test_a_naive_date_is_read_as_utc(self):
        past = (datetime.utcnow() - timedelta(days=1)).isoformat()
        assert access_expired_at({"access_until": past}) == past

    def test_an_unparseable_date_is_treated_as_unlimited(self):
        """A malformed date is our bug. Locking a paying customer out is the
        wrong way to discover it."""
        assert access_expired_at({"access_until": "не дата"}) is None
        assert access_expired_at({"access_until": 12345}) is None


class TestParsing:
    def test_a_bare_date_lasts_until_the_end_of_that_day(self):
        """"До 31 декабря" means they work through the 31st; starting at
        midnight would quietly cut a day off every licence."""
        from services.user_service import _parse_access_until

        parsed = _parse_access_until("2026-12-31")
        assert parsed.hour == 23 and parsed.minute == 59

    def test_an_explicit_time_is_respected(self):
        from services.user_service import _parse_access_until

        parsed = _parse_access_until("2026-12-31T09:00:00")
        assert parsed.hour == 9

    def test_empty_clears_the_date(self):
        """Distinct from "not mentioned in this patch": this is how an admin
        grants unlimited access again."""
        from services.user_service import _parse_access_until

        assert _parse_access_until("") is None
        assert _parse_access_until(None) is None

    def test_garbage_is_refused_rather_than_stored(self):
        from services.user_service import _parse_access_until

        with pytest.raises(ValueError):
            _parse_access_until("завтра")


class TestItIsNotTheLoginLockout:
    def test_locked_until_and_access_until_are_separate_columns(self):
        from database.models import User

        assert hasattr(User, "locked_until")
        assert hasattr(User, "access_until")
        assert User.locked_until is not User.access_until
