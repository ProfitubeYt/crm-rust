"""Which application a receipt belongs to, and on what evidence.

«Приложение» is the application or online service that the receipt's own
operator line identifies: the payment app whose gateway processed the
operation (`PAYME OPLATA` — Payme), or the online service that was paid
(`OCTOBANK YANDEXPLUS OPLATA` — Яндекс). It is a statement about that line. It
is not «Сервис», the bank that sent the notice, and it does not claim which app
was open on the cardholder's phone. A shop, a terminal, an ATM or a bank office
names no application, and the field stays empty for them: an empty cell is the
correct answer when the receipt does not say.

Evidence is taken in this order, and the first tier that answers wins:

1. the owner's dictionary knows this very operator line (exact, or the same
   name with another country code, city or 20-character cut);
2. the line names an application outright (`BRAND_RULES`);
3. a dictionary line stands inside the receipt's line (the historical rule);
4. people have already set the application of this operator by hand, at least
   twice and always the same way.

Two tiers that name different applications cancel out: the receipt is reported
as ambiguous with both candidates and nothing is written. The values people set
are never overruled; where they consistently disagree with tiers 1–3 the
detector stands down for that operator.
"""
from __future__ import annotations

import os
import re
import threading
import time
from collections import Counter
from dataclasses import dataclass, field
from typing import Dict, Iterable, List, Mapping, Optional, Sequence, Tuple

from sqlalchemy import inspect as sa_inspect
from sqlalchemy.orm import Session

from database.models import Transaction
from parsers.operator_mapper import MATCH_CONFIDENCE, OperatorForm, OperatorMapper, operator_form

#: Sources this module writes. A value carrying one of them was derived from the
#: operator line and may be derived again; any other value belongs to someone.
DETECTED_SOURCES = ("dict", "rule", "similar")

BRAND_CONFIDENCE = 0.9
SIMILAR_CONFIDENCE = 0.8

#: How many hand-set receipts of one operator make a pattern.
SIMILAR_MIN_SUPPORT = int(os.getenv("APPLICATION_SIMILAR_MIN_SUPPORT", "2"))


@dataclass(frozen=True)
class BrandRule:
    """An application named in the operator line itself.

    `patterns` are token sequences matched at token boundaries; a token ending
    in `*` also matches any longer token that starts with it. `needs_channel`
    marks a name that is an everyday word as well (`hamkor` is Uzbek for
    "partner"): it counts only beside a payment-channel word such as P2P.
    """

    application: str
    patterns: Tuple[str, ...]
    needs_channel: bool = False
    basis: str = ""


#: Every application here is either in the owner's dictionary or one of the
#: owner's own examples; a pattern is that application's name, or a stem that
#: several of the owner's dictionary lines share and no other application uses.
#: A brand that is evident but that the owner never named (Click, Alif, Plum…)
#: is deliberately absent: it is reported for review, not written.
BRAND_RULES: Tuple[BrandRule, ...] = (
    BrandRule("Payme", ("PAYME",), basis="owner example; dictionary name"),
    BrandRule("Яндекс", ("YANDEX*", "ЯНДЕКС*"), basis="owner example: Яндекс Плюс"),
    BrandRule("Uzum", ("UZUM MARKET",), basis="owner example: Uzum Market"),
    BrandRule("UzumBank", ("UZUM BANK", "UZUMB*"), basis="dictionary name; the bank's own receipts"),
    BrandRule("Xazna", ("XAZNA",), basis="dictionary name"),
    BrandRule("Paynet", ("PAYNET", "UZPAYNET"), basis="dictionary name"),
    BrandRule("Beepul", ("BEEPUL",), basis="dictionary name"),
    BrandRule("Humans", ("HUMANS",), basis="dictionary name"),
    BrandRule("Tenge24", ("TENGE24", "TENGE 24"), basis="dictionary name"),
    BrandRule("Davr Mobile", ("DAVR MOBILE",), basis="dictionary name"),
    BrandRule("Mavrid", ("MAVRID",), basis="dictionary name"),
    BrandRule("SmartBank", ("SMARTBANK",), basis="dictionary name"),
    BrandRule("Chakanapay", ("CHAKANAPAY",), basis="dictionary name"),
    BrandRule("Joyda", ("JOYDA",), basis="dictionary name"),
    BrandRule("PayWay", ("PAYWAY",), basis="dictionary name"),
    BrandRule("SQB", ("SQB MOBILE",), basis="dictionary stem: both SQB lines"),
    BrandRule("Milliy 2.0", ("MILLIY 2 0", "NBU P2P", "NBU 2P2"), basis="dictionary stem: all four NBU lines"),
    BrandRule("OQ", ("OQ P2P",), basis="dictionary stem: both OQ lines"),
    BrandRule("Hamkor", ("HAMKOR*",), needs_channel=True, basis="dictionary name; also an everyday word"),
    BrandRule("Asakabank", ("ASAKA*",), needs_channel=True, basis="dictionary name; also a place name"),
)

