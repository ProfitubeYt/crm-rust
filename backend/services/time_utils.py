"""UTC normalization at database boundaries (PostgreSQL and SQLite differ)."""
from datetime import datetime, timezone


def naive_utc(moment: datetime) -> datetime:
    """Interpret a naive timestamp as UTC and convert aware timestamps to UTC."""
    if moment.tzinfo is None:
        return moment
    return moment.astimezone(timezone.utc).replace(tzinfo=None)


def utc_timestamp(moment: datetime) -> int:
    """Unix seconds independent of the host's local time zone."""
    return int(naive_utc(moment).replace(tzinfo=timezone.utc).timestamp())
