"""Fill the application of old receipts from the operator dictionary.

Dry run by default: prints what would change and writes nothing.

    python -m scripts.backfill_applications             # dry run
    python -m scripts.backfill_applications --apply     # write
    python -m scripts.backfill_applications --apply --set-p2p

Run from the backend directory, or inside the backend container. The
connection comes from DATABASE_URL, the same as the API.
"""
from __future__ import annotations

import argparse
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

from database.connection import SessionLocal  # noqa: E402
from services.access_control_service import write_audit_log  # noqa: E402
from services.application_backfill import backfill_applications  # noqa: E402


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--apply", action="store_true", help="write the changes; without it nothing is written")
    parser.add_argument("--set-p2p", action="store_true", help="also set is_p2p from the dictionary")
    parser.add_argument("--limit", type=int, default=None, help="look at no more than N receipts")
    parser.add_argument("--top", type=int, default=25, help="how many unmatched operators to list")
    args = parser.parse_args(argv)

    with SessionLocal() as db:
        report = backfill_applications(db, apply=args.apply, set_p2p=args.set_p2p, limit=args.limit)
        if args.apply and report.matched:
            write_audit_log(
                db,
                action="application_backfill",
                success=True,
                details={
                    "matched": report.matched,
                    "p2p_changed": report.p2p_changed,
                    "by_application": dict(report.by_application),
                    "first_id": min(report.ids),
                    "last_id": max(report.ids),
                },
            )

    mode = "APPLIED" if args.apply else "DRY RUN, nothing written"
    print(f"{mode}")
    print(f"receipts without application: {report.candidates}")
    print(f"filled from dictionary:        {report.matched}")
    print(f"skipped, locked period:        {report.locked}")
    print(f"no dictionary match:           {report.unmatched}")
    if args.set_p2p:
        print(f"is_p2p changed:                {report.p2p_changed}")
    if report.by_application:
        print("\nby application:")
        for app, count in report.by_application.most_common():
            print(f"  {count:6d}  {app}")
    if report.unmatched_operators:
        print(f"\ntop unmatched operators:")
        for op, count in report.unmatched_operators.most_common(args.top):
            print(f"  {count:6d}  {op}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
