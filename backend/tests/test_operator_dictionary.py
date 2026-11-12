"""The shared dictionary: what may be added, what waits for review, and what receipts then do.

Synthetic operators only. Every name below is a made-up or public gateway label.
"""
from __future__ import annotations

import asyncio
import json
from datetime import date, datetime
from decimal import Decimal

import pytest

from api.routes import reference
from database.models import LockedPeriod, OperatorReference, Transaction
from parsers.operator_mapper import OperatorMapper, invalidate_operator_mapper_cache
from parsers.parser_orchestrator import ParserOrchestrator
from services import operator_dictionary_service as dictionary
from services.application_backfill import backfill_applications
from services.application_detection import _PERSON_MARKERS, invalidate_confirmed_cache
from services.operator_dictionary_service import DictionaryLine


@pytest.fixture(autouse=True)
def _fresh_caches():
    invalidate_operator_mapper_cache()
    invalidate_confirmed_cache()
    yield
    invalidate_operator_mapper_cache()
    invalidate_confirmed_cache()


def _tx(n, operator, app=None, src=None, when=datetime(2026, 5, 10, 12)):
    return Transaction(
        raw_message=f"m{n}", source_type="AUTO", source_chat_id=1, source_message_id=n,
        transaction_date=when, amount=Decimal("-1000"), transaction_type="DEBIT",
        operator_raw=operator, application_mapped=app, application_src=src,
    )


def _receipt(operator):
    return f"🔴 Oplata\n➖ 40 000.00 UZS\n💳 ***0001\n📍 {operator}\n🕓 14.04.26 23:11\n💵 1 000 000.00 UZS"


RESEARCH = {
    "verified": [
        {
            "id": "novapay", "application": "NovaPay", "confidence": "high",
            "sources": [{"url": "https://novapay.example/about", "quote": "NovaPay app", "checked": "2026-11-12"}],
            "lines": [
                {"operator": "NOVAPAY OPLATA, UZ", "is_p2p": False},
                {"operator": "NOVAPAY P2P U2H>TAS", "is_p2p": True},
            ],
        },
        {"id": "store", "application": "Store Play", "lines": [{"operator": "STOREX", "is_p2p": False}], "sources": []},
    ],
    "needs_owner_review": [
        {"id": "bankx", "candidate": "BankX", "confidence": "medium", "prefixes": ["BANKX"], "lines": [],
         "why_not_verified": "a bank's name alone", "sources": [{"url": "https://bankx.example"}]},
    ],
    "not_an_application": [
        {"id": "c2c", "category": "payment_processing", "prefixes": ["HUMO TO VISA"], "finding": "generic card transfer", "sources": []},
        {"id": "office", "category": "bank_office", "prefixes": ["OPERU"], "exact": ["HUMO"], "finding": "a bank office", "sources": []},
    ],
}


# --- the import plan ------------------------------------------------------------------------