#: Words that show the line describes a payment channel rather than a shop.
_CHANNEL_WORDS = frozenset({
    "P2P", "2P2", "UZCARD", "UZKARD", "UZKART", "HUMO", "VISA", "OPLATA", "PEREVOD",
    "PAYMENTS", "PAYMENT", "TOLOV", "TULOV", "MOBILE", "KONVERS", "KONVERSIYA", "KONVERSIA",
})
#: `H2U`, `HUM2UZC`, `UZCARD2HU`, `HUMO2UZCARD` and their kin.
_CHANNEL_PAIR = re.compile(r"^(?:H|U|HUM|UZC|HUMO|UZCARD|UZKARD)(?:2|TO)(?:H|U|HUM|UZC|HUMO|UZCARD|UZKARD)\w*$")

#: Cash from a machine is not an application, whatever bank owns the machine.
_CASH_WORDS = frozenset({"ATM", "BANKOMAT", "BANKOMATI"})

#: What the parsers store when the receipt has no operator line at all.
_NO_OPERATOR = frozenset({"", "UNKNOWN", "HUMOCARD", "UZCARD", "BALANCE CHANGE"})


@dataclass(frozen=True)
class Detection:
    """What the evidence says about one operator line."""

    application: Optional[str] = None
    #: `dict`, `rule` or `similar` when an application was found.
    source: Optional[str] = None
    confidence: Optional[float] = None
    #: Why, in a fixed vocabulary that is safe to count and to publish.
    reason: str = "no_evidence"
    #: Applications the evidence pointed at when it could not choose.
    candidates: Tuple[str, ...] = ()
    #: From the dictionary line, when there is one; brand rules do not know it.
    is_p2p: Optional[bool] = None
    reference_id: Optional[int] = None

    @property
    def found(self) -> bool:
        return bool(self.application)


def _matches(tokens: Sequence[str], pattern: Sequence[str]) -> bool:
    size = len(pattern)
    for start in range(len(tokens) - size + 1):
        for offset, want in enumerate(pattern):
            have = tokens[start + offset]
            if want.endswith("*"):
                if not have.startswith(want[:-1]):
                    break
            elif have != want:
                break
        else:
            return True
    return False


def _has_channel_word(tokens: Sequence[str]) -> bool:
    return any(token in _CHANNEL_WORDS or _CHANNEL_PAIR.match(token) for token in tokens)


def brand_applications(form: OperatorForm) -> List[str]:
    """Applications the operator line names outright, in rule order, each once."""
    found: List[str] = []
    for rule in BRAND_RULES:
        if rule.application in found:
            continue
        if rule.needs_channel and not _has_channel_word(form.tokens):
            continue
        if any(_matches(form.tokens, tuple(pattern.split())) for pattern in rule.patterns):
            found.append(rule.application)
    return found


