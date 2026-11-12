"""Review the shared application dictionary. Reads only; nothing is ever written.

    python -m scripts.operator_dictionary plan            # what adding the verified lines would do
    python -m scripts.operator_dictionary review          # operators that still have no application
    python -m scripts.operator_dictionary review --top 40
    python -m scripts.operator_dictionary review --names  # also list merchant lines

``plan`` reads the research file (``data/operator_dictionary/research_*.json``)
and says, line by line, whether a verified line would be added, is already
answered, or contradicts something the dictionary or a person says (such a line
is left out, never overwritten). Dictionary lines are public gateway names.

``review`` counts the operator lines of receipts without an application by
review status and lists the gateway-shaped ones with their candidate. Merchant
lines carry addresses and sometimes a person's name: they are counted, and
printed only with ``--names``, which is not for a ticket or a chat.

Adding lines is a separate, journalled step: ``scripts.build_change_manifest``
and ``scripts/apply_change_manifest.py``, or the reference screen by hand.

Run from the backend directory, or inside the backend container. The
connection comes from DATABASE_URL; a read-only role is enough.
"""
from __future__ import annotations

import argparse
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

from database.connection import SessionLocal  # noqa: E402
from services import operator_dictionary_service as dictionary  # noqa: E402


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("command", choices=("plan", "review"))
    parser.add_argument("--research", type=Path, default=None, help="research file; the newest one by default")
    parser.add_argument("--top", type=int, default=0, help="review: list no more than N operator lines")
    parser.add_argument("--names", action="store_true",
                        help="review: also print merchant lines (addresses, sometimes a person's name)")
    args = parser.parse_args(argv)

    research = dictionary.load_research(args.research)
    with SessionLocal() as db:
        if args.command == "plan":
            planned = dictionary.plan_import(db, dictionary.verified_lines(research))
            print("\n".join(dictionary.plan_lines(planned)))
        else:
            items = dictionary.review_operators(db, research)
            print("\n".join(dictionary.review_lines(items, names=args.names, top=args.top)))
        db.rollback()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
