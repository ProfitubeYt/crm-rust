"""
Operator name to application mapping module
Uses operator_reference as single source of truth.
"""
import os
import re
import threading
import time
import unicodedata
from dataclasses import dataclass
from typing import Iterable, Optional, List, Tuple, Dict

from sqlalchemy.orm import Session

from database.models import OperatorReference


# Process-wide cache: TTL-aware, shared across requests to avoid SELECT * per receipt.
_CACHE_TTL_SECONDS = float(os.getenv("OPERATOR_MAPPER_CACHE_TTL", "120"))
_cache_lock = threading.Lock()
_cache_data: List[Tuple[int, str, str, bool]] = []
_cache_forms: Dict[int, "OperatorForm"] = {}
_cache_loaded_at: float = 0.0

# CardXabar closes the operator line with a country code: `…, UZ`, and on older
# receipts `…, 99`. The owner's dictionary was typed from the older ones.
_COUNTRY_SUFFIX = re.compile(r",\s*(?:[A-Za-z]{2}|\d{2,3})\s*$")
# HUMO cuts the operator line at 20 characters; a trailing space may be lost.
_CUT_LENGTHS = (19, 20)

#: How far each kind of dictionary match is trusted. Ordinal, not a probability.
MATCH_CONFIDENCE: Dict[str, float] = {
    "exact": 1.0,
    "variant": 0.95,
    "truncated": 0.9,
    "substring": 0.85,
}


@dataclass(frozen=True)
class OperatorForm:
    """An operator line reduced to the name it carries.

    The same gateway reaches us as `NBU 2P2 U PLAT UZCARD, 99`, as
    `NBU 2P2 U PLAT UZCARD, UZ` and, through HUMO, cut at 20 characters with a
    city after `>`. The country code, the city and the cut say nothing about
    which application it is.
    """

    #: `normalize_operator` of the whole line: the key every other module uses.
    full: str
    #: Tokens of the name alone, without the country code and the city.
    tokens: Tuple[str, ...]
    #: The name itself may have been cut at 20 characters.
    cut: bool

    @property
    def key(self) -> str:
        return " ".join(self.tokens)


def operator_form(value: Optional[str]) -> OperatorForm:
    text = unicodedata.normalize("NFKC", str(value or "")).strip()
    name = _COUNTRY_SUFFIX.sub("", text)
    had_country = name != text
    name, arrow, _city = name.partition(">")
    return OperatorForm(
        full=OperatorMapper.normalize_operator(text),
        tokens=tuple(OperatorMapper.normalize_operator(name).split()),
        cut=not arrow and not had_country and len(text) in _CUT_LENGTHS,
    )


def _is_cut_of(short: OperatorForm, long: OperatorForm) -> bool:
    """`short` is `long` cut off: same tokens, the last one broken or the rest missing."""
    if not short.cut or len(short.tokens) < 2 or len(short.key) < 12:
        return False
    count = len(short.tokens)
    if len(long.tokens) < count or short.tokens[:-1] != long.tokens[:count - 1]:
        return False
    last, whole = short.tokens[-1], long.tokens[count - 1]
    return whole.startswith(last) and (len(whole) > len(last) or len(long.tokens) > count)


def _load_cache(db_session: Session) -> Tuple[List[Tuple[int, str, str, bool]], Dict[int, OperatorForm]]:
    rows = (
        db_session.query(OperatorReference)
        .filter(OperatorReference.is_active == True)  # noqa: E712
        .all()
    )
    data: List[Tuple[int, str, str, bool]] = []
    forms: Dict[int, OperatorForm] = {}
    for row in rows:
        if not row.operator_name or not row.application_name:
            continue
        form = operator_form(row.operator_name)
        data.append((row.id, form.full, row.application_name, bool(row.is_p2p)))
        forms[row.id] = form
    return data, forms