class ApplicationDetector:
    """Runs the evidence tiers over one operator line at a time."""

    def __init__(
        self,
        mapper: Optional[OperatorMapper],
        *,
        confirmed: Optional[Mapping[str, Mapping[str, int]]] = None,
    ):
        self.mapper = mapper
        #: operator key -> {application: how many receipts people set it on}
        self.confirmed = confirmed or {}

    def detect(self, operator_raw: Optional[str]) -> Detection:
        form = operator_form(operator_raw)
        if not form.tokens or form.key in _NO_OPERATOR:
            return Detection(reason="no_operator_in_receipt")

        found = self._from_operator(operator_raw or "", form)
        return self._against_confirmed(form, found)

    def _from_operator(self, operator_raw: str, form: OperatorForm) -> Detection:
        match = self.mapper.map_operator_details(operator_raw) if self.mapper else None
        kind = (match or {}).get("match_kind")
        if match and kind == "ambiguous":
            return Detection(reason="ambiguous", candidates=tuple(match.get("candidates") or ()))

        def from_dictionary() -> Detection:
            return Detection(
                application=match["application_name"],
                source="dict",
                confidence=match.get("confidence", MATCH_CONFIDENCE.get(kind, MATCH_CONFIDENCE["substring"])),
                reason=f"dictionary_{kind}",
                is_p2p=match.get("is_p2p"),
                reference_id=match.get("reference_id"),
            )

        if match and match.get("application_name") and kind in ("exact", "variant", "truncated"):
            # The owner filed this very line: that outranks any reading of its words.
            return from_dictionary()

        if _CASH_WORDS.intersection(form.tokens):
            return Detection(reason="cash_or_terminal")

        brands = brand_applications(form)
        substring = match["application_name"] if match and match.get("application_name") else None
        if len(brands) > 1:
            return Detection(reason="ambiguous", candidates=tuple(brands))
        if brands and substring and substring != brands[0]:
            return Detection(reason="ambiguous", candidates=(brands[0], substring))
        if substring:
            return from_dictionary()
        if brands:
            return Detection(application=brands[0], source="rule", confidence=BRAND_CONFIDENCE, reason="brand_in_operator")
        return Detection(reason="no_evidence")

    def _against_confirmed(self, form: OperatorForm, found: Detection) -> Detection:
        """Hand-set receipts of the same operator: the last tier, and a veto."""
        votes = self.confirmed.get(form.key)
        if not votes:
            return found
        total = sum(votes.values())
        application, support = max(votes.items(), key=lambda item: (item[1], item[0]))
        unanimous = support == total and total >= SIMILAR_MIN_SUPPORT
        if found.found:
            if unanimous and application != found.application:
                # People keep correcting this operator to something else.
                return Detection(reason="conflicts_with_confirmed", candidates=(application, found.application))
            return found
        if unanimous:
            return Detection(application=application, source="similar", confidence=SIMILAR_CONFIDENCE, reason="confirmed_similar")
        if len(votes) > 1:
            return Detection(reason="ambiguous", candidates=tuple(sorted(votes)))
        return found


# --- What people confirmed -----------------------------------------------------

_CONFIRMED_TTL_SECONDS = float(os.getenv("APPLICATION_CONFIRMED_CACHE_TTL", "120"))
_confirmed_lock = threading.Lock()
_confirmed_cache: Dict[str, object] = {"data": None, "expires_at": 0.0}


def has_provenance(db: Session) -> bool:
    """Whether this database already has the provenance columns (revision 0033)."""
    try:
        columns = {item["name"] for item in sa_inspect(db.get_bind()).get_columns(Transaction.__tablename__)}
    except Exception:  # noqa: BLE001
        return False
    return "application_src" in columns


def load_confirmed_applications(db: Session) -> Dict[str, Dict[str, int]]:
    """`{operator key: {application: receipts}}` from values people set by hand.

    Only `manual` counts. What the dictionary, a rule, this tier or the AI wrote
    is the system's own output; feeding it back would let one wrong match
    confirm itself. Values of unknown origin are left out for the same reason.
    """
    if not has_provenance(db):
        return {}
    votes: Dict[str, Counter] = {}
    rows = (
        db.query(Transaction.operator_raw, Transaction.application_mapped)
        .filter(Transaction.application_src == "manual")
        .filter(Transaction.application_mapped.isnot(None))
        .filter(Transaction.operator_raw.isnot(None))
    )
    for operator_raw, application in rows.yield_per(1000):
        application = (application or "").strip()
        key = operator_form(operator_raw).key
        if key and application:
            votes.setdefault(key, Counter())[application] += 1
    return {key: dict(counter) for key, counter in votes.items()}


