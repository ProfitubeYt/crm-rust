"""Transactional Excel edits; transport and grant authentication live in the API.

``apply_operation(db, grant, operation, current_user, authorize)`` never commits.
The caller owns the outer transaction and validates/locks the grant first.
``authorize(tx, *, candidate_date=None, creating=False)`` must raise PermissionError
or HTTPException(401/403/404) when the current actor cannot edit the record. For a
create it receives an unsaved MANUAL Transaction. It is called before returning
any existing field values, and again for a proposed date change.

Operations use ``kind``, a UUID ``operation_id``, and, for existing rows, string
``id`` and ``uuid``. Updates carry ``fields`` and ``base`` for every touched
field. Deletes carry the complete ``base`` or the snapshot's ``version``.
Omitted fields are untouched; explicit null clears a nullable field.
"""
from __future__ import annotations

import copy
import hashlib
import json
import re
from datetime import date, datetime
from decimal import Decimal, InvalidOperation, localcontext
from typing import Any, Callable, Dict
from uuid import UUID, uuid4
from zoneinfo import ZoneInfo

from sqlalchemy.exc import IntegrityError
from sqlalchemy.orm import Session

from database.excel_sync_models import ExcelSyncGrant, ExcelSyncOperation
from database.models import AccessAuditLog, SyncDeletion, Transaction, TransactionAnnotationEvent
from services import annotation_service
from services.application_detection import mark_manual_application
from services.fingerprint import compute_fingerprint_candidates


LEGACY_EDITABLE_FIELDS = (
    "transaction_date", "amount", "transaction_type", "currency", "operator_raw",
    "application_mapped", "balance_after", "card_last_4", "receiver_name",
    "receiver_card", "is_p2p", "raw_message", "annotation_details",
    "annotation_person_code", "annotation_period", "annotation_article_code",
)
NULLABLE_EXTENSION_FIELDS = ("excel_bank", "p2p_confirmed")
EDITABLE_FIELDS = LEGACY_EDITABLE_FIELDS + NULLABLE_EXTENSION_FIELDS
LEGACY_EDITABLE_FIELD_SET = frozenset(LEGACY_EDITABLE_FIELDS)
EDITABLE_FIELD_SET = frozenset(EDITABLE_FIELDS)
CREATE_REQUIRED_FIELDS = frozenset({
    "transaction_date", "amount", "transaction_type", "currency", "operator_raw", "card_last_4",
})
TRANSACTION_TYPES = frozenset({"DEBIT", "CREDIT", "CONVERSION", "REVERSAL"})
CURRENCIES = frozenset({"UZS", "USD", "EUR", "RUB", "KZT", "GBP", "CNY"})
EXCEL_BANKS = frozenset({"CardXabar", "HUMO Card"})
TEXT_LIMITS = {
    "operator_raw": 255, "application_mapped": 100, "receiver_name": 255,
    "raw_message": 200_000, "annotation_details": 500,
    "annotation_person_code": 32, "annotation_article_code": 16,
}
LOCAL_TIMEZONE = ZoneInfo("Asia/Tashkent")
MAX_MONEY = Decimal("9999999999999999.99")


class ExcelSyncValidationError(ValueError):
    """A stable error code safe to return to the workbook."""


def token_hash(token: str) -> str:
    """Store only the hash of the independently generated bearer capability."""
    return hashlib.sha256(token.encode("utf-8")).hexdigest()


def _json_value(value: Any) -> Any:
    if isinstance(value, Decimal):
        return format(value, ".2f") if value else "0.00"
    if isinstance(value, (datetime, date)):
        return value.isoformat()
    if isinstance(value, UUID):
        return str(value)
    return value


def _hash_json(value: Any) -> str:
    try:
        encoded = json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":"),
                             allow_nan=False, default=_json_value)
    except (TypeError, ValueError, OverflowError):
        raise ExcelSyncValidationError("invalid_json") from None
    return hashlib.sha256(encoded.encode("utf-8")).hexdigest()


