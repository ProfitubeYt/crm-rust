"""Evidence tiers of the application detector, on synthetic operators only."""
from datetime import datetime
from decimal import Decimal

import pytest

from database.models import OperatorReference, Transaction
from parsers.operator_mapper import OperatorMapper, invalidate_operator_mapper_cache, operator_form
from services.application_detection import (
    BRAND_RULES,
    ApplicationDetector,
    brand_applications,
    invalidate_confirmed_cache,
    load_confirmed_applications,
    mark_manual_application,
    may_replace_application,
    stamp_application,
)


@pytest.fixture(autouse=True)
def _fresh_caches():
    invalidate_operator_mapper_cache()
    invalidate_confirmed_cache()
    yield
    invalidate_operator_mapper_cache()
    invalidate_confirmed_cache()


#: A slice of the owner's reference list (backend/ops.txt).
OWNER_LINES = [
    ("NBU 2P2 U PLAT UZCARD, 99", "Milliy 2.0"),
    ("NBU P2P HUMO UZCARD>", "Milliy 2.0"),
    ("NBU P2P UZCARD HUMO, 99", "Milliy 2.0"),
    ("UPAY P2P, UZ", "Humans"),
    ("DAVR UPAY HUMANS UZCARD2HU, UZ", "Humans"),
    ("XAZNA OTHERS 2 ANY, 99", "Xazna"),
    ("DAVR MOBILE P2P H2U>", "Davr Mobile"),
    ("HAMKORBANK ATB, UZ", "Hamkor"),
    ("HAMKOR P2P UZKARD>AN", "Hamkor"),
    ("TOSHKENT SH., AT KHALK BANKI BOSH AMALIY, 99", "Paynet"),
    ("UZPAYNET>Shayxontoxu", "Paynet"),
    ("UZCARD OTHERS 2 ANY PAYNET, 99", "Paynet"),
    ("UZCARD PLYUS P2P, 99", "Joyda"),
    ("UZCARD P2P, UZ", "Agrobank"),
    ("ChakanaPay Humo Uzcard", "Chakanapay"),
    ("ChakanaPay Humo to Uzcard", "Chakanapay"),
    ("ASAKABANK HUMO UZCAR", "Asakabank"),
    ("PAYME P2P, UZ", "Payme"),
    ("SQB MOBILE HUMO P2P", "SQB"),
    ("OQ P2P, UZ", "OQ"),
]


def _detector(db, extra=(), confirmed=None):
    db.add_all([
        OperatorReference(operator_name=name, application_name=app, is_p2p=True, is_active=True)
        for name, app in list(OWNER_LINES) + list(extra)
    ])
    db.commit()
    mapper = OperatorMapper(db)
    mapper.refresh_cache()
    return ApplicationDetector(mapper, confirmed=confirmed)


# --- the owner's own lines and their meaningless variants -----------------------

def test_every_owner_line_still_resolves_to_its_own_application(db_session):
    detector = _detector(db_session)
    for name, app in OWNER_LINES:
        found = detector.detect(name)
        assert (found.application, found.source, found.reason) == (app, "dict", "dictionary_exact"), name
        assert found.confidence == 1.0


@pytest.mark.parametrize("operator, app, reason", [
    # CardXabar now prints the country as UZ where the dictionary has 99.
    ("NBU 2P2 U PLAT UZCARD, UZ", "Milliy 2.0", "dictionary_variant"),
    ("TOSHKENT SH., AT KHALK BANKI BOSH AMALIY, UZ", "Paynet", "dictionary_variant"),
    ("UZCARD OTHERS 2 ANY PAYNET, UZ", "Paynet", "dictionary_variant"),
    # HUMO puts another city after the arrow.
    ("UZPAYNET>Toshkent Sh", "Paynet", "dictionary_variant"),
    ("HAMKOR P2P UZKARD>TA", "Hamkor", "dictionary_variant"),
    # HUMO cuts the line at 20 characters; the dictionary has the long spelling.
    ("ChakanaPay Humo Uzca", "Chakanapay", "dictionary_truncated"),
    # ... or the dictionary has the cut spelling and the receipt the long one.
    ("ASAKABANK HUMO UZCARD P2P, UZ", "Asakabank", "dictionary_truncated"),
])
def test_country_code_city_and_cut_do_not_hide_a_dictionary_line(db_session, operator, app, reason):
    found = _detector(db_session).detect(operator)
    assert (found.application, found.source, found.reason) == (app, "dict", reason)
    assert found.is_p2p is True
    assert 0.9 <= found.confidence < 1.0


