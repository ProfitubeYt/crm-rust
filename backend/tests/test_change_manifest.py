"""The change manifest and the tool that applies it: exact, journalled, and undone one batch at a time.

Synthetic receipts only. The tool gets its own connection, as it does on a server.
"""
from __future__ import annotations

import json
import stat
from datetime import date, datetime
from decimal import Decimal

import pytest
from sqlalchemy import create_engine
from sqlalchemy.orm import sessionmaker

from database.models import Base, LockedPeriod, OperatorReference, Transaction, TransactionAnnotationEvent
from parsers.operator_mapper import invalidate_operator_mapper_cache
from scripts import apply_change_manifest as tool
from scripts import build_change_manifest as builder
from services import annotation_rule_service
from services.application_detection import invalidate_confirmed_cache

RESEARCH = {
    "title": "synthetic research",
    "researched_on": "2026-11-12",
    "verified": [
        {"id": "novapay", "application": "NovaPay", "lines": [
            {"operator": "NOVAPAY OPLATA, UZ", "is_p2p": False},
            {"operator": "NOVAPAY P2P U2H>TAS", "is_p2p": True},
            {"operator": "OLDPAY P2P, UZ", "is_p2p": True},
        ]},
    ],
}
NOTE = "услуги мониторинга"


@pytest.fixture(autouse=True)
def _fresh_caches():
    invalidate_operator_mapper_cache()
    invalidate_confirmed_cache()
    annotation_rule_service.invalidate_rule_cache()
    yield
    invalidate_operator_mapper_cache()
    invalidate_confirmed_cache()
    annotation_rule_service.invalidate_rule_cache()


@pytest.fixture
def database(tmp_path):
    url = f"sqlite:///{tmp_path / 'receipts.sqlite'}"
    engine = create_engine(url)
    Base.metadata.create_all(engine)
    session = sessionmaker(bind=engine)()
    try:
        yield url, session
    finally:
        session.close()
        engine.dispose()


def _tx(n, operator, *, app=None, src=None, note=None, note_src=None, card="1111", when=None):
    return Transaction(
        raw_message=f"m{n}", source_type="AUTO", source_chat_id=1, source_message_id=n,
        transaction_date=when or datetime(2026, 5, 1 + n % 27, 12, n % 60), amount=Decimal("-1000"), transaction_type="DEBIT",
        operator_raw=operator, application_mapped=app, application_src=src, card_last_4=card,
        annotation_details=note, annotation_details_src=note_src,
    )


def _seed(db):
    db.add(OperatorReference(operator_name="OLDPAY P2P, UZ", application_name="OldPay", is_p2p=True, is_active=True))
    db.add_all([
        _tx(1, "NOVAPAY OPLATA, UZ"),
        _tx(2, "NOVAPAY OPLATA>TASHKENT"),
        _tx(3, "NOVAPAY OPLATA, UZ", app="Typed long ago"),
        _tx(4, "NOVAPAY OPLATA, UZ", app="NovaPay", src="manual"),         # set by a person: theirs, whatever it says
        _tx(5, "PAYME OPLATA, UZ"),
        _tx(6, "SHOP, Tashkent, Some street 1"),
    ])
    # A confirmed pattern: one operator, one card, one direction, always the same note.
    db.add_all([_tx(10 + n, "MONITOR FEE, UZ", note=NOTE, note_src="import") for n in range(5)])
    db.add_all([
        _tx(20, "MONITOR FEE, UZ"),
        _tx(21, "MONITOR FEE, UZ"),
        _tx(22, "MONITOR FEE, UZ", card="2222"),                        # another card: another pattern
        _tx(23, "MONITOR FEE, UZ", when=datetime(2026, 1, 15, 9)),       # a closed month
    ])
    # Three confirmed receipts are a suggestion, not a rule.
    db.add_all([_tx(30 + n, "RARE FEE, UZ", note="абон.плата", note_src="import") for n in range(3)])
    db.add(_tx(35, "RARE FEE, UZ"))
    # What automation wrote is never evidence, however often it repeats.
    db.add_all([_tx(40 + n, "AGENT OP, UZ", note="догадка", note_src="agent") for n in range(6)])
    db.add_all([_tx(50 + n, "RULE OP, UZ", note="старое правило", note_src="rule") for n in range(6)])
    db.add_all([_tx(60, "AGENT OP, UZ"), _tx(61, "RULE OP, UZ")])
    db.add(LockedPeriod(date_from=date(2026, 1, 1), date_to=date(2026, 1, 31), is_active=True, locked_by_tg_id=1))
    db.commit()


