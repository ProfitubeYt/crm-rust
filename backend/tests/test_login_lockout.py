"""Sign-in lockout after wrong passwords, with time stamps as PostgreSQL returns them.

`users.locked_until` is TIMESTAMP WITH TIME ZONE on the server, so the value
read back carries a time zone; SQLite, which this suite runs on, hands back a
naive one. The end-to-end run on PostgreSQL (scripts/hub/e2e) found that the
next sign-in of a locked account answered 500, lock run out or not.
"""
from __future__ import annotations

from datetime import datetime, timedelta, timezone

import pytest

from hub_support import PASSWORD, app_client, seed


@pytest.fixture
def client(db_session, monkeypatch, tmp_path):
    with app_client(db_session, monkeypatch, tmp_path) as test_client:
        yield test_client


def _sign_in(client, password: str = PASSWORD):
    return client.post("/api/auth/login", json={"username": "locked", "password": password})


def test_a_locked_account_is_told_so_when_the_lock_carries_a_time_zone(client, db_session):
    user = seed(db_session, "locked")
    # Set on the loaded row and not committed: the value stays as given, time
    # zone included, which is how PostgreSQL returns the column.
    user.locked_until = datetime.now(timezone.utc) + timedelta(minutes=10)

    refused = _sign_in(client)

    assert refused.status_code == 423, refused.text
    body = refused.json()
    assert body["error"] == "locked" and 0 < body["locked_seconds"] <= 600


def test_a_lock_that_has_run_out_lets_the_account_in(client, db_session):
    user = seed(db_session, "locked")
    user.failed_attempts = 5
    user.locked_until = datetime.now(timezone.utc) - timedelta(seconds=1)

    signed_in = _sign_in(client)

    assert signed_in.status_code == 200 and signed_in.json()["ok"] is True, signed_in.text
    assert user.locked_until is None and user.failed_attempts == 0


def test_five_wrong_passwords_lock_the_account_and_the_right_one_does_not_open_it(client, db_session):
    seed(db_session, "locked")

    answers = [_sign_in(client, "wrong-password") for _ in range(5)]

    assert [answer.status_code for answer in answers] == [401, 401, 401, 401, 423]
    assert [answer.json()["attempts_left"] for answer in answers] == [4, 3, 2, 1, 0]
    assert _sign_in(client).status_code == 423
