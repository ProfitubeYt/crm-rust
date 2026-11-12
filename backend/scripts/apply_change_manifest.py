#!/usr/bin/env python3
"""Apply a reviewed change manifest exactly, with a journal, or take one batch back.

A single file with no import from the application: it needs Python, SQLAlchemy and the
database driver, so it runs against whatever backend version is live, from the backend
container or any host that reaches the database. It deploys nothing and restarts nothing.

    python apply_change_manifest.py verify   --manifest M.json
    python apply_change_manifest.py apply    --manifest M.json --journal J.json --i-have-a-verified-backup
    python apply_change_manifest.py rollback --journal J.json
    (add  --only dictionary,applications,details  to any of them to take a part)

The manifest comes from ``python -m scripts.build_change_manifest`` and lists every
change with the value it replaces. This tool adds nothing of its own:

* ``verify`` reads only. It compares every planned change with the database and counts
  what is ready, already done, or no longer as the manifest saw it.
* ``apply`` writes each change only where the receipt still holds the value the manifest
  saw, inside one transaction, and skips receipts in a locked period. A receipt somebody
  edited meanwhile is left alone and reported. Suggestions in the manifest are never
  applied. The journal (mode 600) keeps the batch id and, per record, what was there
  before and what was written. A second run changes nothing.
* ``rollback`` undoes the records of that journal and nothing else, and only where the
  value the batch wrote is still there: a later manual edit and every newer receipt stay.
  It never restores a database dump. A note it takes back would be written again within
  minutes by the background pass if its rule stayed on, so the computed rules behind the
  notes it takes back are switched off as well (never a rule a person pinned); the next
  relearning switches them on again. A worker keeps its rules for up to two minutes: run
  ``rollback`` once more after that, it undoes whatever was written in between.

What it writes, and nothing more:
  operator_reference   new active lines (dictionary.add)
  transactions         application_mapped / application_src / application_confidence
                       annotation_details / annotation_details_src / annotation_updated_*
                       updated_at, so every synchronised copy picks the change up
  transaction_annotation_events   one row per note written or taken back, as the
                       application itself journals a note
  annotation_rules     rollback only: is_enabled = false for the computed note rules of
                       the notes it takes back

The connection comes from DATABASE_URL. ``verify`` works through a read-only role;
``apply`` and ``rollback`` need the application's own role.
"""
from __future__ import annotations

import argparse
import json
import os
import sys
from datetime import date, datetime, timezone
from pathlib import Path
from typing import Any, Dict, List, Optional, Sequence, Tuple

from sqlalchemy import create_engine, text

FORMAT = "tbs-change-manifest/1"
JOURNAL_FORMAT = "tbs-change-journal/1"
PARTS = ("dictionary", "applications", "details")


def _blank(value: Optional[str]) -> bool:
    return value is None or not str(value).strip()


def _same(left: Optional[str], right: Optional[str]) -> bool:
    """Two stored texts are one value: an empty cell is an empty cell however it is spelt."""
    if _blank(left) and _blank(right):
        return True
    return (left or "") == (right or "")


def _day(value: Any) -> Optional[date]:
    if value is None:
        return None
    if isinstance(value, datetime):
        return value.date()
    if isinstance(value, date):
        return value
    try:
        return datetime.fromisoformat(str(value)[:19]).date()
    except ValueError:
        return None


