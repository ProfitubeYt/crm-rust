"""Build the change manifest for the dictionary import and the historical fills. Reads only.

    python -m scripts.build_change_manifest --out /protected/dir/manifest.json

The manifest is the whole reviewed plan in one file, receipt by receipt:

* ``dictionary.add``        the verified lines that would be added (and why the others are not);
* ``applications.changes``  every «Приложение» the historical pass would fill once those lines
                            are in the dictionary, with the value it replaces and the evidence;
* ``details.fills``         every «Подробности» the confirmed patterns would fill, and, apart
                            from them, ``details.suggestions`` that are reported and never applied.

Nothing is written to the database: the dictionary lines are only previewed
(`OperatorMapper.preview_rows`), so a read-only role is enough.

The file names receipts and notes. It is written with mode 600, never inside
the repository, and what this script prints is counts and a digest only. The
digest covers the planned changes and nothing else, so two runs over the same
data give the same digest whoever runs them.

Applying it is a separate step with its own journal: ``scripts/apply_change_manifest.py``.

Run from the backend directory, or inside the backend container. The
connection comes from DATABASE_URL.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
import sys
import uuid
from collections import Counter
from datetime import datetime, timezone
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

from sqlalchemy import func, or_  # noqa: E402

from database.connection import SessionLocal  # noqa: E402
from database.models import OperatorReference, Transaction  # noqa: E402
from services import annotation_rule_service  # noqa: E402
from services import operator_dictionary_service as dictionary  # noqa: E402
from services.application_backfill import backfill_applications  # noqa: E402

FORMAT = "tbs-change-manifest/1"


def content_digest(manifest: dict) -> str:
    """SHA-256 of the planned changes alone, in a canonical form."""
    planned = {
        "dictionary": manifest["dictionary"]["add"],
        "applications": manifest["applications"]["changes"],
        "details": manifest["details"]["fills"],
    }
    canonical = json.dumps(planned, ensure_ascii=False, sort_keys=True, separators=(",", ":"), default=str)
    return hashlib.sha256(canonical.encode("utf-8")).hexdigest()


def build_manifest(db, research: dict, *, batch_id: str) -> dict:
    lines = dictionary.verified_lines(research)
    planned = dictionary.plan_import(db, lines)
    to_add = [item.line for item in planned if item.adds]

    report = backfill_applications(db, apply=False, detector=dictionary.preview_detector(db, to_add))
    details = annotation_rule_service.details_plan(db)
    details_counts = annotation_rule_service.details_report(db)

    empty = or_(Transaction.application_mapped.is_(None), func.trim(Transaction.application_mapped) == "")
    no_note = or_(Transaction.annotation_details.is_(None), func.trim(Transaction.annotation_details) == "")
    manifest = {
        "format": FORMAT,
        "batch_id": batch_id,
        "created_at": datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "research_file": research.get("title", ""),
        "researched_on": research.get("researched_on"),
        "source": {
            "transactions": db.query(func.count(Transaction.id)).scalar(),
            "max_transaction_id": db.query(func.max(Transaction.id)).scalar(),
            "without_application": db.query(func.count(Transaction.id)).filter(empty).scalar(),
            "without_details": db.query(func.count(Transaction.id)).filter(no_note).scalar(),
            "dictionary_lines": db.query(func.count(OperatorReference.id)).scalar(),
            "dictionary_active": db.query(func.count(OperatorReference.id)).filter(OperatorReference.is_active.is_(True)).scalar(),
        },
        "dictionary": {
            "add": [
                {"operator_name": line.operator, "application_name": line.application, "is_p2p": line.is_p2p, "entry": line.entry_id}
                for line in to_add
            ],
            "not_added": [
                {"operator_name": item.line.operator, "application_name": item.line.application, "action": item.action, "detail": item.detail}
                for item in planned if not item.adds
            ],
        },
        "applications": {
            "changes": report.changes,
            "summary": {
                "receipts": report.total,
                "already_set": report.already_set,
                "manual": report.manual,
                "without_application": report.candidates,
                "would_fill": report.matched,
                "locked": report.locked,
                "left_empty": report.unmatched,
                "by_reason": dict(report.by_reason),
                "by_application": dict(report.by_application),
                "left_empty_by_reason": dict(report.unresolved),
                "ambiguous": dict(report.ambiguous),
                "stored_differs_from_evidence": {f"{stored} -> {found}": count for (stored, found), count in report.disagreements.items()},
            },
        },
        "details": {
            "fills": details["fills"],
            "suggestions": details["suggestions"],
            "locked": details["locked"],
            "summary": {
                "confirmed": details_counts["confirmed"],
                "confirmed_patterns": details_counts["keys"],
                "rules": details_counts["rules"],
                "without_details": details_counts["empty"],
                "would_fill": len(details["fills"]),
                "locked": len(details["locked"]),
                "suggestions": dict(Counter(item["reason"] for item in details["suggestions"])),
                "left_empty": dict(details_counts["unresolved"]),
                "rule_written": details_counts["rule_written"],
                "rule_would_retract": details_counts["rule_would_retract"],
                "agent_written": details_counts["agent_written"],
            },
        },
    }
    manifest["content_sha256"] = content_digest(manifest)
    return manifest


def summary_lines(manifest: dict) -> list:
    """Counts and the digest: nothing that names a receipt or a note."""
    apps = manifest["applications"]["summary"]
    notes = manifest["details"]["summary"]
    lines = [
        "CHANGE MANIFEST, nothing written to the database",
        f"batch:                          {manifest['batch_id']}",
        f"receipts:                       {manifest['source']['transactions']} (highest id {manifest['source']['max_transaction_id']})",
        f"dictionary lines to add:        {len(manifest['dictionary']['add'])} (not added: {len(manifest['dictionary']['not_added'])})",
        f"«Приложение» empty:             {apps['without_application']}",
        f"  would be filled:              {apps['would_fill']}",
        f"  skipped, locked period:       {apps['locked']}",
        f"  left empty:                   {apps['left_empty']}",
    ]
    lines.extend(f"    {count:6d}  {reason}" for reason, count in sorted(apps["by_reason"].items(), key=lambda item: -item[1]))
    lines.append("  filled, by application:")
    lines.extend(f"    {count:6d}  {name}" for name, count in sorted(apps["by_application"].items(), key=lambda item: -item[1]))
    lines.append("  left empty, by reason:")
    lines.extend(f"    {count:6d}  {reason}" for reason, count in sorted(apps["left_empty_by_reason"].items(), key=lambda item: -item[1]))
    if apps["stored_differs_from_evidence"]:
        lines.append("  stored value differs from the evidence (not changed, for review):")
        lines.extend(f"    {count:6d}  {pair}" for pair, count in sorted(apps["stored_differs_from_evidence"].items(), key=lambda item: -item[1]))
    lines += [
        f"«Подробности» empty:            {notes['without_details']}",
        f"  would be filled:              {notes['would_fill']}",
        f"  skipped, locked period:       {notes['locked']}",
        f"  suggestion only, not applied: {sum(notes['suggestions'].values())} {notes['suggestions']}",
        f"  left empty:                   {sum(notes['left_empty'].values())} {notes['left_empty']}",
        f"content digest (sha256):        {manifest['content_sha256']}",
    ]
    return lines


def _inside_repository(path: Path) -> bool:
    repository = ROOT.parent
    # Source archives and deployed containers do not carry Git metadata.
    # Protect the whole checkout when recognizable, otherwise the backend package.
    if not (repository / ".git").exists() and not (repository / "desktop" / "Cargo.toml").is_file():
        repository = ROOT
    try:
        path.resolve().relative_to(repository.resolve())
    except ValueError:
        return False
    return True


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--out", type=Path, required=True, help="where to write the manifest (outside the repository)")
    parser.add_argument("--research", type=Path, default=None, help="research file; the newest one by default")
    parser.add_argument("--batch-id", default=None, help="name of the batch; generated when omitted")
    args = parser.parse_args(argv)

    if _inside_repository(args.out):
        parser.error("the manifest names receipts and notes: write it outside the repository")
    batch_id = args.batch_id or f"dict-details-{datetime.now(timezone.utc):%Y%m%d}-{uuid.uuid4().hex[:8]}"

    with SessionLocal() as db:
        manifest = build_manifest(db, dictionary.load_research(args.research), batch_id=batch_id)
        db.rollback()

    args.out.parent.mkdir(parents=True, exist_ok=True)
    descriptor = os.open(args.out, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
    with os.fdopen(descriptor, "w", encoding="utf-8") as handle:
        json.dump(manifest, handle, ensure_ascii=False, indent=1, default=str)
    os.chmod(args.out, 0o600)

    print("\n".join(summary_lines(manifest)))
    print(f"written (mode 600):             {args.out}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
