from datetime import datetime, timedelta, timezone

from services.time_utils import naive_utc, utc_timestamp


def test_database_timestamps_compare_in_utc():
    local = datetime(2026, 11, 15, 15, tzinfo=timezone(timedelta(hours=5)))
    assert naive_utc(local) == datetime(2026, 11, 15, 10)
    assert utc_timestamp(local) == utc_timestamp(datetime(2026, 11, 15, 10))
    assert utc_timestamp(datetime(1970, 1, 1)) == 0