def _write_private(path: Path, payload: Dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_suffix(path.suffix + ".tmp")
    descriptor = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
    with os.fdopen(descriptor, "w", encoding="utf-8") as handle:
        json.dump(payload, handle, ensure_ascii=False, indent=1, default=str)
        handle.flush()
        os.fsync(handle.fileno())
    os.replace(temporary, path)
    os.chmod(path, 0o600)


def _load(path: Path, expected: str) -> Dict[str, Any]:
    with open(path, encoding="utf-8") as handle:
        payload = json.load(handle)
    if payload.get("format") != expected:
        raise SystemExit(f"{path}: not a {expected} file")
    return payload


class Store:
    """The few statements this tool runs, in one place."""

    def __init__(self, connection):
        self.connection = connection

    def locks(self) -> List[Tuple[date, date]]:
        rows = self.connection.execute(text(
            "SELECT date_from, date_to FROM locked_periods WHERE is_active = :active"
        ), {"active": True}).fetchall()
        return [(_day(start), _day(end)) for start, end in rows if _day(start) and _day(end)]

    def receipts(self, ids: Sequence[int]) -> Dict[int, Dict[str, Any]]:
        found: Dict[int, Dict[str, Any]] = {}
        ids = list(ids)
        for start in range(0, len(ids), 500):
            page = ids[start:start + 500]
            marks = ", ".join(f":id{index}" for index in range(len(page)))
            rows = self.connection.execute(text(
                "SELECT id, application_mapped, application_src, annotation_details, annotation_details_src, transaction_date "
                f"FROM transactions WHERE id IN ({marks})"
            ), {f"id{index}": value for index, value in enumerate(page)}).mappings().all()
            for row in rows:
                found[int(row["id"])] = dict(row)
        return found

    def dictionary_line(self, operator: str, application: str) -> Optional[Dict[str, Any]]:
        row = self.connection.execute(text(
            "SELECT id, is_active FROM operator_reference WHERE operator_name = :operator AND application_name = :application "
            "ORDER BY id LIMIT 1"
        ), {"operator": operator, "application": application}).mappings().first()
        return dict(row) if row else None


def _locked(when: Any, locks: Sequence[Tuple[date, date]]) -> bool:
    day = _day(when)
    return bool(day and any(start <= day <= end for start, end in locks))


def _classify(manifest: Dict[str, Any], store: Store, parts: Sequence[str]) -> Dict[str, Dict[str, List[Dict[str, Any]]]]:
    """Every planned change against the database: ready, done, changed, locked or missing."""
    result: Dict[str, Dict[str, List[Dict[str, Any]]]] = {part: {} for part in parts}
    locks = store.locks()

    if "dictionary" in parts:
        buckets = result["dictionary"]
        for line in manifest["dictionary"]["add"]:
            existing = store.dictionary_line(line["operator_name"], line["application_name"])
            if existing is None:
                state = "ready"
            elif existing["is_active"]:
                state = "done"
            else:
                state = "changed"  # the same line stands there inactive: a person's call, not ours
            buckets.setdefault(state, []).append(line)

    for part, field, source_field in (("applications", "application_mapped", "application_src"), ("details", "annotation_details", "annotation_details_src")):
        if part not in parts:
            continue
        changes = manifest[part]["changes" if part == "applications" else "fills"]
        current = store.receipts([int(change["id"]) for change in changes])
        key = "application" if part == "applications" else "details"
        buckets = result[part]
        for change in changes:
            row = current.get(int(change["id"]))
            if row is None:
                state = "missing"
            elif _same(row[field], change["after"][key]) and (row[source_field] or None) == change["after"]["src"]:
                state = "done"
            elif not _same(row[field], change["before"][key]) or (row[source_field] or None) != (change["before"]["src"] or None):
                state = "changed"
            elif _locked(row["transaction_date"], locks):
                state = "locked"
            else:
                state = "ready"
            buckets.setdefault(state, []).append(change)
    return result


def _counts(classified: Dict[str, Dict[str, List[Any]]]) -> Dict[str, Dict[str, int]]:
    return {part: {state: len(items) for state, items in sorted(buckets.items())} for part, buckets in classified.items()}


def _print_counts(title: str, counts: Dict[str, Dict[str, int]]) -> None:
    print(title)
    for part, states in counts.items():
        described = ", ".join(f"{state} {count}" for state, count in states.items()) or "nothing planned"
        print(f"  {part:13s} {described}")


def verify(engine, manifest: Dict[str, Any], parts: Sequence[str]) -> Dict[str, Dict[str, int]]:
    with engine.connect() as connection:
        counts = _counts(_classify(manifest, Store(connection), parts))
        connection.rollback()
    return counts


def apply(engine, manifest: Dict[str, Any], parts: Sequence[str], journal_path: Path) -> Dict[str, Any]:
    now = datetime.now(timezone.utc).replace(tzinfo=None)
    journal: Dict[str, Any] = {
        "format": JOURNAL_FORMAT,
        "batch_id": manifest["batch_id"],
        "manifest_sha256": manifest.get("content_sha256"),
        "applied_at": now.strftime("%Y-%m-%dT%H:%M:%SZ"),
        "status": "pending",
        "parts": list(parts),
        "dictionary": [],
        "applications": [],
        "details": [],
        "skipped": {},
    }
    with engine.begin() as connection:
        store = Store(connection)
        classified = _classify(manifest, store, parts)
        journal["skipped"] = {
            part: {state: [item.get("id", item.get("operator_name")) for item in items] for state, items in buckets.items() if state != "ready"}
            for part, buckets in classified.items()
        }

        for line in classified.get("dictionary", {}).get("ready", []):
            new_id = connection.execute(text(
                "INSERT INTO operator_reference (operator_name, application_name, is_p2p, is_active) "
                "VALUES (:operator, :application, :p2p, :active) RETURNING id"
            ), {"operator": line["operator_name"], "application": line["application_name"], "p2p": bool(line["is_p2p"]), "active": True}).scalar()
            journal["dictionary"].append({"id": int(new_id), **line})

        for change in classified.get("applications", {}).get("ready", []):
            done = connection.execute(text(
                "UPDATE transactions SET application_mapped = :value, application_src = :src, application_confidence = :confidence, "
                "updated_at = CURRENT_TIMESTAMP WHERE id = :id "
                "AND COALESCE(TRIM(application_mapped), '') = :before AND COALESCE(application_src, '') = :before_src"
            ), {
                "id": int(change["id"]), "value": change["after"]["application"], "src": change["after"]["src"],
                "confidence": change["after"].get("confidence"),
                "before": (change["before"]["application"] or "").strip(), "before_src": change["before"]["src"] or "",
            }).rowcount
            if done == 1:
                journal["applications"].append({"id": int(change["id"]), "before": change["before"], "after": change["after"]})
            else:
                journal["skipped"].setdefault("applications", {}).setdefault("changed", []).append(change["id"])

        for change in classified.get("details", {}).get("ready", []):
            done = connection.execute(text(
                "UPDATE transactions SET annotation_details = :value, annotation_details_src = :src, annotation_updated_at = :now, "
                "annotation_updated_by = NULL, updated_at = CURRENT_TIMESTAMP WHERE id = :id "
                "AND COALESCE(TRIM(annotation_details), '') = '' AND COALESCE(annotation_details_src, '') = :before_src"
            ), {
                "id": int(change["id"]), "value": change["after"]["details"], "src": change["after"]["src"], "now": now,
                "before_src": change["before"]["src"] or "",
            }).rowcount
            if done != 1:
                journal["skipped"].setdefault("details", {}).setdefault("changed", []).append(change["id"])
                continue
            connection.execute(text(
                "INSERT INTO transaction_annotation_events (transaction_id, field, before_value, after_value, before_src, source) "
                "VALUES (:id, 'details', NULL, :value, :before_src, :src)"
            ), {"id": int(change["id"]), "value": change["after"]["details"], "before_src": change["before"]["src"], "src": change["after"]["src"]})
            journal["details"].append({"id": int(change["id"]), "key": change.get("key"), "before": change["before"], "after": change["after"]})

        # On disk before the commit: a journal without its changes is harmless, changes without a journal are not.
        _write_private(journal_path, journal)
    journal["status"] = "committed"
    _write_private(journal_path, journal)
    return journal


def rollback(engine, journal: Dict[str, Any], parts: Sequence[str], journal_path: Path) -> Dict[str, Any]:
    now = datetime.now(timezone.utc).replace(tzinfo=None)
    undone: Dict[str, List[Any]] = {part: [] for part in parts}
    kept: Dict[str, List[Any]] = {part: [] for part in parts}
    rules_stopped: List[int] = []
    with engine.begin() as connection:
        if "details" in parts:
            # The rules first: a note taken back while its rule is on comes back with the next background pass.
            for key in sorted({record["key"] for record in journal.get("details", []) if record.get("key")}):
                rows = connection.execute(text(
                    "SELECT id FROM annotation_rules WHERE field = 'details' AND operator_key = :key "
                    "AND is_enabled = :on AND is_pinned = :off"
                ), {"key": key, "on": True, "off": False}).fetchall()
                for (rule_id,) in rows:
                    connection.execute(text("UPDATE annotation_rules SET is_enabled = :off WHERE id = :id"), {"off": False, "id": rule_id})
                    rules_stopped.append(int(rule_id))
            for record in journal.get("details", []):
                done = connection.execute(text(
                    "UPDATE transactions SET annotation_details = :before, annotation_details_src = :before_src, annotation_updated_at = :now, "
                    "annotation_updated_by = NULL, updated_at = CURRENT_TIMESTAMP WHERE id = :id "
                    "AND annotation_details = :after AND annotation_details_src = :after_src"
                ), {
                    "id": record["id"], "before": record["before"]["details"], "before_src": record["before"]["src"], "now": now,
                    "after": record["after"]["details"], "after_src": record["after"]["src"],
                }).rowcount
                if done != 1:
                    kept["details"].append(record["id"])
                    continue
                connection.execute(text(
                    "INSERT INTO transaction_annotation_events (transaction_id, field, before_value, after_value, before_src, source) "
                    "VALUES (:id, 'details', :after, :before, :after_src, :after_src)"
                ), {"id": record["id"], "after": record["after"]["details"], "before": record["before"]["details"], "after_src": record["after"]["src"]})
                undone["details"].append(record["id"])
        if "applications" in parts:
            for record in journal.get("applications", []):
                done = connection.execute(text(
                    "UPDATE transactions SET application_mapped = :before, application_src = :before_src, application_confidence = NULL, "
                    "updated_at = CURRENT_TIMESTAMP WHERE id = :id AND application_mapped = :after AND application_src = :after_src"
                ), {
                    "id": record["id"], "before": record["before"]["application"], "before_src": record["before"]["src"],
                    "after": record["after"]["application"], "after_src": record["after"]["src"],
                }).rowcount
                (undone if done == 1 else kept)["applications"].append(record["id"])
        if "dictionary" in parts:
            for record in journal.get("dictionary", []):
                done = connection.execute(text(
                    "DELETE FROM operator_reference WHERE id = :id AND operator_name = :operator AND application_name = :application "
                    "AND is_active = :active AND is_p2p = :p2p"
                ), {"id": record["id"], "operator": record["operator_name"], "application": record["application_name"], "active": True, "p2p": bool(record["is_p2p"])}).rowcount
                (undone if done == 1 else kept)["dictionary"].append(record["id"])
        report = {
            "format": JOURNAL_FORMAT,
            "batch_id": journal["batch_id"],
            "rolled_back_at": now.strftime("%Y-%m-%dT%H:%M:%SZ"),
            "parts": list(parts),
            "undone": undone,
            "kept_because_changed_since": kept,
            "note_rules_switched_off": rules_stopped,
        }
        _write_private(journal_path.with_suffix(journal_path.suffix + ".rollback.json"), report)
    return report


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("command", choices=("verify", "apply", "rollback"))
    parser.add_argument("--manifest", type=Path, help="verify, apply: the reviewed manifest")
    parser.add_argument("--journal", type=Path, help="apply: where to write the journal; rollback: the journal to undo")
    parser.add_argument("--only", default=",".join(PARTS), help="comma-separated parts: dictionary, applications, details")
    parser.add_argument("--i-have-a-verified-backup", action="store_true",
                        help="apply: confirms a fresh database backup was taken and checked")
    parser.add_argument("--database-url", default=None, help="instead of DATABASE_URL")
    args = parser.parse_args(argv)

    parts = tuple(part for part in (item.strip() for item in args.only.split(",")) if part)
    unknown = [part for part in parts if part not in PARTS]
    if unknown or not parts:
        parser.error(f"--only takes {', '.join(PARTS)}")
    url = args.database_url or os.environ.get("DATABASE_URL")
    if not url:
        parser.error("DATABASE_URL is not set")
    engine = create_engine(url)

    if args.command == "rollback":
        if not args.journal:
            parser.error("rollback needs --journal")
        journal = _load(args.journal, JOURNAL_FORMAT)
        report = rollback(engine, journal, parts, args.journal)
        print(f"ROLLED BACK batch {report['batch_id']}")
        for part in parts:
            print(f"  {part:13s} undone {len(report['undone'][part])}, kept because changed since {len(report['kept_because_changed_since'][part])}")
        if "details" in parts:
            print(f"  note rules switched off:  {len(report['note_rules_switched_off'])} (run rollback again in three minutes: see --help)")
        return 0

    if not args.manifest:
        parser.error(f"{args.command} needs --manifest")
    manifest = _load(args.manifest, FORMAT)
    if args.command == "verify":
        _print_counts(f"VERIFY batch {manifest['batch_id']}, nothing written", verify(engine, manifest, parts))
        return 0

    if not args.journal:
        parser.error("apply needs --journal")
    if not args.i_have_a_verified_backup:
        parser.error("apply needs --i-have-a-verified-backup: take a fresh backup and check it first")
    if args.journal.exists():
        parser.error(f"{args.journal} exists: one journal per run, so that a batch can be taken back exactly")
    journal = apply(engine, manifest, parts, args.journal)
    print(f"APPLIED batch {journal['batch_id']}")
    print(f"  dictionary lines added:   {len(journal['dictionary'])}")
    print(f"  «Приложение» filled:      {len(journal['applications'])}")
    print(f"  «Подробности» filled:     {len(journal['details'])}")
    for part, states in journal["skipped"].items():
        for state, items in states.items():
            if items:
                print(f"  {part}: skipped, {state}: {len(items)}")
    print(f"  journal (mode 600):       {args.journal}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
