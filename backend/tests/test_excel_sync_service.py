"""Excel round-trip writes against isolated SQLite; no application startup."""
from __future__ import annotations

import copy
import importlib.util
from datetime import datetime, timedelta
from decimal import Decimal
from pathlib import Path
from uuid import UUID, uuid4

import pytest
from sqlalchemy import create_engine, inspect
from sqlalchemy.exc import IntegrityError
from sqlalchemy.orm import Session

from database.excel_sync_models import ExcelSyncGrant, ExcelSyncOperation
from database.models import (
    AccessAuditLog, AnnotationArticle, AnnotationPerson, SyncDeletion,
    Transaction, TransactionAnnotationEvent, User,
)
from services import annotation_service as annotations
from services import excel_sync_service as sync


@pytest.fixture
def grant(db_session):
    user = User(username="excel-test", password_hash="unused", salt="unused", role="admin")
    db_session.add(user)
    db_session.flush()
    row = ExcelSyncGrant(
        user_id=user.id, name="Test workbook", token_hash="a" * 64,
        workbook_id=str(uuid4()), permissions_version=1, scope=None,
        expires_at=datetime.utcnow() + timedelta(days=1),
    )
    db_session.add_all([
        row, AnnotationArticle(code="р1", label="Test", direction="expense"),
        AnnotationPerson(code="СТ", full_name="Test person"),
    ])
    db_session.commit()
    return row


@pytest.fixture
def tx(db_session, grant):
    row = Transaction(
        source_type="AUTO", source_chat_id=10, source_message_id=20,
        transaction_date=datetime(2026, 9, 1, 12, 30), amount=Decimal("-100.25"),
        transaction_type="DEBIT", currency="UZS", operator_raw="Test merchant",
        card_last_4="0001", raw_message="Synthetic test text", is_p2p=False,
        balance_after=Decimal("500.00"), annotation_details="Original note",
        annotation_details_src="manual",
    )
    db_session.add(row)
    db_session.commit()
    return row


def allow(_tx, *, candidate_date=None, creating=False):
    return None


def apply(db, grant, operation, authorize=allow):
    return sync.apply_operation(db, grant, operation, {"user_id": grant.user_id}, authorize)


def update(tx, fields, *, snapshot=None):
    record = snapshot or sync.serialize_record(tx)
    return {
        "operation_id": str(uuid4()), "kind": "update", "id": record["id"],
        "uuid": record["uuid"], "base": {key: record["fields"].get(key) for key in fields},
        "fields": fields,
    }


def create(**overrides):
    return {
        "operation_id": str(uuid4()), "kind": "create",
        "fields": {
            "transaction_date": "2026-09-02T10:15:00", "amount": "123.45",
            "transaction_type": "DEBIT", "currency": "UZS",
            "operator_raw": "Created merchant", "card_last_4": "0002", **overrides,
        },
    }


def delete(tx, *, version=True):
    record = sync.serialize_record(tx)
    operation = {
        "operation_id": str(uuid4()), "kind": "delete", "id": record["id"], "uuid": record["uuid"],
    }
    operation["version" if version else "base"] = record["version" if version else "fields"]
    return operation


def test_snapshot_has_exact_ids_decimal_strings_and_editable_content_hash(db_session, tx):
    record = sync.serialize_record(tx)
    assert isinstance(record["id"], str) and isinstance(record["uuid"], str)
    assert record["fields"]["amount"] == "-100.25"
    assert record["fields"]["balance_after"] == "500.00"
    assert record["fields"]["transaction_date"] == "2026-09-01T12:30:00"
    assert record["metadata"]["source_message_id"] == "20"
    assert set(record["fields"]) == sync.EDITABLE_FIELD_SET
    assert len(record["version"]) == 64
    tx.updated_at = datetime(2026, 9, 2)
    assert sync.serialize_record(tx)["version"] == record["version"]
    tx.operator_raw = "Changed"
    assert sync.serialize_record(tx)["version"] != record["version"]


def test_multiple_fields_are_normalized_and_audited_together(db_session, grant, tx):
    result = apply(db_session, grant, update(tx, {
        "amount": "-25.50", "transaction_type": "CREDIT", "operator_raw": "Changed",
        "annotation_article_code": "p1", "annotation_person_code": "CT",
        "annotation_period": "09.2026", "annotation_details": "New note",
    }))
    assert result["status"] == "applied"
    fields = result["record"]["fields"]
    assert fields["amount"] == "25.50"
    assert fields["annotation_article_code"] == "р1"
    assert fields["annotation_person_code"] == "СТ"
    assert fields["annotation_period"] == "2026-09-01"
    assert result["record"]["metadata"]["annotation_details_src"] == "manual"
    assert db_session.query(TransactionAnnotationEvent).count() == 4
    assert db_session.query(AccessAuditLog).count() == 1
    assert db_session.query(ExcelSyncOperation).count() == 1