def _ids(db):
    return {t.source_message_id: t.id for t in db.query(Transaction)}


def _state(db, number):
    db.expire_all()
    t = db.query(Transaction).filter_by(source_message_id=number).one()
    return (t.application_mapped, t.application_src, t.annotation_details, t.annotation_details_src)


def test_the_manifest_lists_every_change_and_nothing_private_reaches_the_console(database, capsys, tmp_path):
    url, db = database
    _seed(db)
    ids = _ids(db)
    manifest = builder.build_manifest(db, RESEARCH, batch_id="batch-1")

    assert manifest["format"] == "tbs-change-manifest/1" and manifest["batch_id"] == "batch-1"
    # Two new lines are offered. The third the owner files under another application:
    # it is left out and reported, never overwritten.
    assert [line["operator_name"] for line in manifest["dictionary"]["add"]] == ["NOVAPAY OPLATA, UZ", "NOVAPAY P2P U2H>TAS"]
    assert [(line["operator_name"], line["action"]) for line in manifest["dictionary"]["not_added"]] == [("OLDPAY P2P, UZ", "conflict_dictionary")]

    changes = {change["id"]: change for change in manifest["applications"]["changes"]}
    assert set(changes) == {ids[1], ids[2], ids[5]}
    assert changes[ids[1]] == {"id": ids[1], "before": {"application": None, "src": None}, "after": {"application": "NovaPay", "src": "dict", "confidence": 1.0}, "reason": "dictionary_exact"}
    assert changes[ids[5]]["after"] == {"application": "Payme", "src": "rule", "confidence": 0.9}
    summary = manifest["applications"]["summary"]
    assert (summary["would_fill"], summary["manual"], summary["stored_differs_from_evidence"]) == (3, 1, {"Typed long ago -> NovaPay": 1})

    fills = {fill["id"]: fill for fill in manifest["details"]["fills"]}
    assert set(fills) == {ids[20], ids[21]}
    assert fills[ids[20]] == {
        "id": ids[20], "key": "MONITOR FEE UZ | ПК 1111 | DEBIT", "before": {"details": None, "src": None},
        "after": {"details": NOTE, "src": "rule"}, "reason": "confirmed_similar", "support": 5, "total": 5,
    }
    # Weaker evidence is set apart; a closed month is set apart; automation's own notes teach nothing.
    assert [item["id"] for item in manifest["details"]["suggestions"]] == [ids[35]]
    assert [item["id"] for item in manifest["details"]["locked"]] == [ids[23]]
    assert ids[60] not in fills and ids[61] not in fills and ids[22] not in fills
    # The same counts as the report the administrator already knows.
    report = annotation_rule_service.details_report(db)
    notes = manifest["details"]["summary"]
    assert (notes["would_fill"], notes["locked"], sum(notes["suggestions"].values())) == (report["would_fill"], report["locked"], sum(report["suggestions"].values()))

    # The digest is of the planned changes alone: the same data gives the same digest.
    again = builder.build_manifest(db, RESEARCH, batch_id="another-name")
    assert again["content_sha256"] == manifest["content_sha256"] and len(manifest["content_sha256"]) == 64
    db.add(_tx(70, "NOVAPAY OPLATA, UZ"))
    db.commit()
    assert builder.build_manifest(db, RESEARCH, batch_id="batch-1")["content_sha256"] != manifest["content_sha256"]

    # What is printed is counts: no receipt, no operator line, no note.
    printed = "\n".join(builder.summary_lines(manifest))
    assert "would be filled:              3" in printed and manifest["content_sha256"] in printed
    assert NOTE not in printed and "NOVAPAY" not in printed and "SHOP" not in printed
    # Building wrote nothing.
    assert db.query(OperatorReference).count() == 1 and _state(db, 1) == (None, None, None, None)


def test_the_manifest_is_refused_inside_the_repository(database, monkeypatch):
    url, db = database
    monkeypatch.setattr(builder, "SessionLocal", lambda: db)
    inside = builder.ROOT / "manifest.json"
    with pytest.raises(SystemExit):
        builder.main(["--out", str(inside)])
    assert not inside.exists()