def test_the_plan_adds_only_what_nothing_contradicts(db_session):
    db_session.add_all([
        OperatorReference(operator_name="OLDPAY P2P, UZ", application_name="OldPay", is_p2p=True, is_active=True),
        OperatorReference(operator_name="TAKEN LINE, UZ", application_name="Owner's App", is_p2p=True, is_active=True),
        OperatorReference(operator_name="GUESSED LINE UZ", application_name="AI Guess", is_p2p=False, is_active=False),
        _tx(1, "NOVAPAY OPLATA, UZ"),
        _tx(2, "NOVAPAY OPLATA>TASHKENT"),
        _tx(3, "NOVAPAY OPLATA, UZ", app="Something Else"),
        _tx(4, "HANDSET LINE, UZ", app="By Hand", src="manual"),
        _tx(5, "HANDSET LINE, UZ", app="By Hand", src="manual"),
    ])
    db_session.commit()

    planned = dictionary.plan_import(db_session, [
        DictionaryLine("NOVAPAY OPLATA, UZ", "NovaPay", False),
        DictionaryLine("OLDPAY P2P, UZ", "OldPay", True),               # typed by the owner already
        DictionaryLine("OLDPAY P2P>SAMARKAND", "OldPay", True),         # the same name with a city
        DictionaryLine("TAKEN LINE, UZ", "Another App", False),         # the owner says otherwise
        DictionaryLine("HANDSET LINE, UZ", "Another App", False),       # people say otherwise
        DictionaryLine("GUESSED LINE, UZ", "Verified App", False),      # only an unreviewed guess disagrees
        DictionaryLine("PAYME OPLATA, UZ", "Payme", False),             # a brand rule answers it
        DictionaryLine("PAYME OPLATA, UZ", "Not Payme", False),         # and would be contradicted
        DictionaryLine("NOVAPAY OPLATA, UZ", "NovaPay", False),         # twice in one import
        DictionaryLine("  ", "Nameless", False),
    ])
    actions = [(item.line.operator.strip(), item.line.application, item.action) for item in planned]
    assert actions == [
        ("NOVAPAY OPLATA, UZ", "NovaPay", "add"),
        ("OLDPAY P2P, UZ", "OldPay", "exists"),
        ("OLDPAY P2P>SAMARKAND", "OldPay", "covered"),
        ("TAKEN LINE, UZ", "Another App", "conflict_dictionary"),
        ("HANDSET LINE, UZ", "Another App", "conflict_confirmed"),
        ("GUESSED LINE, UZ", "Verified App", "add"),
        ("PAYME OPLATA, UZ", "Payme", "covered"),
        ("PAYME OPLATA, UZ", "Not Payme", "conflict_dictionary"),
        ("NOVAPAY OPLATA, UZ", "NovaPay", "duplicate"),
        ("", "Nameless", "invalid"),
    ]
    first = planned[0]
    # Country code and city aside, three receipts carry the name; one holds another value, which is reported.
    assert (first.receipts, first.stored_differs) == (3, {"Something Else": 1})
    assert planned[5].inactive_twins == {"AI Guess": 1}
    # Planning is reading: nothing was stored and nobody else's mapper heard of the lines.
    assert db_session.query(OperatorReference).count() == 3
    assert OperatorMapper(db_session).map_operator("NOVAPAY OPLATA, UZ") is None
    text = "\n".join(dictionary.plan_lines(planned))
    assert "add: 2" in text and "conflict_confirmed" in text and "'NOVAPAY OPLATA, UZ' -> 'NovaPay'" in text


def test_preview_lines_live_in_one_mapper_only_and_match_like_stored_ones(db_session):
    db_session.add(OperatorReference(operator_name="OLDPAY P2P, UZ", application_name="OldPay", is_p2p=True, is_active=True))
    db_session.commit()
    mapper = OperatorMapper(db_session)
    mapper.preview_rows([("NOVAPAY KONVERS 2026 UZCARD, UZ", "NovaPay", False), ("STOREX", "Store Play", False), ("", "Nothing", False)])

    exact = mapper.map_operator_details("NOVAPAY KONVERS 2026 UZCARD, UZ")
    assert (exact["application_name"], exact["match_kind"], exact["reference_id"] < 0) == ("NovaPay", "exact", True)
    # The same name with the old country code, and cut at twenty characters by HUMO.
    assert mapper.map_operator_details("NOVAPAY KONVERS 2026 UZCARD, 99")["match_kind"] == "variant"
    assert mapper.map_operator_details("NOVAPAY KONVERS 2026")["match_kind"] == "truncated"
    # One word inside a longer line: the historical rule.
    assert mapper.map_operator_details("STOREX *Video Editor")["match_kind"] == "substring"
    # Stored lines go on working beside them, and a refresh keeps the preview.
    mapper.refresh_cache()
    assert mapper.map_operator("OLDPAY P2P, UZ") == "OldPay" and mapper.map_operator("STOREX") == "Store Play"

    # Nobody else sees them: not another mapper, not the shared cache, not the table.
    assert OperatorMapper(db_session).map_operator("STOREX") is None
    assert db_session.query(OperatorReference).count() == 1