def test_offset_date_is_converted_to_tashkent_wall_time(db_session, grant, tx):
    result = apply(db_session, grant, update(tx, {"transaction_date": "2026-09-01T07:30:00Z"}))
    assert result["status"] == "noop"
    assert tx.transaction_date == datetime(2026, 9, 1, 12, 30)


def test_direction_change_normalizes_sign_without_losing_current_magnitude(db_session, grant, tx):
    snapshot = sync.serialize_record(tx)
    tx.amount = Decimal("-200.00")
    db_session.commit()
    result = apply(db_session, grant, update(tx, {"transaction_type": "CREDIT"}, snapshot=snapshot))
    assert result["status"] == "applied"
    assert result["record"]["fields"]["amount"] == "200.00"


def test_conflict_is_per_field_and_does_not_write_any_part_of_patch(db_session, grant, tx):
    snapshot = sync.serialize_record(tx)
    tx.annotation_details = "Other editor"
    db_session.commit()
    result = apply(db_session, grant, update(tx, {
        "annotation_details": "Our edit", "operator_raw": "Should not apply",
    }, snapshot=snapshot))
    assert result["status"] == "conflict"
    assert result["conflicts"]["annotation_details"] == {
        "base": "Original note", "current": "Other editor", "proposed": "Our edit",
    }
    assert tx.operator_raw == "Test merchant"
    assert db_session.query(TransactionAnnotationEvent).count() == 0
    assert db_session.query(AccessAuditLog).count() == 0


def test_stale_identity_map_reads_concurrent_committed_value(db_session, grant, tx):
    snapshot = sync.serialize_record(tx)
    with Session(bind=db_session.get_bind()) as other:
        changed = other.get(Transaction, tx.id)
        changed.operator_raw = "Concurrent edit"
        other.commit()
    assert tx.operator_raw == "Test merchant"  # intentionally stale identity map
    result = apply(db_session, grant, update(tx, {"operator_raw": "Our edit"}, snapshot=snapshot))
    assert result["status"] == "conflict"
    assert result["conflicts"]["operator_raw"]["current"] == "Concurrent edit"


def test_disjoint_concurrent_change_is_preserved(db_session, grant, tx):
    snapshot = sync.serialize_record(tx)
    tx.receiver_name = "Another edit"
    db_session.commit()
    result = apply(db_session, grant, update(tx, {"operator_raw": "Our edit"}, snapshot=snapshot))
    assert result["status"] == "applied"
    assert result["record"]["fields"]["receiver_name"] == "Another edit"


def test_desired_value_already_current_is_noop_even_if_base_is_stale(db_session, grant, tx):
    operation = update(tx, {"operator_raw": "Already applied"})
    tx.operator_raw = "Already applied"
    db_session.commit()
    before_time = tx.updated_at
    result = apply(db_session, grant, operation)
    assert result["status"] == "noop"
    assert tx.updated_at == before_time
    assert db_session.query(AccessAuditLog).count() == 0


@pytest.mark.parametrize("field,value", [
    ("amount", "NaN"), ("amount", "Infinity"), ("amount", "0.001"),
    ("amount", "10000000000000000.00"), ("amount", True), ("amount", None),
    ("balance_after", "-Infinity"), ("transaction_date", "not a date"),
    ("transaction_type", "BAD"), ("currency", "XYZ"), ("is_p2p", "true"),
    ("card_last_4", "123"), ("card_last_4", 1234), ("receiver_card", "12xx"),
    ("raw_message", None), ("operator_raw", "a" * 256),
    ("annotation_details", "a" * 501), ("annotation_period", "99.2026"),
    ("annotation_article_code", "unknown"), ("annotation_person_code", "unknown"),
])
def test_invalid_multifield_edit_has_no_partial_writes(db_session, grant, tx, field, value):
    original = sync.serialize_record(tx)
    fields = {"application_mapped": "Should not apply", field: value}
    result = apply(db_session, grant, update(tx, fields))
    assert result["status"] == "invalid"
    assert sync.serialize_record(tx)["fields"] == original["fields"]
    assert db_session.query(TransactionAnnotationEvent).count() == 0
    assert db_session.query(AccessAuditLog).count() == 0
    assert db_session.query(ExcelSyncOperation).count() == 1


@pytest.mark.parametrize("field", ["source_chat_id", "source_type", "source_message_id", "id", "uuid", "parsing_method"])
def test_source_identity_and_metadata_cannot_be_changed(db_session, grant, tx, field):
    operation = update(tx, {"operator_raw": "Changed"})
    operation["fields"][field] = "unauthorized change"
    result = apply(db_session, grant, operation)
    assert result["status"] == "invalid"
    assert tx.operator_raw == "Test merchant"