def serialize_record(tx: Transaction) -> Dict[str, Any]:
    """Canonical editable values, a content version, and read-only metadata.

    Decimal and source identifiers are strings to avoid Excel/JavaScript numeric
    precision loss. Transaction timestamps are local Asia/Tashkent wall time.
    The version hashes editable content, not a modification timestamp. Added
    nullable fields do not change legacy versions until they carry a value;
    queued deletes from the previous protocol therefore remain valid when the
    receipt itself has not changed.
    """
    fields = {field: _json_value(getattr(tx, field, None)) for field in EDITABLE_FIELDS}
    fields["is_p2p"] = bool(fields["is_p2p"])
    version_fields = {key: value for key, value in fields.items()
                      if key not in NULLABLE_EXTENSION_FIELDS or value is not None}
    return {
        "id": str(tx.id),
        "uuid": str(tx.uuid),
        "version": _hash_json(version_fields),
        "fields": fields,
        "metadata": {
            "source_type": tx.source_type,
            "source_chat_id": str(tx.source_chat_id) if tx.source_chat_id is not None else None,
            "source_message_id": str(tx.source_message_id) if tx.source_message_id is not None else None,
            "source_device_id": tx.source_device_id,
            "source_device_sms_id": tx.source_device_sms_id,
            "parsing_method": tx.parsing_method,
            "parsing_confidence": tx.parsing_confidence,
            "is_gpt_parsed": bool(tx.is_gpt_parsed),
            "created_at": _json_value(tx.created_at),
            "updated_at": _json_value(tx.updated_at),
            "annotation_updated_at": _json_value(tx.annotation_updated_at),
            "annotation_updated_by": str(tx.annotation_updated_by) if tx.annotation_updated_by is not None else None,
            **{name: getattr(tx, name, None) for name in annotation_service.SRC_COLUMNS},
        },
    }


def _money(value: Any, field: str, *, nullable: bool = False) -> Decimal | None:
    if value is None and nullable:
        return None
    if value is None or isinstance(value, bool):
        raise ExcelSyncValidationError(f"invalid_{field}")
    try:
        number = Decimal(str(value))
        if not number.is_finite() or abs(number) > MAX_MONEY:
            raise ExcelSyncValidationError(f"invalid_{field}")
        with localcontext() as context:
            context.prec = 40
            rounded = number.quantize(Decimal("0.01"))
        if rounded != number:
            raise ExcelSyncValidationError(f"{field}_precision")
        return rounded if rounded else Decimal("0.00")
    except (InvalidOperation, ValueError, OverflowError):
        raise ExcelSyncValidationError(f"invalid_{field}") from None


def _local_datetime(value: Any) -> datetime:
    if isinstance(value, datetime):
        result = value
    elif isinstance(value, str) and value.strip():
        try:
            result = datetime.fromisoformat(value.strip().replace("Z", "+00:00"))
        except ValueError:
            raise ExcelSyncValidationError("invalid_transaction_date") from None
    else:
        raise ExcelSyncValidationError("invalid_transaction_date")
    if result.tzinfo is not None:
        result = result.astimezone(LOCAL_TIMEZONE).replace(tzinfo=None)
    return result


def _check_fields(value: Any, name: str) -> dict:
    if not isinstance(value, dict):
        raise ExcelSyncValidationError(f"invalid_{name}")
    if set(value) - EDITABLE_FIELD_SET:
        raise ExcelSyncValidationError(f"unknown_{name}_field")
    return value