def test_a_dry_run_with_previewed_lines_names_each_receipt_and_writes_nothing(db_session):
    db_session.add_all([
        _tx(1, "NOVAPAY OPLATA, UZ"),
        _tx(2, "NOVAPAY OPLATA>TASHKENT"),
        _tx(3, "NOVAPAY OPLATA, UZ", app="Typed long ago"),            # unknown origin: never touched
        _tx(4, "NOVAPAY OPLATA, UZ", app="By Hand", src="manual"),      # a person's: never touched
        _tx(5, "NOVAPAY OPLATA, UZ", when=datetime(2026, 1, 15, 9)),    # in a closed month
        _tx(6, "SHOP, Tashkent, Some street 1"),
        LockedPeriod(date_from=date(2026, 1, 1), date_to=date(2026, 1, 31), is_active=True, locked_by_tg_id=1),
    ])
    db_session.commit()
    lines = dictionary.verified_lines(RESEARCH)

    report = backfill_applications(db_session, detector=dictionary.preview_detector(db_session, lines))
    assert (report.matched, report.locked, report.unmatched) == (2, 1, 1)
    ids = {t.source_message_id: t.id for t in db_session.query(Transaction)}
    assert report.changes == [
        {"id": ids[1], "before": {"application": None, "src": None}, "after": {"application": "NovaPay", "src": "dict", "confidence": 1.0}, "reason": "dictionary_exact"},
        {"id": ids[2], "before": {"application": None, "src": None}, "after": {"application": "NovaPay", "src": "dict", "confidence": 0.95}, "reason": "dictionary_variant"},
    ]
    assert report.disagreements == {("Typed long ago", "NovaPay"): 1}
    # Nothing was written, and without the preview the same pass finds nothing.
    assert {t.application_mapped for t in db_session.query(Transaction)} == {None, "Typed long ago", "By Hand"}
    assert backfill_applications(db_session).matched == 0


# --- a dictionary edit reaches the next receipt -----------------------------------------------

def test_a_line_added_in_the_reference_screen_is_used_by_the_next_receipt(db_session):
    orchestrator = lambda: ParserOrchestrator(db_session=db_session)  # noqa: E731  (the worker builds one per receipt)
    assert orchestrator().process(_receipt("NOVAPAY OPLATA, UZ"))["application_mapped"] is None

    created = asyncio.run(reference.create_operator(
        operator=reference.OperatorReferenceCreate(operator_name="NOVAPAY OPLATA, UZ", application_name="NovaPay", is_p2p=False),
        db=db_session, current_user={},
    ))
    parsed = orchestrator().process(_receipt("NOVAPAY OPLATA, UZ"))
    assert (parsed["application_mapped"], parsed["application_src"], parsed["application_confidence"]) == ("NovaPay", "dict", 1.0)
    assert orchestrator().process(_receipt("NOVAPAY OPLATA>TASHKENT"))["application_mapped"] == "NovaPay"

    # Renamed, switched off and deleted there: each is heard at once as well.
    asyncio.run(reference.update_operator(created.id, reference.OperatorReferenceUpdate(application_name="Nova"), db=db_session, current_user={}))
    assert orchestrator().process(_receipt("NOVAPAY OPLATA, UZ"))["application_mapped"] == "Nova"
    asyncio.run(reference.update_operator(created.id, reference.OperatorReferenceUpdate(is_active=False), db=db_session, current_user={}))
    assert orchestrator().process(_receipt("NOVAPAY OPLATA, UZ"))["application_mapped"] is None
    asyncio.run(reference.update_operator(created.id, reference.OperatorReferenceUpdate(is_active=True), db=db_session, current_user={}))
    asyncio.run(reference.delete_operator(created.id, db=db_session, current_user={}))
    assert orchestrator().process(_receipt("NOVAPAY OPLATA, UZ"))["application_mapped"] is None


def test_a_line_written_past_the_routes_waits_for_the_cache_and_a_duplicate_is_refused(db_session):
    assert OperatorMapper(db_session).map_operator("NOVAPAY OPLATA, UZ") is None
    db_session.add(OperatorReference(operator_name="OTHER LINE, UZ", application_name="Other", is_p2p=False, is_active=True))
    db_session.commit()
    assert OperatorMapper(db_session).map_operator("OTHER LINE, UZ") == "Other"  # first load
    # Written straight into the table, as an administrator's import does: other processes hear of it
    # when their cache runs out (OPERATOR_MAPPER_CACHE_TTL, two minutes), not before.
    db_session.add(OperatorReference(operator_name="NOVAPAY OPLATA, UZ", application_name="NovaPay", is_p2p=False, is_active=True))
    db_session.commit()
    assert OperatorMapper(db_session).map_operator("NOVAPAY OPLATA, UZ") is None
    invalidate_operator_mapper_cache()
    assert OperatorMapper(db_session).map_operator("NOVAPAY OPLATA, UZ") == "NovaPay"

    with pytest.raises(reference.HTTPException) as refused:
        asyncio.run(reference.create_operator(
            operator=reference.OperatorReferenceCreate(operator_name="NOVAPAY OPLATA, UZ", application_name="NovaPay"),
            db=db_session, current_user={},
        ))
    assert refused.value.status_code == 400
    assert db_session.query(OperatorReference).filter_by(operator_name="NOVAPAY OPLATA, UZ").count() == 1