def test_a_generic_dictionary_line_does_not_spread_to_longer_operators(db_session):
    detector = _detector(db_session)
    # `UZCARD P2P, UZ` is Agrobank only as the whole line.
    assert detector.detect("UZCARD P2P, 99").application == "Agrobank"
    assert detector.detect("UZCARD P2P HUMO BOSHQA, UZ").application is None
    assert detector.detect("UZCARD TO VISA, UZ").application is None


def test_two_dictionary_lines_with_one_name_and_two_applications_are_ambiguous(db_session):
    detector = _detector(db_session, extra=[("GATE X PAY, 99", "First"), ("GATE X PAY, 77", "Second")])
    found = detector.detect("GATE X PAY, UZ")
    assert found.application is None and found.reason == "ambiguous"
    assert found.candidates == ("First", "Second")


# --- an application named in the operator line ----------------------------------

@pytest.mark.parametrize("operator, app", [
    ("OCTOBANK YANDEXPLUS OPLATA, UZ", "Яндекс"),      # owner example: Яндекс Плюс -> Яндекс
    ("YANDEX GO TASHKENT, UZ", "Яндекс"),
    ("PAYME OPLATA, UZ", "Payme"),                      # owner example: Payme -> Payme
    ("PAYME EDIN QR, UZ", "Payme"),
    ("OOO UZUM MARKET, TOSHKENT, SOME STREET 1", "Uzum"),  # owner example: Uzum Market -> Uzum
    ("OOO UZUM MARKET>TASH", "Uzum"),
    ("Uzum Bank", "UzumBank"),
    ("XAZNA PAYMENTS, UZ", "Xazna"),
    ("XAZNA PAYMENTS>TOSHK", "Xazna"),
    ("AO PAYNET MOBILE, UZ", "Paynet"),
    ("PAYNET P2P HUM2HUM>T", "Paynet"),
    ("BEEPUL OPLATA, UZ", "Beepul"),
    ("MAVRID UZCARD TO HUMO, UZ", "Mavrid"),
    ("PL HUMANS OPLATA>TOS", "Humans"),
    ("TENGE 24 WS P2P U2UE, UZ", "Tenge24"),
    ("DAVR MOBILE P2P U2H>", "Davr Mobile"),
    ("NBU P2P UZCARD UZCARD 2, UZ", "Milliy 2.0"),
    ("SQB JOYDA UZCARD P2P HUMO BOSHQA BANK, UZ", "Joyda"),
    ("HAMKOR HUMO P2P>ANDI", "Hamkor"),
])
def test_an_application_named_in_the_operator_is_explicit_evidence(db_session, operator, app):
    found = _detector(db_session).detect(operator)
    assert (found.application, found.source, found.reason) == (app, "rule", "brand_in_operator")
    assert found.confidence == 0.9
    # A brand rule knows the application, not whether the operation was a transfer.
    assert found.is_p2p is None


@pytest.mark.parametrize("operator", [
    "ANGLESEY FOOD, TOSHKENT, G TASHKENT SOME DISTRICT",   # a shop
    "MCHJ SOME PHARM, TASHKENT, UL. SOMEWHERE 11",         # a pharmacy
    "YATT SOMEBODY SOMEONE, TOSHKENT, STREET 4",           # an individual entrepreneur
    "FATH OIL PETROL, 206, SOME MFY",                      # a fuel station
    "UZCARD TO VISA, UZ",                                  # a card gateway that names no app
    "VIA PAYMENT, UZ",
    "MILLIY BANK PK KIRIM, UZ",                            # a bank credit, not the bank's app
    "CLICK P2P, UZ",                                       # evident, but the owner never named it
    "GOOGLE *SOME APP",
])
def test_a_merchant_or_an_unnamed_brand_stays_empty(db_session, operator):
    found = _detector(db_session).detect(operator)
    assert found.application is None and found.source is None and found.confidence is None
    assert found.reason == "no_evidence"