def test_explicit_clear_retains_manual_protection_and_journal(db_session, grant, tx):
    result = apply(db_session, grant, update(tx, {"annotation_details": None}))
    assert result["status"] == "applied"
    assert tx.annotation_details is None
    assert tx.annotation_details_src == "manual"
    event = db_session.query(TransactionAnnotationEvent).one()
    assert event.before_value == "Original note" and event.after_value is None
    assert event.source == "manual"
    automated = annotations.apply_annotation_patch(
        db_session, tx, {"details": "Automatic refill"}, source="agent", validate=False,
    )
    assert automated["changed"] == {}
    assert automated["skipped"] == {"details": "protected_by:manual"}


def test_empty_clear_and_same_value_confirmation_acquire_manual_ownership(db_session, grant, tx):
    tx.annotation_details = None
    tx.annotation_details_src = "agent"
    db_session.commit()
    result = apply(db_session, grant, update(tx, {"annotation_details": None}))
    assert result["status"] == "applied"
    event = db_session.query(TransactionAnnotationEvent).one()
    assert event.before_value is None and event.after_value is None
    assert event.before_src == "agent" and event.source == "manual"
    repeated = apply(db_session, grant, update(tx, {"annotation_details": None}))
    assert repeated["status"] == "noop"
    assert db_session.query(TransactionAnnotationEvent).count() == 1


def test_annotation_fields_omitted_from_patch_are_not_cleared(db_session, grant, tx):
    result = apply(db_session, grant, update(tx, {"annotation_article_code": "р1"}))
    assert result["status"] == "applied"
    assert tx.annotation_details == "Original note"


def test_same_operation_retry_returns_same_receipt_without_second_audit(db_session, grant, tx):
    operation = update(tx, {"annotation_details": "Saved"})
    first = apply(db_session, grant, operation)
    db_session.commit()
    second = apply(db_session, grant, copy.deepcopy(operation))
    assert second == first
    assert db_session.query(ExcelSyncOperation).count() == 1
    assert db_session.query(TransactionAnnotationEvent).count() == 1
    assert db_session.query(AccessAuditLog).count() == 1


def test_operation_id_cannot_be_reused_with_a_different_payload(db_session, grant, tx):
    operation = update(tx, {"operator_raw": "First"})
    assert apply(db_session, grant, operation)["status"] == "applied"
    operation["fields"]["operator_raw"] = "Second"
    result = apply(db_session, grant, operation)
    assert result["status"] == "operation_id_reused"
    assert tx.operator_raw == "First"


@pytest.mark.parametrize("original_status", ["applied", "noop", "conflict"])
def test_replay_reauthorizes_current_scope_without_rewriting_receipt(db_session, grant, tx, original_status):
    operation = update(tx, {"operator_raw": tx.operator_raw if original_status == "noop" else "Our edit"})
    if original_status == "conflict":
        tx.operator_raw = "Other edit"
        db_session.commit()
    original = apply(db_session, grant, operation)
    assert original["status"] == original_status
    db_session.commit()
    # Simulate moving this same row into a newly forbidden month after the
    # successful write/conflict was already returned to the workbook.
    tx.transaction_date = datetime(2026, 10, 1)
    db_session.commit()
    def authorize(current, **_kwargs):
        if current.transaction_date.month == 10:
            raise PermissionError
    replay = apply(db_session, grant, operation, authorize)
    assert replay == {
        "status": "forbidden", "operation_id": operation["operation_id"],
        "replayed": True, "original_status": original_status,
        "applied": original_status in {"applied", "noop"},
    }
    assert db_session.query(ExcelSyncOperation).one().response == original
    # Restoring access permits the original receipt, without a second mutation.
    assert apply(db_session, grant, operation) == original
    assert tx.transaction_date.month == 10


def test_replay_does_not_return_snapshot_of_a_deleted_row(db_session, grant, tx):
    operation = update(tx, {"operator_raw": "Saved"})
    original = apply(db_session, grant, operation)
    db_session.commit()
    db_session.delete(tx)
    db_session.commit()
    replay = apply(db_session, grant, operation)
    assert replay == {
        "status": "not_found", "operation_id": operation["operation_id"],
        "replayed": True, "original_status": "applied", "applied": True,
    }
    assert db_session.query(ExcelSyncOperation).one().response == original


def test_replay_does_not_disclose_another_uuid_at_the_same_id(db_session, grant, tx):
    operation = update(tx, {"operator_raw": "Saved"})
    original = apply(db_session, grant, operation)
    db_session.commit()
    tx.uuid = uuid4()
    db_session.commit()
    replay = apply(db_session, grant, operation)
    assert replay == {
        "status": "not_found", "operation_id": operation["operation_id"],
        "replayed": True, "original_status": "applied", "applied": True,
    }
    assert db_session.query(ExcelSyncOperation).one().response == original


def test_uuid_guard_blocks_reused_numeric_id(db_session, grant, tx):
    operation = update(tx, {"operator_raw": "Wrong row"})
    operation["uuid"] = str(uuid4())
    result = apply(db_session, grant, operation)
    assert result["status"] == "invalid"
    assert result["error"] == "transaction_uuid_mismatch"
    assert "record" not in result
    assert tx.operator_raw == "Test merchant"


