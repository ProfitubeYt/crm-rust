"""Fill «Подробности» of old receipts from confirmed similar operations.

Dry run by default: prints what would change, in counts, and writes nothing.

    python -m scripts.enrich_details                 # dry run, aggregate report
    python -m scripts.enrich_details --apply         # recompute the note rules and fill
    python -m scripts.enrich_details --apply --revalidate
                                                     # also correct or take back notes an
                                                     # earlier rule wrote and the confirmed
                                                     # receipts no longer support

A note is learned only from what a person wrote or the workbook supplied, per
operator, own card and direction, and only while the most recent confirmed
receipts agree on it (see services/annotation_rule_service.py). It is copied
whole or not at all. A note a person or the workbook wrote is never replaced,
and receipts inside a locked period are left alone. A second run changes
nothing.

The report holds counts and reasons only: no note, operator or card number.

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
from services import annotation_rule_service  # noqa: E402


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--apply", action="store_true", help="write the changes; without it nothing is written")
    parser.add_argument("--revalidate", action="store_true",
                        help="with --apply: also correct or take back notes an earlier rule wrote")
    args = parser.parse_args(argv)

    with SessionLocal() as db:
        report = annotation_rule_service.details_report(db)
        print("APPLYING" if args.apply else "DRY RUN, nothing written")
        print("\n".join(annotation_rule_service.details_report_lines(report)))
        if not args.apply:
            db.rollback()
            return 0

        rules = annotation_rule_service.recompute(db, fields=("details",))
        applied = annotation_rule_service.apply_rules(
            db, fields=("details",), limit=10_000_000, revalidate=args.revalidate,
        )
        # Imported late: a dry run through a read-only role needs none of it.
        from services.access_control_service import write_audit_log

        write_audit_log(
            db,
            action="details_enrichment",
            success=True,
            details={
                "rules": {key: rules[key] for key in ("created", "updated", "disabled", "pinned_kept")},
                "updated": applied["updated"],
                "retracted": applied["retracted"],
                "skipped_locked": applied["skipped_locked"],
                "revalidate": bool(args.revalidate),
            },
        )
        print(
            f"\nAPPLIED: rules created {rules['created']}, updated {rules['updated']}, disabled {rules['disabled']};"
            f" receipts changed {applied['updated']}, notes taken back {applied['retracted']},"
            f" skipped in locked periods {applied['skipped_locked']}"
        )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