@pytest.mark.parametrize("operator", [
    "HAMKOR SAVDO MCHJ, TOSHKENT, SOME STREET",   # "hamkor" is also an everyday word
    "ASAKA TEKSTIL, ANDIJON",                     # and Asaka is a town
    "OQ SAROY MCHJ, TOSHKENT",                    # "oq" means white
    "NBU BOSH OFIS, UZ",
])
def test_an_everyday_word_that_is_also_a_brand_needs_a_payment_channel_beside_it(db_session, operator):
    assert _detector(db_session).detect(operator).application is None


@pytest.mark.parametrize("operator, candidates", [
    ("XAZNA PAYNET TOLOV>T", ("Xazna", "Paynet")),
    ("UZUMBAN PAYWAY UZCAR", ("UzumBank", "PayWay")),
])
def test_two_applications_in_one_operator_are_ambiguous_and_stay_empty(db_session, operator, candidates):
    found = _detector(db_session).detect(operator)
    assert found.application is None and found.reason == "ambiguous"
    assert set(found.candidates) == set(candidates)


def test_a_brand_and_a_dictionary_substring_that_disagree_are_ambiguous(db_session):
    detector = _detector(db_session, extra=[("STORE", "StoreApp")])
    found = detector.detect("PAYME STORE CITY")
    assert found.application is None and found.reason == "ambiguous"
    assert set(found.candidates) == {"Payme", "StoreApp"}


def test_the_owner_filing_a_line_outranks_the_words_in_it(db_session):
    # The owner files a Paynet-branded line under another application: theirs wins.
    detector = _detector(db_session, extra=[("PAYNET SPECIAL GATE, UZ", "Humans")])
    found = detector.detect("PAYNET SPECIAL GATE, 99")
    assert (found.application, found.source) == ("Humans", "dict")


@pytest.mark.parametrize("operator", ["XKKM 00014 ATM 000001, UZ", "HAMKORBANK BANKOMAT 12, UZ", "PAYNET ATM 7"])
def test_cash_from_a_machine_is_never_an_application(db_session, operator):
    found = _detector(db_session).detect(operator)
    assert found.application is None and found.reason == "cash_or_terminal"


@pytest.mark.parametrize("operator", [None, "", "  ", "Unknown", "UNKNOWN", "HUMOCARD", "BALANCE CHANGE"])
def test_a_receipt_without_an_operator_line_has_no_application(db_session, operator):
    found = _detector(db_session).detect(operator)
    assert found.application is None and found.reason == "no_operator_in_receipt"


def test_every_brand_rule_names_its_basis():
    assert all(rule.basis and rule.patterns and rule.application for rule in BRAND_RULES)
    assert brand_applications(operator_form("PAYME OPLATA, UZ")) == ["Payme"]
    # Inside a longer word nothing matches.
    assert brand_applications(operator_form("PAYMENTS SERVICE, UZ")) == []


def test_the_detector_works_without_a_dictionary():
    detector = ApplicationDetector(None)
    assert detector.detect("PAYME OPLATA, UZ").application == "Payme"
    assert detector.detect("SOME SHOP, CITY").application is None


# --- values people set: the last tier, and a veto --------------------------------

def test_two_hand_set_receipts_of_one_operator_make_a_pattern(db_session):
    detector = _detector(db_session, confirmed={operator_form("VIA PAYMENT, UZ").key: {"Via": 2}})
    found = detector.detect("VIA PAYMENT, 99")
    assert (found.application, found.source, found.confidence) == ("Via", "similar", 0.8)
    assert found.reason == "confirmed_similar"


def test_one_hand_set_receipt_is_not_a_pattern(db_session):
    detector = _detector(db_session, confirmed={"VIA PAYMENT": {"Via": 1}})
    assert detector.detect("VIA PAYMENT, UZ").application is None