def test_forbidden_response_does_not_disclose_current_or_proposed_record(db_session, grant, tx):
    def deny(_tx, **_kwargs):
        raise PermissionError("must not be exposed")
    result = apply(db_session, grant, update(tx, {"operator_raw": "Wrong"}), deny)
    assert set(result) == {"status", "operation_id"}
    assert result["status"] == "forbidden"
    assert tx.operator_raw == "Test merchant"


def test_candidate_date_reauthorized_before_any_write(db_session, grant, tx):
    def authorize(_tx, *, candidate_date=None, creating=False):
        if candidate_date is not None and candidate_date.month == 10:
            raise PermissionError
    result = apply(db_session, grant, update(tx, {
        "operator_raw": "Wrong", "transaction_date": "2026-10-01T00:00:00",
    }), authorize)
    assert result["status"] == "forbidden"
    assert tx.operator_raw == "Test merchant"
    assert tx.transaction_date.month == 9


def test_create_is_manual_and_idempotent_without_fuzzy_deduplication(db_session, grant):
    operation = create(annotation_details="Created note")
    result = apply(db_session, grant, operation)
    assert result["status"] == "applied"
    row = db_session.get(Transaction, int(result["record"]["id"]))
    assert row.source_type == "MANUAL" and row.source_chat_id == 0
    assert row.source_message_id is None and row.source_device_sms_id is None
    assert row.amount == Decimal("-123.45") and row.annotation_details_src == "manual"
    db_session.commit()
    assert apply(db_session, grant, operation) == result
    assert db_session.query(Transaction).count() == 1
    # An explicit different operation is a second manual record even if values match.
    other = copy.deepcopy(operation)
    other["operation_id"] = str(uuid4())
    assert apply(db_session, grant, other)["status"] == "applied"
    assert db_session.query(Transaction).count() == 2


def test_invalid_create_and_forbidden_create_leave_no_transaction(db_session, grant):
    assert apply(db_session, grant, create(annotation_article_code="missing"))["status"] == "invalid"
    def deny(_tx, *, candidate_date=None, creating=False):
        assert creating and _tx.source_chat_id == 0
        raise PermissionError
    assert apply(db_session, grant, create(), deny)["status"] == "forbidden"
    assert db_session.query(Transaction).count() == 0
    assert db_session.query(AccessAuditLog).count() == 0


@pytest.mark.parametrize("version", [True, False])
def test_explicit_delete_creates_one_tombstone_and_replays(db_session, grant, tx, version):
    from services.sync_deletion_service import register_sync_deletion_listeners
    register_sync_deletion_listeners()
    operation = delete(tx, version=version)
    tx_id = tx.id
    result = apply(db_session, grant, operation)
    assert result["status"] == "deleted" and result["id"] == str(tx_id)
    db_session.commit()
    assert db_session.get(Transaction, tx_id) is None
    assert db_session.query(SyncDeletion).count() == 1
    assert db_session.query(AccessAuditLog).one().action == "excel_sync_delete"
    assert apply(db_session, grant, operation) == result
    assert db_session.query(SyncDeletion).count() == 1


def test_stale_delete_is_a_conflict_and_incomplete_base_is_invalid(db_session, grant, tx):
    operation = delete(tx)
    tx.operator_raw = "Concurrent edit"
    db_session.commit()
    assert apply(db_session, grant, operation)["status"] == "conflict"
    partial = delete(tx, version=False)
    partial["base"].pop("raw_message")
    assert apply(db_session, grant, partial)["status"] == "invalid"
    assert db_session.query(Transaction).count() == 1
    assert db_session.query(SyncDeletion).count() == 0


def test_missing_transaction_does_not_create_or_infer_delete(db_session, grant, tx):
    operation = update(tx, {"operator_raw": "Missing"})
    operation["id"] = "999999"
    assert apply(db_session, grant, operation)["status"] == "not_found"
    assert db_session.query(Transaction).count() == 1
    assert db_session.query(SyncDeletion).count() == 0


def test_caller_rollback_undoes_changes_operation_receipt_and_audit(db_session, grant, tx):
    operation = update(tx, {"annotation_details": "Will roll back"})
    assert apply(db_session, grant, operation)["status"] == "applied"
    db_session.rollback()
    assert tx.annotation_details == "Original note"
    assert db_session.query(ExcelSyncOperation).count() == 0
    assert db_session.query(TransactionAnnotationEvent).count() == 0
    assert db_session.query(AccessAuditLog).count() == 0
    assert apply(db_session, grant, operation)["status"] == "applied"