def _normalize_fields(db: Session, fields: dict, *, tx: Transaction | None = None) -> dict:
    """Validate the complete patch before assigning any ORM attribute."""
    _check_fields(fields, "fields")
    normalized: dict = {}
    for key, value in fields.items():
        if key == "transaction_date":
            normalized[key] = _local_datetime(value)
        elif key in {"amount", "balance_after"}:
            normalized[key] = _money(value, key, nullable=key == "balance_after")
        elif key in {"transaction_type", "currency"}:
            allowed = TRANSACTION_TYPES if key == "transaction_type" else CURRENCIES
            text = str(value or "").strip().upper()
            if text not in allowed:
                raise ExcelSyncValidationError(f"invalid_{key}")
            normalized[key] = text
        elif key in {"is_p2p", "p2p_confirmed"}:
            if not isinstance(value, bool) and not (key == "p2p_confirmed" and value is None):
                raise ExcelSyncValidationError(f"invalid_{key}")
            normalized[key] = value
        elif key == "excel_bank":
            if value is not None and (not isinstance(value, str) or value not in EXCEL_BANKS):
                raise ExcelSyncValidationError("invalid_excel_bank")
            normalized[key] = value
        elif key in {"card_last_4", "receiver_card"}:
            if value in (None, ""):
                normalized[key] = None
            else:
                pattern = r"\d{4}" if key == "card_last_4" else r"\d{4,32}"
                if not isinstance(value, str) or re.fullmatch(pattern, value, flags=re.ASCII) is None:
                    raise ExcelSyncValidationError(f"invalid_{key}")
                normalized[key] = value
        elif key == "annotation_period":
            try:
                normalized[key] = annotation_service.coerce_period(value)
            except (ValueError, TypeError):
                raise ExcelSyncValidationError("invalid_annotation_period") from None
        else:
            if value is None:
                if key == "raw_message":
                    raise ExcelSyncValidationError("invalid_raw_message")
                normalized[key] = None
                continue
            if not isinstance(value, str) or "\x00" in value or len(value) > TEXT_LIMITS[key]:
                raise ExcelSyncValidationError(f"invalid_{key}")
            normalized[key] = value if key == "raw_message" else value.strip() or None

    annotation_patch = {key: value for key, value in normalized.items() if key in annotation_service.VALUE_COLUMNS}
    if annotation_patch:
        try:
            # Notes, periods and explicit clears do not reference dictionaries.
            # Nonempty codes still validate against the current database.
            needs_reference_codes = any(annotation_patch.get(key) not in (None, "") for key in
                                        ("annotation_person_code", "annotation_article_code"))
            validated = annotation_service.normalize_patch(
                annotation_patch, reference_codes=(
                    annotation_service.load_reference_codes(db) if needs_reference_codes else None
                ),
            )
        except (ValueError, TypeError):
            raise ExcelSyncValidationError("invalid_annotation_reference") from None
        for short, value in validated.items():
            normalized[annotation_service.ANNOTATION_FIELDS[short][0]] = value

    tx_type = normalized.get("transaction_type") or (tx.transaction_type if tx is not None else None)
    if "amount" in normalized:
        if tx_type not in TRANSACTION_TYPES:
            raise ExcelSyncValidationError("invalid_transaction_type")
        normalized["amount"] = -abs(normalized["amount"]) if tx_type == "DEBIT" else abs(normalized["amount"])
    return normalized


def _normalize_base(base: dict) -> dict:
    """Base values are canonical snapshots; never reinterpret their amount sign."""
    _check_fields(base, "base")
    result = dict(base)
    for key, value in result.items():
        if key in {"amount", "balance_after"}:
            result[key] = _json_value(_money(value, key, nullable=key == "balance_after"))
        elif key == "transaction_date":
            result[key] = _json_value(_local_datetime(value))
        elif key == "annotation_period" and value is not None:
            try:
                result[key] = _json_value(annotation_service.coerce_period(value))
            except (ValueError, TypeError):
                raise ExcelSyncValidationError("invalid_base_period") from None
        elif key in {"is_p2p", "p2p_confirmed"}:
            if not isinstance(value, bool) and not (key == "p2p_confirmed" and value is None):
                raise ExcelSyncValidationError(f"invalid_base_{key}")
        elif key == "excel_bank":
            if value is not None and (not isinstance(value, str) or value not in EXCEL_BANKS):
                raise ExcelSyncValidationError("invalid_base_excel_bank")
        elif value is not None and not isinstance(value, str):
            raise ExcelSyncValidationError(f"invalid_base_{key}")
    return result


def _actor_id(current_user: dict) -> int:
    try:
        return int(current_user.get("user_id") or current_user.get("id"))
    except (ValueError, TypeError, AttributeError):
        raise PermissionError("missing_actor") from None


def _audit(db: Session, grant: ExcelSyncGrant, operation: dict, actor_id: int,
           *, before: dict | None, after: dict | None, changed: list[str]) -> None:
    record = after or before or {}
    db.add(AccessAuditLog(
        action=f"excel_sync_{operation['kind']}", success=True, user_id=actor_id,
        details_json=json.dumps({
            "grant_id": str(grant.id), "workbook_id": grant.workbook_id,
            "operation_id": str(operation["operation_id"]),
            "transaction_id": record.get("id"), "transaction_uuid": record.get("uuid"),
            "fields": changed,
            "before": {key: before["fields"].get(key) for key in changed} if before else None,
            "after": {key: after["fields"].get(key) for key in changed} if after else None,
        }, ensure_ascii=False),
    ))