def test_people_who_disagree_with_each_other_confirm_nothing(db_session):
    detector = _detector(db_session, confirmed={"VIA PAYMENT": {"Via": 3, "Other": 1}})
    found = detector.detect("VIA PAYMENT, UZ")
    assert found.application is None and found.reason == "ambiguous"
    assert found.candidates == ("Other", "Via")


def test_people_consistently_correcting_an_operator_silence_the_rule(db_session):
    detector = _detector(db_session, confirmed={"PAYME OPLATA": {"Shop": 2}})
    found = detector.detect("PAYME OPLATA, UZ")
    assert found.application is None and found.reason == "conflicts_with_confirmed"
    assert found.candidates == ("Shop", "Payme")


def _tx(n, operator, app=None, src=None):
    return Transaction(
        raw_message=f"m{n}", source_type="AUTO", source_chat_id=1, source_message_id=n,
        transaction_date=datetime(2026, 5, 10, 12), amount=Decimal("-1000"), transaction_type="DEBIT",
        operator_raw=operator, application_mapped=app, application_src=src,
    )


def test_only_hand_set_values_are_learned_from(db_session):
    db_session.add_all([
        _tx(1, "VIA PAYMENT, UZ", "Via", "manual"),
        _tx(2, "VIA PAYMENT, 99", "Via", "manual"),
        # The system's own output and values of unknown origin are not evidence.
        _tx(3, "GATE ONE, UZ", "Guess", "ai"),
        _tx(4, "GATE ONE, UZ", "Guess", "rule"),
        _tx(5, "GATE ONE, UZ", "Guess", "dict"),
        _tx(6, "GATE ONE, UZ", "Guess", "similar"),
        _tx(7, "GATE ONE, UZ", "Guess", None),
        _tx(8, "GATE TWO, UZ", None, "manual"),
    ])
    db_session.commit()
    assert load_confirmed_applications(db_session) == {"VIA PAYMENT": {"Via": 2}}


# --- provenance always describes the value beside it ------------------------------

def test_a_change_of_application_drops_the_old_stamp(db_session):
    tx = _tx(1, "PAYME OPLATA, UZ", "Payme", "rule")
    tx.application_confidence = Decimal("0.9")
    db_session.add(tx)
    db_session.commit()
    assert (tx.application_src, float(tx.application_confidence)) == ("rule", 0.9)

    # After a commit the attributes are expired; the change must still be seen.
    tx.application_mapped = "Something else"
    assert tx.application_src is None and tx.application_confidence is None
    db_session.commit()
    assert db_session.get(Transaction, tx.id).application_src is None


def test_writing_the_same_application_keeps_the_stamp(db_session):
    tx = _tx(1, "PAYME OPLATA, UZ", "Payme", "manual")
    db_session.add(tx)
    db_session.commit()
    tx.application_mapped = "Payme"
    assert tx.application_src == "manual"


def test_the_stamp_survives_any_keyword_order_at_creation(db_session):
    tx = Transaction(
        application_src="dict", application_confidence=Decimal("1"), application_mapped="Hamkor",
        raw_message="m", source_type="AUTO", source_chat_id=1, source_message_id=1,
        transaction_date=datetime(2026, 5, 10, 12), amount=Decimal("-1"), transaction_type="DEBIT",
    )
    assert tx.application_src == "dict"


def test_stamping_and_the_overwrite_guard(db_session):
    tx = _tx(1, "PAYME OPLATA, UZ")
    db_session.add(tx)
    db_session.commit()
    assert may_replace_application(tx)

    stamp_application(tx, "Payme", source="rule", confidence=0.9)
    assert (tx.application_mapped, tx.application_src, float(tx.application_confidence)) == ("Payme", "rule", 0.9)
    assert may_replace_application(tx)                     # derived: may be derived again

    tx.application_mapped = "Typed by automation that does not say"
    assert tx.application_src is None and not may_replace_application(tx)   # unknown origin: protected

    tx.application_mapped = None
    mark_manual_application(tx)
    assert tx.application_src == "manual" and not may_replace_application(tx)   # emptied by a person: theirs