def test_one_invalid_operation_does_not_poison_later_valid_operation(db_session, grant, tx):
    assert apply(db_session, grant, update(tx, {"operator_raw": "First"}))["status"] == "applied"
    invalid = update(tx, {"operator_raw": "Wrong", "annotation_article_code": "missing"})
    assert apply(db_session, grant, invalid)["status"] == "invalid"
    assert apply(db_session, grant, update(tx, {"receiver_name": "Last"}))["status"] == "applied"
    db_session.commit()
    assert tx.operator_raw == "First" and tx.receiver_name == "Last"
    assert db_session.query(ExcelSyncOperation).count() == 3
    assert db_session.query(AccessAuditLog).count() == 2


def test_grant_cannot_be_used_with_another_actor(db_session, grant, tx):
    result = sync.apply_operation(db_session, grant, update(tx, {"operator_raw": "Wrong"}),
                                  {"user_id": grant.user_id + 1}, allow)
    assert result["status"] == "forbidden"
    assert db_session.query(ExcelSyncOperation).count() == 0


def replacement_grant(db_session, original, *, another_book=False, another_user=False):
    uid = original.user_id
    if another_user:
        user = User(username="second-excel-user", password_hash="unused", salt="unused", role="admin")
        db_session.add(user)
        db_session.flush()
        uid = user.id
    renewed = ExcelSyncGrant(
        user_id=uid, name="Replacement grant", token_hash=uuid4().hex + uuid4().hex,
        workbook_id=str(uuid4()) if another_book else original.workbook_id,
        permissions_version=1, expires_at=datetime.utcnow() + timedelta(days=1),
    )
    db_session.add(renewed)
    db_session.commit()
    return renewed


def test_grant_renewal_replays_lost_create_acknowledgement_without_duplicate(db_session, grant):
    operation = create()
    original = apply(db_session, grant, operation)
    db_session.commit()
    grant.revoked_at = datetime.utcnow()
    grant.expires_at = datetime.utcnow() - timedelta(days=1)
    db_session.commit()
    renewed = replacement_grant(db_session, grant)
    assert apply(db_session, renewed, operation) == original
    assert db_session.query(Transaction).count() == 1
    receipt = db_session.query(ExcelSyncOperation).one()
    assert receipt.grant_id == grant.id  # provenance remains the original grant
    altered = copy.deepcopy(operation)
    altered["fields"]["amount"] = "99.00"
    assert apply(db_session, renewed, altered)["status"] == "operation_id_reused"
    assert db_session.query(Transaction).count() == 1


def test_renewed_grant_replay_still_requires_current_record_authorization(db_session, grant):
    operation = create()
    original = apply(db_session, grant, operation)
    db_session.commit()
    renewed = replacement_grant(db_session, grant)
    def deny(_tx, **_kwargs):
        raise PermissionError
    result = apply(db_session, renewed, operation, deny)
    assert result == {
        "status": "forbidden", "operation_id": operation["operation_id"],
        "replayed": True, "original_status": "applied", "applied": True,
    }
    assert db_session.query(ExcelSyncOperation).one().response == original
    assert apply(db_session, renewed, operation) == original
    assert db_session.query(Transaction).count() == 1


@pytest.mark.parametrize("missing_reason", ["deleted", "uuid_changed"])
def test_lost_create_ack_reports_applied_after_row_disappears(db_session, grant, missing_reason):
    operation = create()
    original = apply(db_session, grant, operation)
    db_session.commit()
    created = db_session.query(Transaction).one()
    if missing_reason == "deleted":
        db_session.delete(created)
    else:
        created.uuid = uuid4()
    db_session.commit()
    renewed = replacement_grant(db_session, grant)
    for _ in range(2):
        replay = apply(db_session, renewed, operation)
        assert replay == {
            "status": "not_found", "operation_id": operation["operation_id"],
            "replayed": True, "original_status": "applied", "applied": True,
        }
    assert db_session.query(ExcelSyncOperation).one().response == original
    assert db_session.query(Transaction).count() == (0 if missing_reason == "deleted" else 1)


def test_initial_forbidden_create_has_no_applied_replay_marker(db_session, grant):
    operation = create()
    def deny(_tx, **_kwargs):
        raise PermissionError
    original = apply(db_session, grant, operation, deny)
    assert original == {"status": "forbidden", "operation_id": operation["operation_id"]}
    db_session.commit()
    renewed = replacement_grant(db_session, grant)
    assert apply(db_session, renewed, operation) == original
    assert db_session.query(Transaction).count() == 0


@pytest.mark.parametrize("another_book,another_user", [(True, False), (False, True)])
def test_receipts_do_not_cross_workbook_or_user_boundaries(db_session, grant, another_book, another_user):
    operation = create()
    first = apply(db_session, grant, operation)
    db_session.commit()
    other = replacement_grant(db_session, grant, another_book=another_book, another_user=another_user)
    second = apply(db_session, other, operation)
    assert second["status"] == "applied"
    assert first["record"]["uuid"] != second["record"]["uuid"]
    assert db_session.query(Transaction).count() == 2
    assert db_session.query(ExcelSyncOperation).count() == 2