def test_receipts_parsed_before_the_line_are_filled_once_and_a_persons_values_stay(db_session):
    db_session.add_all([
        _tx(1, "NOVAPAY OPLATA, UZ"),
        _tx(2, "NOVAPAY OPLATA, UZ", app="By Hand", src="manual"),
        _tx(3, "NOVAPAY OPLATA, UZ", app="Typed long ago"),
        _tx(4, "NOVAPAY OPLATA, UZ", app=None, src="manual"),           # emptied by a person on purpose
    ])
    db_session.commit()
    asyncio.run(reference.create_operator(
        operator=reference.OperatorReferenceCreate(operator_name="NOVAPAY OPLATA, UZ", application_name="NovaPay"),
        db=db_session, current_user={},
    ))

    first = backfill_applications(db_session, apply=True)
    assert (first.matched, first.manual) == (1, 2)
    stored = {t.source_message_id: (t.application_mapped, t.application_src) for t in db_session.query(Transaction)}
    assert stored == {1: ("NovaPay", "dict"), 2: ("By Hand", "manual"), 3: ("Typed long ago", None), 4: (None, "manual")}
    # Again, and nothing moves: no second write, no duplicate.
    second = backfill_applications(db_session, apply=True)
    assert (second.matched, second.changes) == (0, [])
    assert {t.source_message_id: (t.application_mapped, t.application_src) for t in db_session.query(Transaction)} == stored


# --- operators that wait for a person -----------------------------------------------------------

def test_unknown_operators_are_listed_for_review_and_never_guessed(db_session):
    db_session.add_all([
        OperatorReference(operator_name="OLDPAY P2P, UZ", application_name="OldPay", is_p2p=True, is_active=True),
        OperatorReference(operator_name="MYSTERY GATE UZ", application_name="AI Guess", is_p2p=False, is_active=False),
    ])
    rows = [
        ("NOVAPAY OPLATA, UZ", 3), ("NOVAPAY P2P U2H>BUX", 1), ("STOREX *Video Editor", 2),   # verified, not imported yet
        ("BANKX P2P HUMO, UZ", 2),                                                              # researched, not proven
        ("MYSTERY GATE, UZ", 1),                                                                # an unreviewed AI line
        ("HUMO TO VISA>Tashken", 4), ("OPERU BANKX AT BANKI, UZ", 2), ("HUMO", 1),               # not an application
        ("OOO ROMASHKA, Tashkent, Some street 1", 5), ("YATT PERSON NAME, UZ", 1), ("ROMASHKA MCHJ>TAS", 1),
        ("Unknown", 2), ("XKKM 00014 ATM 000001, UZ", 1),
        ("BRAND NEW GATE, UZ", 6),                                                              # nobody has looked yet
        ("OLDPAY P2P, UZ", 3),                                                                  # answered: not listed
    ]
    number = 0
    for operator, count in rows:
        for _ in range(count):
            number += 1
            db_session.add(_tx(number, operator, when=datetime(2026, 5, min(number, 28), 12)))
    db_session.add(_tx(900, "NOVAPAY OPLATA, UZ", app="Typed long ago"))
    db_session.commit()

    items = dictionary.review_operators(db_session, RESEARCH)
    status = {item["operator"]: item["status"] for item in items}
    assert status == {
        "NOVAPAY OPLATA, UZ": "verified_pending_import",
        "NOVAPAY P2P U2H>BUX": "verified_pending_import",
        "STOREX *Video Editor": "verified_pending_import",
        "BANKX P2P HUMO, UZ": "needs_owner_review",
        "MYSTERY GATE, UZ": "needs_owner_review",
        "HUMO TO VISA>Tashken": "not_an_application",
        "OPERU BANKX AT BANKI, UZ": "not_an_application",
        "HUMO": "not_an_application",
        "OOO ROMASHKA, Tashkent, Some street 1": "merchant_or_individual",
        "YATT PERSON NAME, UZ": "merchant_or_individual",
        "ROMASHKA MCHJ>TAS": "merchant_or_individual",
        "Unknown": "no_operator_or_cash",
        "XKKM 00014 ATM 000001, UZ": "no_operator_or_cash",
        "BRAND NEW GATE, UZ": "unreviewed",
    }
    by_operator = {item["operator"]: item for item in items}
    nova = by_operator["NOVAPAY OPLATA, UZ"]
    # Four receipts carry the line, three of them without an application; the candidate comes with its source.
    assert (nova["receipts"], nova["without_application"], nova["candidate"], nova["confidence"]) == (4, 3, "NovaPay", "high")
    assert nova["sources"] == ["https://novapay.example/about"] and nova["last_seen"] == "2026-05-10"
    assert by_operator["BANKX P2P HUMO, UZ"]["candidate"] == "BankX"
    assert by_operator["MYSTERY GATE, UZ"]["candidate"] == "AI Guess"
    # A line with an address or a person's name is counted, never handed out.
    assert not by_operator["OOO ROMASHKA, Tashkent, Some street 1"]["shareable"]
    assert not by_operator["YATT PERSON NAME, UZ"]["shareable"]

    summary = dictionary.review_summary(items)
    assert summary["verified_pending_import"] == {"operators": 3, "receipts": 6}
    assert summary["unreviewed"] == {"operators": 1, "receipts": 6}
    assert summary["merchant_or_individual"] == {"operators": 3, "receipts": 7}
    text = "\n".join(dictionary.review_lines(items))
    assert "'BRAND NEW GATE, UZ'" in text and "ROMASHKA" not in text and "PERSON NAME" not in text
    assert "PERSON NAME" in "\n".join(dictionary.review_lines(items, names=True))
    # Reviewing wrote nothing and decided nothing.
    assert db_session.query(OperatorReference).count() == 2
    assert db_session.query(Transaction).filter(Transaction.application_mapped.isnot(None)).count() == 1