def test_applying_writes_exactly_the_manifest_and_a_batch_is_taken_back_alone(database, tmp_path, capsys):
    url, db = database
    _seed(db)
    ids = _ids(db)
    manifest_path = tmp_path / "private" / "manifest.json"
    manifest_path.parent.mkdir()
    manifest_path.write_text(json.dumps(builder.build_manifest(db, RESEARCH, batch_id="batch-1"), ensure_ascii=False, default=str), encoding="utf-8")
    journal_path = tmp_path / "private" / "journal.json"
    run = lambda *args: tool.main([*args, "--database-url", url])  # noqa: E731

    # Checking reads only.
    assert run("verify", "--manifest", str(manifest_path)) == 0
    assert "dictionary    ready 2" in capsys.readouterr().out
    assert db.query(OperatorReference).count() == 1

    # Between the review and the run a person fills one receipt and closes a month.
    edited = db.query(Transaction).filter_by(source_message_id=2).one()
    edited.application_mapped, edited.application_src = "Set meanwhile", "manual"
    db.add(LockedPeriod(date_from=date(2026, 5, 22), date_to=date(2026, 5, 22), is_active=True, locked_by_tg_id=1))  # receipt 21
    db.commit()

    # No backup, no run.
    with pytest.raises(SystemExit):
        run("apply", "--manifest", str(manifest_path), "--journal", str(journal_path))
    assert not journal_path.exists()

    assert run("apply", "--manifest", str(manifest_path), "--journal", str(journal_path), "--i-have-a-verified-backup") == 0
    printed = capsys.readouterr().out
    assert "dictionary lines added:   2" in printed and "«Приложение» filled:      2" in printed and "«Подробности» filled:     1" in printed
    assert "applications: skipped, changed: 1" in printed and "details: skipped, locked: 1" in printed

    assert _state(db, 1) == ("NovaPay", "dict", None, None)
    assert _state(db, 5) == ("Payme", "rule", None, None)
    assert _state(db, 2)[:2] == ("Set meanwhile", "manual")          # the person's value stands
    assert _state(db, 3)[:2] == ("Typed long ago", None) and _state(db, 4)[:2] == ("NovaPay", "manual")
    assert _state(db, 20)[2:] == (NOTE, "rule") and _state(db, 21)[2:] == (None, None)
    assert _state(db, 35)[2:] == (None, None)                         # a suggestion is never applied
    filled = db.query(Transaction).filter_by(source_message_id=20).one()
    assert filled.annotation_updated_at is not None and float(db.query(Transaction).filter_by(source_message_id=1).one().application_confidence) == 1.0
    lines = {(row.operator_name, row.application_name, row.is_p2p, row.is_active) for row in db.query(OperatorReference)}
    assert lines == {("OLDPAY P2P, UZ", "OldPay", True, True), ("NOVAPAY OPLATA, UZ", "NovaPay", False, True), ("NOVAPAY P2P U2H>TAS", "NovaPay", True, True)}
    events = [(e.transaction_id, e.field, e.before_value, e.after_value, e.before_src, e.source) for e in db.query(TransactionAnnotationEvent)]
    assert events == [(ids[20], "details", None, NOTE, None, "rule")]

    journal = json.loads(journal_path.read_text(encoding="utf-8"))
    assert (journal["format"], journal["batch_id"], journal["status"]) == ("tbs-change-journal/1", "batch-1", "committed")
    assert [record["id"] for record in journal["applications"]] == [ids[1], ids[5]] and [record["id"] for record in journal["details"]] == [ids[20]]
    assert journal["skipped"]["applications"] == {"changed": [ids[2]]} and journal["skipped"]["details"] == {"locked": [ids[21]]}
    assert stat.S_IMODE(journal_path.stat().st_mode) == 0o600

    # A journal is one run: it cannot be written over. A second run finds nothing left to do.
    with pytest.raises(SystemExit):
        run("apply", "--manifest", str(manifest_path), "--journal", str(journal_path), "--i-have-a-verified-backup")
    second = tmp_path / "private" / "journal-2.json"
    assert run("apply", "--manifest", str(manifest_path), "--journal", str(second), "--i-have-a-verified-backup") == 0
    again = json.loads(second.read_text(encoding="utf-8"))
    assert (again["dictionary"], again["applications"], again["details"]) == ([], [], [])
    assert db.query(OperatorReference).count() == 3 and db.query(TransactionAnnotationEvent).count() == 1

    # The application's own passes agree with what was written: nothing more to fill, nothing to correct.
    invalidate_operator_mapper_cache()
    from services.application_backfill import backfill_applications
    after = backfill_applications(db, revalidate=True)
    assert (after.matched, after.stale) == (0, 0)

    # The administrator then lets the application learn its rules, as `enrich_details --apply` does:
    # the rule behind the note exists from now on, and the background pass fills new receipts with it.
    from database.models import AnnotationRule
    learned = annotation_rule_service.recompute(db, fields=("details",))
    assert learned["created"] == 1
    rule = db.query(AnnotationRule).filter_by(field="details").one()
    assert (rule.operator_key, rule.value, rule.is_enabled) == ("MONITOR FEE UZ | ПК 1111 | DEBIT", NOTE, True)
    db.add(_tx(81, "MONITOR FEE, UZ"))
    db.commit()
    annotation_rule_service.fill_defaults(db)
    assert _state(db, 81)[2:] == (NOTE, "rule")

    # Life goes on: a person corrects one filled receipt and renames one new line; a new receipt arrives.
    fixed = db.query(Transaction).filter_by(source_message_id=1).one()
    fixed.application_mapped, fixed.application_src = "Fixed by hand", "manual"
    db.query(OperatorReference).filter_by(operator_name="NOVAPAY P2P U2H>TAS").one().application_name = "Nova Transfers"
    db.add(_tx(80, "NOVAPAY OPLATA, UZ", app="NovaPay", src="dict"))
    db.commit()

    assert run("rollback", "--journal", str(journal_path)) == 0
    printed = capsys.readouterr().out
    assert "applications  undone 1, kept because changed since 1" in printed and "dictionary    undone 1, kept because changed since 1" in printed
    assert _state(db, 5) == (None, None, None, None)                  # the batch's fill is taken back
    assert _state(db, 1)[:2] == ("Fixed by hand", "manual")           # the later manual edit stays
    assert _state(db, 80)[:2] == ("NovaPay", "dict")                  # so does the newer receipt
    assert _state(db, 20)[2:] == (None, None)
    assert _state(db, 81)[2:] == (NOTE, "rule")                       # a newer receipt is not the batch's
    assert {row.operator_name for row in db.query(OperatorReference)} == {"OLDPAY P2P, UZ", "NOVAPAY P2P U2H>TAS"}
    # The rule behind the note is off, so the background pass does not write the note back.
    db.expire_all()
    assert db.query(AnnotationRule).filter_by(field="details").one().is_enabled is False
    annotation_rule_service.invalidate_rule_cache()                   # a worker's cache runs out in two minutes
    annotation_rule_service.fill_defaults(db)
    assert _state(db, 20)[2:] == (None, None)
    events = [(e.transaction_id, e.before_value, e.after_value, e.source) for e in db.query(TransactionAnnotationEvent).filter_by(field="details").order_by(TransactionAnnotationEvent.id)]
    assert events == [(ids[20], None, NOTE, "rule"), (_ids(db)[81], None, NOTE, "rule"), (ids[20], NOTE, None, "rule")]
    report = json.loads((tmp_path / "private" / "journal.json.rollback.json").read_text(encoding="utf-8"))
    assert report["undone"] == {"dictionary": [journal["dictionary"][0]["id"]], "applications": [ids[5]], "details": [ids[20]]}
    assert report["note_rules_switched_off"] == [rule.id]
    # Taken back twice, nothing more happens.
    assert run("rollback", "--journal", str(journal_path)) == 0
    assert "applications  undone 0" in capsys.readouterr().out and _state(db, 1)[:2] == ("Fixed by hand", "manual")