def test_database_unique_ledger_prevents_duplicate_receipt_under_another_grant(db_session, grant):
    operation = create()
    apply(db_session, grant, operation)
    db_session.commit()
    renewed = replacement_grant(db_session, grant)
    original = db_session.query(ExcelSyncOperation).one()
    with pytest.raises(IntegrityError):
        with db_session.begin_nested():
            db_session.add(ExcelSyncOperation(
                grant_id=renewed.id, user_id=renewed.user_id, workbook_id=renewed.workbook_id,
                operation_id=original.operation_id, request_hash=original.request_hash, response={},
            ))
            db_session.flush()
    assert db_session.query(ExcelSyncOperation).count() == 1


def test_unbound_workbook_cannot_write(db_session, grant):
    grant.workbook_id = None
    db_session.commit()
    result = apply(db_session, grant, create())
    assert result["status"] == "invalid" and result["error"] == "workbook_not_bound"
    assert db_session.query(Transaction).count() == 0


def test_migration_creates_tables_and_is_repeatable(monkeypatch):
    from alembic.migration import MigrationContext
    from alembic.operations import Operations
    path = Path(__file__).resolve().parents[1] / "alembic" / "versions" / "20260909_0028_excel_sync.py"
    spec = importlib.util.spec_from_file_location("excel_sync_migration", path)
    migration = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(migration)
    engine = create_engine("sqlite://")
    User.__table__.create(engine)
    with engine.begin() as connection:
        monkeypatch.setattr(migration, "op", Operations(MigrationContext.configure(connection)))
        migration.upgrade()
        migration.upgrade()
        inspector = inspect(connection)
        assert inspector.has_table("excel_sync_grants")
        assert inspector.has_table("excel_sync_operations")
        unique = inspector.get_unique_constraints("excel_sync_operations")
        assert any(item["column_names"] == ["grant_id", "operation_id"] for item in unique)
        assert any(item["column_names"] == ["user_id", "workbook_id", "operation_id"] for item in unique)
        migration.downgrade()
        migration.downgrade()
        assert not inspect(connection).has_table("excel_sync_operations")


@pytest.mark.parametrize("bank", [None, "CardXabar", "HUMO Card"])
def test_excel_bank_create_persists_across_sessions_and_retries(db_session, grant, bank):
    operation = create(excel_bank=bank)
    original = copy.deepcopy(operation)
    result = apply(db_session, grant, operation)
    assert result["status"] == "applied"
    assert result["record"]["fields"]["excel_bank"] == bank
    assert result["record"]["metadata"]["source_type"] == "MANUAL"
    assert result["record"]["metadata"]["source_chat_id"] == "0"
    db_session.commit()
    with Session(bind=db_session.get_bind()) as other:
        stored = other.get(Transaction, int(result["record"]["id"]))
        assert stored.excel_bank == bank
        assert sync.serialize_record(stored)["fields"]["excel_bank"] == bank
    assert apply(db_session, grant, operation) == result
    assert operation == original
    assert db_session.query(Transaction).count() == 1
    assert db_session.query(ExcelSyncOperation).one().request_hash == sync._hash_json(original)
    assert db_session.query(AccessAuditLog).count() == 1


@pytest.mark.parametrize("bank", ["", "HUMO", "humo card", "CardXabar ", "Other", True, 1, [], {}])
def test_invalid_excel_bank_rejects_entire_update(db_session, grant, tx, bank):
    result = apply(db_session, grant, update(tx, {
        "excel_bank": bank, "operator_raw": "Must not change",
    }))
    assert result["status"] == "invalid"
    assert result["error"] == "invalid_excel_bank"
    assert tx.excel_bank is None
    assert tx.operator_raw == "Test merchant"
    assert db_session.query(AccessAuditLog).count() == 0


def test_excel_bank_update_clear_versions_and_disjoint_edit(db_session, grant, tx):
    original = sync.serialize_record(tx)
    legacy_fields = {key: original["fields"][key] for key in sync.LEGACY_EDITABLE_FIELDS}
    assert original["version"] == sync._hash_json(legacy_fields)
    tx.receiver_name = "Concurrent receiver"
    db_session.commit()
    assigned = apply(db_session, grant, update(tx, {"excel_bank": "HUMO Card"}, snapshot=original))
    assert assigned["status"] == "applied"
    assert assigned["record"]["fields"]["receiver_name"] == "Concurrent receiver"
    assert assigned["record"]["version"] != original["version"]
    assert tx.source_type == "AUTO" and tx.source_chat_id == 10
    db_session.commit()
    duplicate_value = apply(db_session, grant, update(tx, {"excel_bank": "HUMO Card"}, snapshot=original))
    assert duplicate_value["status"] == "noop"
    cleared = apply(db_session, grant, update(tx, {"excel_bank": None}))
    assert cleared["status"] == "applied"
    assert cleared["record"]["fields"]["excel_bank"] is None
    expected = {key: cleared["record"]["fields"][key] for key in sync.LEGACY_EDITABLE_FIELDS}
    assert cleared["record"]["version"] == sync._hash_json(expected)