def test_the_review_route_lists_gateway_lines_only(db_session, monkeypatch):
    monkeypatch.setattr(dictionary, "load_research", lambda path=None: RESEARCH)
    db_session.add_all([_tx(1, "BRAND NEW GATE, UZ"), _tx(2, "OOO ROMASHKA, Tashkent, Some street 1"), _tx(3, "NOVAPAY OPLATA, UZ")])
    db_session.commit()
    reply = asyncio.run(reference.review_unknown_operators(limit=200, db=db_session, current_user={}))
    assert [item["operator"] for item in reply["items"]] == ["NOVAPAY OPLATA, UZ", "BRAND NEW GATE, UZ"]
    assert reply["total"] == 2 and reply["summary"]["merchant_or_individual"] == {"operators": 1, "receipts": 1}


# --- the research file that ships with the code ------------------------------------------------

def test_the_research_file_is_complete_and_holds_no_private_data():
    research = dictionary.load_research()
    lines = dictionary.verified_lines(research)
    assert lines and len({OperatorMapper.normalize_operator(line.operator) for line in lines}) == len(lines)
    for entry in research["verified"]:
        assert entry["application"].strip() and entry["confidence"] and entry["rationale"] and entry["label_basis"], entry["id"]
        assert entry["sources"], entry["id"]
        for source in entry["sources"]:
            assert source["url"].startswith("https://") and source["quote"].strip() and source["checked"] == research["researched_on"]
    for entry in research["needs_owner_review"]:
        assert entry["candidate"] and entry["why_not_verified"], entry["id"]
    for entry in research["not_an_application"]:
        assert entry["category"] and entry["finding"], entry["id"]

    # Public organisation and service names only: no person's line, no card, no phone number.
    def operators():
        for section in ("verified", "needs_owner_review", "not_an_application"):
            for entry in research[section]:
                yield from (line["operator"] for line in entry.get("lines", []))
                yield from entry.get("prefixes", [])
                yield from entry.get("exact", [])

    for operator in operators():
        tokens = OperatorMapper.normalize_operator(operator).split()
        assert not set(tokens) & _PERSON_MARKERS, operator
        assert not any(token.isdigit() and len(token) >= 7 for token in tokens), operator
    text = json.dumps(research, ensure_ascii=False)
    assert "annotation_details" not in text and "card_last_4" not in text
    # A verified application never doubles as an unresolved candidate for the same line.
    unresolved = {OperatorMapper.normalize_operator(line["operator"]) for entry in research["needs_owner_review"] for line in entry.get("lines", [])}
    assert not unresolved & {OperatorMapper.normalize_operator(line.operator) for line in lines}