def test_a_part_of_the_manifest_can_be_run_alone_and_the_tool_stands_by_itself(database, tmp_path):
    url, db = database
    _seed(db)
    manifest_path = tmp_path / "manifest.json"
    manifest_path.write_text(json.dumps(builder.build_manifest(db, RESEARCH, batch_id="batch-2"), ensure_ascii=False, default=str), encoding="utf-8")
    journal_path = tmp_path / "journal.json"
    assert tool.main(["apply", "--manifest", str(manifest_path), "--journal", str(journal_path), "--only", "dictionary", "--i-have-a-verified-backup", "--database-url", url]) == 0
    assert db.query(OperatorReference).count() == 3
    assert _state(db, 1) == (None, None, None, None) and _state(db, 20) == (None, None, None, None)
    with pytest.raises(SystemExit):
        tool.main(["verify", "--manifest", str(manifest_path), "--only", "everything", "--database-url", url])
    with pytest.raises(SystemExit):
        tool.main(["verify", "--manifest", str(journal_path), "--database-url", url])   # a journal is not a manifest
    # It runs against whatever backend is live: nothing of the application is imported.
    source = open(tool.__file__, encoding="utf-8").read()
    for module in ("database", "services", "parsers", "api", "workers"):
        assert f"from {module}" not in source and f"import {module}" not in source