def test_excel_bank_conflict_does_not_apply_other_fields(db_session, grant, tx):
    original = sync.serialize_record(tx)
    tx.excel_bank = "HUMO Card"
    db_session.commit()
    operation = update(tx, {"excel_bank": "CardXabar", "operator_raw": "Must not change"}, snapshot=original)
    result = apply(db_session, grant, operation)
    assert result["status"] == "conflict"
    assert result["conflicts"] == {"excel_bank": {
        "base": None, "current": "HUMO Card", "proposed": "CardXabar",
    }}
    assert tx.excel_bank == "HUMO Card"
    assert tx.operator_raw == "Test merchant"
    db_session.commit()
    assert apply(db_session, grant, operation) == result


def test_legacy_full_field_update_does_not_clear_bank(db_session, grant, tx):
    tx.excel_bank = "HUMO Card"
    tx.p2p_confirmed = True
    db_session.commit()
    record = sync.serialize_record(tx)
    fields = {key: record["fields"][key] for key in sync.LEGACY_EDITABLE_FIELDS}
    fields["operator_raw"] = "Legacy client edit"
    operation = update(tx, fields)
    original = copy.deepcopy(operation)
    result = apply(db_session, grant, operation)
    assert result["status"] == "applied"
    assert result["record"]["fields"]["excel_bank"] == "HUMO Card"
    assert result["record"]["fields"]["p2p_confirmed"] is True
    assert result["record"]["fields"]["operator_raw"] == "Legacy client edit"
    assert operation == original and "excel_bank" not in operation["fields"]


@pytest.mark.parametrize("version", [True, False])
@pytest.mark.parametrize("bank", [None, "HUMO Card"])
@pytest.mark.parametrize("confirmed", [None, False, True])
def test_legacy_delete_preserves_unknown_extensions(db_session, grant, tx, version, bank, confirmed):
    record = sync.serialize_record(tx)
    legacy_fields = {key: record["fields"][key] for key in sync.LEGACY_EDITABLE_FIELDS}
    operation = {"operation_id": str(uuid4()), "kind": "delete", "id": record["id"], "uuid": record["uuid"]}
    operation["version" if version else "base"] = sync._hash_json(legacy_fields) if version else legacy_fields
    tx.excel_bank = bank
    tx.p2p_confirmed = confirmed
    db_session.commit()
    result = apply(db_session, grant, operation)
    unchanged = bank is None and confirmed is None
    assert result["status"] == ("deleted" if unchanged else "conflict")
    assert db_session.query(Transaction).count() == (0 if unchanged else 1)
    if not unchanged and not version:
        assert result["conflicts"] == {
            key: {"base": None, "current": value, "proposed": None}
            for key, value in {"excel_bank": bank, "p2p_confirmed": confirmed}.items() if value is not None
        }


def test_legacy_operation_receipt_is_replayed_without_new_fields_or_rehash(db_session, grant, tx):
    operation = update(tx, {"operator_raw": "Test merchant"})
    record = sync.serialize_record(tx)
    for field in sync.NULLABLE_EXTENSION_FIELDS:
        del record["fields"][field]
    legacy_response = {"status": "noop", "record": record, "operation_id": operation["operation_id"]}
    receipt = ExcelSyncOperation(
        grant_id=grant.id, user_id=grant.user_id, workbook_id=grant.workbook_id,
        operation_id=UUID(operation["operation_id"]), request_hash=sync._hash_json(operation),
        response=copy.deepcopy(legacy_response),
    )
    db_session.add(receipt)
    db_session.commit()
    original_hash = receipt.request_hash
    assert apply(db_session, grant, operation) == legacy_response
    assert receipt.response == legacy_response
    assert receipt.request_hash == original_hash
    assert db_session.query(AccessAuditLog).count() == 0
    assert tx.excel_bank is None