def _write_annotations(db: Session, tx: Transaction, patch: dict, actor_id: int) -> bool:
    """Human clears and confirmations retain manual ownership, including null."""
    if not patch:
        return False
    previous = {
        key: (getattr(tx, key, None), getattr(tx, src, None))
        for _short, (key, src) in annotation_service.ANNOTATION_FIELDS.items() if key in patch
    }
    result = annotation_service.apply_annotation_patch(
        db, tx, patch, source="manual", actor_user_id=actor_id, validate=False,
    )
    ownership_changed = False
    for short, (key, src) in annotation_service.ANNOTATION_FIELDS.items():
        if key not in patch:
            continue
        old_value, old_source = previous[key]
        if old_value == patch[key] and old_source != "manual":
            db.add(TransactionAnnotationEvent(
                transaction_id=tx.id, field=short,
                before_value=_json_value(old_value), after_value=_json_value(patch[key]),
                before_src=old_source, source="manual", actor_user_id=actor_id,
            ))
            ownership_changed = True
        setattr(tx, src, "manual")
    changed = bool(result["changed"] or ownership_changed)
    if changed:
        tx.annotation_updated_at = datetime.utcnow()
        tx.annotation_updated_by = actor_id
    return changed


def _refresh_fingerprint(tx: Transaction) -> None:
    tx.fingerprint = compute_fingerprint_candidates(
        amount=tx.amount, transaction_date=tx.transaction_date, card_last4=tx.card_last_4,
        operator_raw=tx.operator_raw, transaction_type=tx.transaction_type,
    )[0]