def confirmed_applications(db: Session, *, force: bool = False) -> Dict[str, Dict[str, int]]:
    """`load_confirmed_applications` behind a short cache, for the ingestion path."""
    now = time.monotonic()
    with _confirmed_lock:
        if not force and _confirmed_cache["data"] is not None and now < float(_confirmed_cache["expires_at"]):
            return _confirmed_cache["data"]  # type: ignore[return-value]
    try:
        data = load_confirmed_applications(db)
    except Exception:  # noqa: BLE001
        # Ingestion must not fail because the hand-set values could not be read.
        data = {}
    with _confirmed_lock:
        _confirmed_cache["data"] = data
        _confirmed_cache["expires_at"] = time.monotonic() + _CONFIRMED_TTL_SECONDS
    return data


def invalidate_confirmed_cache() -> None:
    with _confirmed_lock:
        _confirmed_cache["data"] = None
        _confirmed_cache["expires_at"] = 0.0


def detector_for(db: Session, *, mapper: Optional[OperatorMapper] = None, fresh: bool = False) -> ApplicationDetector:
    """The detector the ingestion path and the historical pass both use."""
    mapper = mapper or OperatorMapper(db)
    if fresh:
        mapper.refresh_cache()
    return ApplicationDetector(mapper, confirmed=confirmed_applications(db, force=fresh))


def confirmed_vocabulary(mapper: Optional[OperatorMapper]) -> Dict[str, str]:
    """Application names the owner has confirmed: `{casefolded: as written}`.

    The active dictionary plus the brand rules. Anything else, however
    plausible, is a suggestion for the owner and not a value for a receipt.
    """
    names = [rule.application for rule in BRAND_RULES]
    if mapper is not None:
        try:
            names.extend(name for name in mapper.get_existing_applications() if name)
        except Exception:  # noqa: BLE001
            pass
    vocabulary: Dict[str, str] = {}
    for name in names:
        vocabulary.setdefault(name.strip().casefold(), name.strip())
    return vocabulary


#: Legal forms of a one-person business: the rest of such a line is a name.
_PERSON_MARKERS = frozenset({"YATT", "YTT", "IP", "CHP", "ИП", "ЧП", "ЯТТ"})


def ai_may_see(operator_raw: Optional[str]) -> bool:
    """Whether an operator line may be shown to an external model at all.

    A gateway descriptor is a handful of words. A merchant line carries an
    address and often a person's name; it names no application anyway, so
    nothing is lost by keeping it at home.
    """
    text = str(operator_raw or "")
    form = operator_form(text)
    return (
        bool(form.tokens)
        and text.count(",") <= 1
        and len(form.tokens) <= 6
        and not _PERSON_MARKERS.intersection(form.tokens)
    )


def may_replace_application(transaction: Transaction) -> bool:
    """Whether automation may write this receipt's application.

    Yes for an empty cell nobody emptied on purpose, and for a value this
    module (or the AI) derived and can derive again. No for anything a person
    set, and no for a value whose origin is unknown: it may be theirs.
    """
    source = getattr(transaction, "application_src", None)
    if source == "manual":
        return False
    if not (transaction.application_mapped or "").strip():
        return True
    return source in DETECTED_SOURCES + ("ai",)


def stamp_application(
    transaction: Transaction,
    application: Optional[str],
    *,
    source: Optional[str],
    confidence: Optional[float] = None,
) -> None:
    """Write an application together with where it came from.

    The value goes first: changing it clears the old stamp (see the listener on
    the model), and the new one must land after that.
    """
    transaction.application_mapped = application or None
    transaction.application_src = source if application else None
    transaction.application_confidence = confidence if application and source in DETECTED_SOURCES + ("ai",) else None


def mark_manual_application(transaction: Transaction) -> None:
    """Call after a person changed the application: the value is now theirs.

    An application a person emptied stays theirs too: "there is none" is a
    decision, and the next historical pass must not fill the cell back in.
    """
    transaction.application_src = "manual"
    transaction.application_confidence = None