def test_excel_bank_migration_preserves_rows_and_guards_downgrade(monkeypatch):
    from alembic.migration import MigrationContext
    from alembic.operations import Operations
    import sqlalchemy as sa

    path = Path(__file__).resolve().parents[1] / "alembic" / "versions" / "20260915_0030_excel_bank.py"
    spec = importlib.util.spec_from_file_location("excel_bank_migration", path)
    migration = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(migration)
    engine = create_engine("sqlite://")
    with engine.begin() as connection:
        connection.exec_driver_sql("CREATE TABLE transactions (id INTEGER PRIMARY KEY, raw_message TEXT NOT NULL)")
        connection.exec_driver_sql("INSERT INTO transactions VALUES (1, 'Synthetic original text')")
        monkeypatch.setattr(migration, "op", Operations(MigrationContext.configure(connection)))
        migration.upgrade()
        migration.upgrade()
        columns = {item["name"]: item for item in inspect(connection).get_columns("transactions")}
        assert columns["excel_bank"]["nullable"] is True
        assert columns["p2p_confirmed"]["nullable"] is True
        assert connection.execute(sa.text("SELECT raw_message, excel_bank, p2p_confirmed FROM transactions")).one() == (
            "Synthetic original text", None, None,
        )
        with pytest.raises(IntegrityError), connection.begin_nested():
            connection.execute(sa.text("UPDATE transactions SET excel_bank='Other' WHERE id=1"))
        connection.execute(sa.text("UPDATE transactions SET excel_bank='HUMO Card' WHERE id=1"))
        with pytest.raises(RuntimeError, match="bank assignment"):
            migration.downgrade()
        assert connection.execute(sa.text("SELECT excel_bank FROM transactions")).scalar_one() == "HUMO Card"
        connection.execute(sa.text("UPDATE transactions SET excel_bank=NULL WHERE id=1"))
        connection.execute(sa.text("UPDATE transactions SET p2p_confirmed=0 WHERE id=1"))
        with pytest.raises(RuntimeError, match="P2P confirmation"):
            migration.downgrade()
        assert connection.execute(sa.text("SELECT p2p_confirmed FROM transactions")).scalar_one() == 0
        connection.execute(sa.text("UPDATE transactions SET p2p_confirmed=NULL WHERE id=1"))
        migration.downgrade()
        migration.downgrade()
        assert "excel_bank" not in {item["name"] for item in inspect(connection).get_columns("transactions")}
        assert "p2p_confirmed" not in {item["name"] for item in inspect(connection).get_columns("transactions")}
        assert connection.execute(sa.text("SELECT raw_message FROM transactions")).scalar_one() == "Synthetic original text"


@pytest.mark.parametrize("confirmed", [None, False, True])
def test_p2p_confirmation_create_persists_without_changing_p2p(db_session, grant, confirmed):
    operation = create(excel_bank="HUMO Card", p2p_confirmed=confirmed)
    result = apply(db_session, grant, operation)
    assert result["status"] == "applied"
    assert result["record"]["fields"]["p2p_confirmed"] is confirmed
    assert result["record"]["fields"]["is_p2p"] is False
    db_session.commit()
    with Session(bind=db_session.get_bind()) as other:
        stored = other.get(Transaction, int(result["record"]["id"]))
        assert stored.p2p_confirmed is confirmed
    assert apply(db_session, grant, operation) == result
    assert db_session.query(Transaction).count() == 1
    assert db_session.query(AccessAuditLog).count() == 1


@pytest.mark.parametrize("value", [0, 1, "0", "1", "true", "false", "", [], {}])
def test_p2p_confirmation_rejects_non_boolean_without_partial_write(db_session, grant, tx, value):
    result = apply(db_session, grant, update(tx, {"p2p_confirmed": value, "operator_raw": "Must not change"}))
    assert result["status"] == "invalid"
    assert result["error"] == "invalid_p2p_confirmed"
    assert tx.p2p_confirmed is None and tx.is_p2p is False
    assert tx.operator_raw == "Test merchant"


def test_p2p_confirmation_three_states_have_distinct_versions_and_clear_is_null(db_session, grant, tx):
    original = sync.serialize_record(tx)
    unconfirmed = apply(db_session, grant, update(tx, {"p2p_confirmed": False}))
    assert unconfirmed["status"] == "applied"
    confirmed = apply(db_session, grant, update(tx, {"p2p_confirmed": True}))
    assert confirmed["status"] == "applied"
    cleared = apply(db_session, grant, update(tx, {"p2p_confirmed": None}))
    assert cleared["status"] == "applied"
    assert cleared["record"]["fields"]["p2p_confirmed"] is None
    assert len({original["version"], unconfirmed["record"]["version"], confirmed["record"]["version"]}) == 3
    assert cleared["record"]["version"] == original["version"]
    assert tx.is_p2p is False


def test_p2p_confirmation_conflict_preserves_false_and_replays(db_session, grant, tx):
    original = sync.serialize_record(tx)
    tx.p2p_confirmed = False
    db_session.commit()
    operation = update(tx, {"p2p_confirmed": True, "operator_raw": "Must not change"}, snapshot=original)
    result = apply(db_session, grant, operation)
    assert result["status"] == "conflict"
    assert result["conflicts"] == {"p2p_confirmed": {"base": None, "current": False, "proposed": True}}
    assert tx.p2p_confirmed is False and tx.operator_raw == "Test merchant"
    db_session.commit()
    assert apply(db_session, grant, operation) == result


@pytest.mark.parametrize("field,value", [("p2p_confirmed", 0), ("excel_bank", "other")])
def test_nullable_extension_base_must_be_canonical(db_session, grant, tx, field, value):
    operation = update(tx, {field: True if field == "p2p_confirmed" else "HUMO Card"})
    operation["base"][field] = value
    result = apply(db_session, grant, operation)
    assert result["status"] == "invalid"
    assert result["error"] == f"invalid_base_{field}"
    assert tx.p2p_confirmed is None and tx.excel_bank is None