def _run_operation(db: Session, grant: ExcelSyncGrant, operation: dict,
                   actor_id: int, authorize: Callable[..., None]) -> dict:
    kind = operation.get("kind")
    if kind not in {"create", "update", "delete"}:
        raise ExcelSyncValidationError("invalid_kind")
    if kind == "create":
        fields = _check_fields(operation.get("fields"), "fields")
        if CREATE_REQUIRED_FIELDS - set(fields):
            raise ExcelSyncValidationError("missing_create_fields")
        if operation.get("id") is not None or operation.get("uuid") is not None:
            raise ExcelSyncValidationError("create_identity_is_server_assigned")
        normalized = _normalize_fields(db, fields)
        if not normalized.get("operator_raw") or not normalized.get("card_last_4"):
            raise ExcelSyncValidationError("missing_create_fields")
        tx = Transaction(
            uuid=uuid4(), source_type="MANUAL", source_chat_id=0, source_message_id=None,
            raw_message="", is_p2p=False, is_gpt_parsed=False,
        )
        for key, value in normalized.items():
            if key not in annotation_service.VALUE_COLUMNS:
                setattr(tx, key, value)
        if "application_mapped" in normalized:
            mark_manual_application(tx)
        authorize(tx, candidate_date=tx.transaction_date, creating=True)
        _refresh_fingerprint(tx)
        db.add(tx)
        db.flush()  # Annotation events require the new immutable transaction id.
        _write_annotations(db, tx, {key: value for key, value in normalized.items()
                                    if key in annotation_service.VALUE_COLUMNS}, actor_id)
        db.flush()
        record = serialize_record(tx)
        _audit(db, grant, operation, actor_id, before=None, after=record, changed=list(EDITABLE_FIELDS))
        return {"status": "applied", "record": record}

    try:
        raw_id = operation.get("id")
        if isinstance(raw_id, bool) or not re.fullmatch(r"[1-9]\d*", str(raw_id), flags=re.ASCII):
            raise ValueError
        tx_id = int(raw_id)
        if tx_id > 9223372036854775807:
            raise ValueError
        tx_uuid = UUID(str(operation.get("uuid")))
    except (ValueError, TypeError, AttributeError):
        raise ExcelSyncValidationError("invalid_transaction_identity") from None
    # populate_existing is essential for several requests using a long-lived
    # Session: the identity map must not hide a concurrent committed edit.
    tx = (db.query(Transaction).filter(Transaction.id == tx_id)
          .with_for_update().populate_existing().first())
    if tx is None:
        return {"status": "not_found"}
    authorize(tx, candidate_date=None, creating=False)
    if tx.uuid != tx_uuid:
        raise ExcelSyncValidationError("transaction_uuid_mismatch")
    before = serialize_record(tx)

    if kind == "delete":
        if operation.get("fields"):
            raise ExcelSyncValidationError("delete_cannot_change_fields")
        version = operation.get("version")
        if version is not None:
            if not isinstance(version, str) or not re.fullmatch(r"[0-9a-f]{64}", version):
                raise ExcelSyncValidationError("invalid_version")
            conflicts = {} if version == before["version"] else {
                "version": {"base": version, "current": before["version"], "proposed": None},
            }
        else:
            base = _normalize_base(_check_fields(operation.get("base"), "base"))
            if not LEGACY_EDITABLE_FIELD_SET.issubset(base):
                raise ExcelSyncValidationError("delete_requires_full_base_or_version")
            # Older clients did not know nullable extension fields. An omitted
            # extension means its old NULL value, never permission to ignore a
            # bank assignment or confirmation made since that snapshot.
            conflicts = {key: {"base": base.get(key), "current": value, "proposed": None}
                         for key, value in before["fields"].items() if base.get(key) != value}
        if conflicts:
            return {"status": "conflict", "conflicts": conflicts, "record": before}
        _audit(db, grant, operation, actor_id, before=before, after=None, changed=list(EDITABLE_FIELDS))
        # Existing sync listeners collect only at before_commit. This operation
        # flushes inside a savepoint, so persist the tombstone here exactly once.
        db.add(SyncDeletion(table_name="transactions", record_id=tx.id))
        db.delete(tx)
        db.flush()
        return {"status": "deleted", "id": before["id"], "uuid": before["uuid"]}

    fields = _check_fields(operation.get("fields"), "fields")
    if not fields:
        raise ExcelSyncValidationError("empty_fields")
    base = _normalize_base(_check_fields(operation.get("base"), "base"))
    if set(fields) - set(base):
        raise ExcelSyncValidationError("missing_base_field")
    normalized = _normalize_fields(db, fields, tx=tx)
    if "transaction_date" in normalized:
        authorize(tx, candidate_date=normalized["transaction_date"], creating=False)
    desired = {key: _json_value(value) for key, value in normalized.items()}
    conflicts = {
        key: {"base": base[key], "current": before["fields"][key], "proposed": value}
        for key, value in desired.items()
        if value != before["fields"][key] and base[key] != before["fields"][key]
    }
    if conflicts:
        return {"status": "conflict", "conflicts": conflicts, "record": before}

    changed = [key for key, value in desired.items() if value != before["fields"][key]]
    for key, value in normalized.items():
        if key not in annotation_service.VALUE_COLUMNS:
            setattr(tx, key, value)
    if "application_mapped" in changed:
        # Typed in the workbook by a person: theirs from now on, emptied or not.
        mark_manual_application(tx)
    # Changing direction also changes the sign while retaining the CURRENT
    # magnitude, including a concurrently edited magnitude not in this patch.
    if "transaction_type" in normalized and "amount" not in normalized:
        tx.amount = -abs(tx.amount) if tx.transaction_type == "DEBIT" else abs(tx.amount)
        if _json_value(tx.amount) != before["fields"]["amount"]:
            changed.append("amount")
    annotation_patch = {key: value for key, value in normalized.items()
                        if key in annotation_service.VALUE_COLUMNS}
    annotation_changed = _write_annotations(db, tx, annotation_patch, actor_id)
    if not changed and not annotation_changed:
        return {"status": "noop", "record": before}
    tx.updated_at = datetime.utcnow()
    _refresh_fingerprint(tx)
    db.flush()
    record = serialize_record(tx)
    _audit(db, grant, operation, actor_id, before=before, after=record,
           changed=sorted(set(changed) | set(annotation_patch)))
    return {"status": "applied", "record": record}


def _ensure_sqlite_outer_transaction(db: Session) -> None:
    """SQLite's legacy driver otherwise commits the first released savepoint.

    Explicit BEGIN preserves the caller's rollback contract without changing
    the engine settings or interfering with an already-open transaction.
    """
    if db.get_bind().dialect.name == "sqlite":
        connection = db.connection()
        raw = connection.connection.driver_connection
        if not raw.in_transaction:
            connection.exec_driver_sql("BEGIN")