def _get_cached(db_session: Session, force: bool = False) -> List[Tuple[int, str, str, bool]]:
    global _cache_data, _cache_forms, _cache_loaded_at
    now = time.time()
    with _cache_lock:
        if force or not _cache_data or (now - _cache_loaded_at) > _CACHE_TTL_SECONDS:
            try:
                _cache_data, _cache_forms = _load_cache(db_session)
                _cache_loaded_at = now
            except Exception:  # noqa: BLE001
                # On failure keep stale cache rather than blowing up the request.
                pass
        return _cache_data


def invalidate_operator_mapper_cache() -> None:
    """Force next caller to reload from DB (call this after operator CRUD)."""
    global _cache_data, _cache_forms, _cache_loaded_at
    with _cache_lock:
        _cache_data = []
        _cache_forms = {}
        _cache_loaded_at = 0.0


class OperatorMapper:
    """
    Maps raw operator strings to applications using operator_reference.

    Cache layout: (id, operator_name_normalized, application_name, is_p2p).
    Process-wide TTL cache (default 120s) — overridable via OPERATOR_MAPPER_CACHE_TTL env.
    """

    def __init__(self, db_session: Session):
        self.db_session = db_session
        # Lines shown to this mapper only (see `preview_rows`): id -> form.
        self._preview: List[Tuple[int, str, str, bool]] = []
        self._preview_forms: Dict[int, OperatorForm] = {}
        # Initial load (uses shared cache).
        self.mappings_cache: List[Tuple[int, str, str, bool]] = _get_cached(db_session)

    @staticmethod
    def normalize_operator(value: str) -> str:
        """
        Normalize operator text for matching:
        - uppercase
        - collapse whitespace
        - strip non-alphanumeric noise
        """
        if not value:
            return ""
        normalized = unicodedata.normalize("NFKC", str(value)).upper()
        normalized = "".join(
            char if char.isalnum() else " "
            for char in normalized
        )
        normalized = " ".join(normalized.split())
        return normalized

    @staticmethod
    def _contains_token_sequence(value: str, pattern: str) -> bool:
        """Match normalized phrases only at token boundaries."""
        return bool(re.search(rf"(?:^| ){re.escape(pattern)}(?: |$)", value))

    def refresh_cache(self) -> None:
        """Reload cache from operator_reference (only active rows). Force = bypass TTL."""
        self.mappings_cache = _get_cached(self.db_session, force=True) + self._preview

    def preview_rows(self, rows: Iterable[Tuple[str, str, bool]]) -> None:
        """Let this mapper alone see dictionary lines that are not stored yet.

        The dry run of an import: `(operator_name, application_name, is_p2p)`
        lines get negative ids, live in this instance and vanish with it. The
        shared cache, and so every other mapper, is not touched.
        """
        for operator_name, application_name, is_p2p in rows:
            form = operator_form(operator_name)
            if not form.full or not (application_name or "").strip():
                continue
            ref_id = -(len(self._preview) + 1)
            self._preview.append((ref_id, form.full, application_name.strip(), bool(is_p2p)))
            self._preview_forms[ref_id] = form
        stored = [item for item in self.mappings_cache if item[0] >= 0]
        self.mappings_cache = stored + self._preview

    def _form_of(self, ref_id: int, pattern: str) -> OperatorForm:
        form = self._preview_forms.get(ref_id) or _cache_forms.get(ref_id)
        if form is None or form.full != pattern:
            # A cache handed over by hand: only the normalized pattern is known.
            form = OperatorForm(full=pattern, tokens=tuple(pattern.split()), cut=False)
        return form

    def map_operator_details(self, operator_raw: str) -> Optional[Dict]:
        """
        Return mapping details or None.
        Strategy, most specific first:
        1. exact      the whole normalized line equals a dictionary line
        2. variant    the same name, differing only in country code or city
        3. truncated  the same name, one of the two cut at 20 characters
        4. substring  a dictionary line inside the receipt's; the longest wins

        Tiers 2 and 3 answer only when every dictionary line they reach names
        the same application; when the lines disagree the result carries no
        application and lists the `candidates`. `match_type` keeps its two
        historical values; `match_kind` and `confidence` tell the tiers apart.
        """
        if not operator_raw:
            return None

        normalized_input = self.normalize_operator(operator_raw)
        if not normalized_input:
            return None

        entry = Tuple[int, str, str, bool]
        exact_match: Optional[entry] = None
        best_substring: Optional[entry] = None
        best_len = -1
        variants: List[entry] = []
        truncated: List[entry] = []
        form = operator_form(operator_raw)

        for ref_id, pattern, app, is_p2p in self.mappings_cache:
            if not pattern:
                continue

            if normalized_input == pattern:
                exact_match = (ref_id, pattern, app, is_p2p)
                break

            known = self._form_of(ref_id, pattern)
            if form.tokens and form.tokens == known.tokens:
                variants.append((ref_id, pattern, app, is_p2p))
            elif _is_cut_of(form, known) or _is_cut_of(known, form):
                truncated.append((ref_id, pattern, app, is_p2p))

            if self._contains_token_sequence(normalized_input, pattern):
                plen = len(pattern)
                if plen > best_len:
                    best_len = plen
                    best_substring = (ref_id, pattern, app, is_p2p)

        chosen, kind = exact_match, "exact"
        for found, found_kind in ((variants, "variant"), (truncated, "truncated")):
            if chosen is not None or not found:
                continue
            applications = sorted({item[2] for item in found})
            if len(applications) > 1:
                # The same name under two applications: the dictionary itself
                # is undecided, and a looser tier must not decide for it.
                return {
                    "reference_id": None,
                    "matched_operator_name": None,
                    "application_name": None,
                    "is_p2p": None,
                    "match_type": "AMBIGUOUS",
                    "match_kind": "ambiguous",
                    "confidence": 0.0,
                    "candidates": applications,
                }
            # The lowest id is the oldest line: a stable choice between runs.
            chosen, kind = min(found, key=lambda item: item[0]), found_kind
        if chosen is None:
            chosen, kind = best_substring, "substring"
        if not chosen:
            return None

        ref_id, matched_operator_name, application_name, is_p2p = chosen

        return {
            "reference_id": ref_id,
            "matched_operator_name": matched_operator_name,
            "application_name": application_name,
            "is_p2p": is_p2p,
            "match_type": "EXACT" if kind == "exact" else "SUBSTRING",
            "match_kind": kind,
            "confidence": MATCH_CONFIDENCE[kind],
        }

    def map_operator(self, operator_raw: str) -> Optional[str]:
        """Backward compatible: return only application_name."""
        details = self.map_operator_details(operator_raw)
        return details["application_name"] if details else None

    def get_existing_applications(self) -> List[str]:
        """Distinct active application names."""
        apps = (
            self.db_session.query(OperatorReference.application_name)
            .filter(OperatorReference.is_active == True)  # noqa: E712
            .distinct()
            .all()
        )
        return [row[0] for row in apps]

    def get_candidate_examples(self, operator_raw: str, limit: int = 10) -> List[Dict]:
        """
        Return top similar reference rows for AI hinting.
        Simple similarity: substring + token overlap score.
        """
        normalized_input = self.normalize_operator(operator_raw)
        if not normalized_input:
            return []

        input_tokens = set(normalized_input.split())
        candidates: List[Tuple[float, Tuple[int, str, str, bool]]] = []

        for item in self.mappings_cache:
            ref_id, pattern, app, is_p2p = item
            if not pattern:
                continue

            score = 0.0
            if pattern == normalized_input:
                score += 100  # exact match bonus
            if (
                self._contains_token_sequence(normalized_input, pattern)
                or self._contains_token_sequence(pattern, normalized_input)
            ):
                score += len(pattern)

            pattern_tokens = set(pattern.split())
            overlap = len(input_tokens & pattern_tokens)
            score += overlap * 5

            if score > 0:
                candidates.append((score, item))

        candidates.sort(key=lambda t: (-t[0], -len(t[1][1]), t[1][0]))
        top = candidates[:limit]

        return [
            {
                "reference_id": ref_id,
                "matched_operator_name": pattern,
                "application_name": app,
                "is_p2p": is_p2p,
                "score": score,
            }
            for score, (ref_id, pattern, app, is_p2p) in top
        ]
