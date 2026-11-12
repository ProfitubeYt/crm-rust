"""Fill the application of old receipts from the evidence in their operator line.

Dry run by default: prints what would change and writes nothing.

    python -m scripts.backfill_applications               # dry run, aggregate report
    python -m scripts.backfill_applications --apply       # write
    python -m scripts.backfill_applications --apply --set-p2p
    python -m scripts.backfill_applications --revalidate  # also re-derive earlier automatic values
    python -m scripts.backfill_applications --top 25      # list the operators left empty

The default report holds counts only and is safe to paste into a ticket. The
operator lines behind ``--top`` are not: some of them carry a person's name.

Run from the backend directory, or inside the backend container. The
connection comes from DATABASE_URL, the same as the API. The dry run reads a
few columns and needs no write permission.
"""
from __future__ import annotations

import argparse
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

from database.connection import SessionLocal  # noqa: E402
from services.application_backfill import backfill_applications, report_lines  # noqa: E402


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--apply", action="store_true", help="write the changes; without it nothing is written")
    parser.add_argument("--set-p2p", action="store_true", help="also set is_p2p from the dictionary")
    parser.add_argument("--revalidate", action="store_true",
                        help="also correct or clear earlier automatic values the evidence no longer supports")
    parser.add_argument("--limit", type=int, default=None, help="look at no more than N receipts without an application")
    parser.add_argument("--top", type=int, default=0,
                        help="list the N most frequent operators left empty (may contain names; off by default)")
    args = parser.parse_args(argv)

    with SessionLocal() as db:
        report = backfill_applications(
            db, apply=args.apply, set_p2p=args.set_p2p, limit=args.limit, revalidate=args.revalidate,
        )
        if args.apply and (report.matched or report.revalidated):
            # Imported late: a dry run through a read-only role needs none of it.
            from services.access_control_service import write_audit_log

            write_audit_log(
                db,
                action="application_backfill",
                success=True,
                details={
                    "matched": report.matched,
                    "revalidated": report.revalidated,
                    "p2p_changed": report.p2p_changed,
                    "by_application": dict(report.by_application),
                    "by_reason": dict(report.by_reason),
                    "first_id": min(report.ids) if report.ids else None,
                    "last_id": max(report.ids) if report.ids else None,
                },
            )

    print("\n".join(report_lines(report, top=args.top, set_p2p=args.set_p2p)))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