def _replay_response(db: Session, receipt: ExcelSyncOperation,
                     authorize: Callable[..., None]) -> dict:
    """Exactly-once execution does not confer permanent access to old snapshots.

    Period locks and scope membership can change without rotating the grant.
    Recheck the current row before disclosing the stored response, but never
    overwrite the original operation receipt or reexecute its mutation.
    Masked responses retain the original outcome without row values so a client
    cannot mistake a previously applied create for a rejected, retryable draft.
    """
    result = copy.deepcopy(receipt.response)
    record = result.get("record")
    if not isinstance(record, dict):
        return result
    operation_id = str(receipt.operation_id)
    masked = {
        "operation_id": operation_id, "replayed": True,
        "original_status": result["status"],
        "applied": result["status"] in {"applied", "noop", "deleted"},
    }
    tx = (db.query(Transaction).filter(Transaction.id == int(record["id"]))
          .with_for_update().populate_existing().first())
    if tx is None or str(tx.uuid) != record.get("uuid"):
        return {"status": "not_found", **masked}
    try:
        authorize(tx, candidate_date=None, creating=False)
    except Exception as exc:
        if isinstance(exc, PermissionError) or getattr(exc, "status_code", None) in {401, 403, 404}:
            return {"status": "forbidden", **masked}
        raise
    return result


def apply_operation(db: Session, grant: ExcelSyncGrant, operation: dict,
                    current_user: dict, authorize: Callable[..., None]) -> dict:
    """Apply one savepoint-isolated operation and store its reply atomically.

    A unique (user, workbook, operation_id) reservation serializes duplicate
    deliveries even after the grant is replaced. The original grant remains
    attached to the receipt for attribution. Keep receipts across revocation
    and expiry; credentials are revoked, not deleted with their operation log.
    Success, conflict and validation replies are durable only if the caller
    commits; rolling back the request rolls back both edits and reservations.
    """
    operation_id = None
    try:
        if not isinstance(operation, dict):
            raise ExcelSyncValidationError("invalid_operation")
        operation_id = UUID(str(operation.get("operation_id")))
        actor_id = _actor_id(current_user)
        if actor_id != grant.user_id:
            return {"status": "forbidden", "operation_id": str(operation_id)}
        try:
            workbook_id = str(UUID(str(grant.workbook_id)))
        except (TypeError, ValueError, AttributeError):
            return {"status": "invalid", "operation_id": str(operation_id), "error": "workbook_not_bound"}
        request_hash = _hash_json(operation)
    except (ExcelSyncValidationError, ValueError, TypeError, AttributeError):
        return {"status": "invalid", "operation_id": str(operation_id) if operation_id else None,
                "error": "invalid_operation"}
    except PermissionError:
        return {"status": "forbidden", "operation_id": str(operation_id) if operation_id else None}

    _ensure_sqlite_outer_transaction(db)
    query = db.query(ExcelSyncOperation).filter(
        ExcelSyncOperation.user_id == actor_id, ExcelSyncOperation.workbook_id == workbook_id,
        ExcelSyncOperation.operation_id == operation_id,
    )
    previous = query.with_for_update().populate_existing().first()
    if previous is None:
        try:
            with db.begin_nested():
                previous = ExcelSyncOperation(grant_id=grant.id, user_id=actor_id, workbook_id=workbook_id,
                                              operation_id=operation_id,
                                              request_hash=request_hash, response={})
                db.add(previous)
                db.flush()
        except IntegrityError:
            # Another request won the unique reservation while this one waited.
            previous = query.with_for_update().populate_existing().one()
    if previous.request_hash != request_hash:
        return {"status": "operation_id_reused", "operation_id": str(operation_id)}
    if previous.response:
        return _replay_response(db, previous, authorize)

    try:
        with db.begin_nested():
            result = _run_operation(db, grant, operation, actor_id, authorize)
            db.flush()
    except ExcelSyncValidationError as exc:
        result = {"status": "invalid", "error": str(exc)}
    except IntegrityError:
        result = {"status": "invalid", "error": "constraint_violation"}
    except Exception as exc:
        if isinstance(exc, PermissionError) or getattr(exc, "status_code", None) in {401, 403, 404}:
            result = {"status": "forbidden"}
        else:
            # Infrastructure/programming failures must abort the request rather
            # than be stored as a terminal client error the caller cannot retry.
            raise
    result["operation_id"] = str(operation_id)
    previous.response = copy.deepcopy(result)
    db.flush()
    return result
