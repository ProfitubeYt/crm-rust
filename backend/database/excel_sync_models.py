"""Persisted, revocable Excel grants and transactional operation receipts."""
from __future__ import annotations

import uuid
from datetime import datetime

from sqlalchemy import Column, DateTime, ForeignKey, Index, Integer, JSON, String, UniqueConstraint, Uuid
from sqlalchemy.sql import func

from .models import Base


class ExcelSyncGrant(Base):
    __tablename__ = "excel_sync_grants"

    id = Column(Uuid(as_uuid=True), primary_key=True, default=uuid.uuid4)
    user_id = Column(Integer, ForeignKey("users.id", ondelete="CASCADE"), nullable=False)
    name = Column(String(120), nullable=False)
    token_hash = Column(String(64), nullable=False, unique=True)
    workbook_id = Column(String(128), nullable=True)
    # Set for connections issued to one computer by password activation. Such a
    # grant is accepted only with the same X-Excel-Device header.
    device_id = Column(String(128), nullable=True)
    device_name = Column(String(120), nullable=True)
    permissions_version = Column(Integer, nullable=False)
    scope = Column(JSON, nullable=True)
    # NULL is an explicitly issued permanent connection; revocation and all
    # current user/scope/workbook checks still apply.
    expires_at = Column(DateTime(timezone=False), nullable=True)
    revoked_at = Column(DateTime(timezone=False), nullable=True)
    created_at = Column(DateTime(timezone=False), default=datetime.utcnow, server_default=func.now(), nullable=False)
    last_used_at = Column(DateTime(timezone=False), nullable=True)

    __table_args__ = (
        Index("idx_excel_sync_grants_user", "user_id"),
        Index("idx_excel_sync_grants_expiry", "expires_at"),
    )


class ExcelSyncActivation(Base):
    """Password that lets a new computer obtain its own grant for one workbook.

    Only a PBKDF2 hash is stored. The scope and permissions version are captured
    when the owner sets the password, exactly like an explicitly issued grant.
    """
    __tablename__ = "excel_sync_activations"

    workbook_id = Column(String(128), primary_key=True)
    user_id = Column(Integer, ForeignKey("users.id", ondelete="CASCADE"), nullable=False)
    password_hash = Column(String(128), nullable=False)
    salt = Column(String(64), nullable=False)
    permissions_version = Column(Integer, nullable=False)
    scope = Column(JSON, nullable=True)
    failed_attempts = Column(Integer, nullable=False, default=0, server_default="0")
    locked_until = Column(DateTime(timezone=False), nullable=True)
    created_at = Column(DateTime(timezone=False), default=datetime.utcnow, server_default=func.now(), nullable=False)
    updated_at = Column(DateTime(timezone=False), default=datetime.utcnow, server_default=func.now(), nullable=False)

    __table_args__ = (Index("idx_excel_sync_activations_user", "user_id"),)


class ExcelSyncOperation(Base):
    __tablename__ = "excel_sync_operations"

    id = Column(Uuid(as_uuid=True), primary_key=True, default=uuid.uuid4)
    grant_id = Column(Uuid(as_uuid=True), ForeignKey("excel_sync_grants.id", ondelete="CASCADE"), nullable=False)
    # Stable across replacement/revocation of the credential. Otherwise a lost
    # create acknowledgement followed by grant renewal could create a duplicate.
    user_id = Column(Integer, ForeignKey("users.id", ondelete="CASCADE"), nullable=False)
    workbook_id = Column(String(128), nullable=False)
    operation_id = Column(Uuid(as_uuid=True), nullable=False)
    request_hash = Column(String(64), nullable=False)
    response = Column(JSON, nullable=False)
    created_at = Column(DateTime(timezone=False), default=datetime.utcnow, server_default=func.now(), nullable=False)

    __table_args__ = (
        UniqueConstraint("grant_id", "operation_id", name="uq_excel_sync_operation_grant_op"),
        UniqueConstraint("user_id", "workbook_id", "operation_id", name="uq_excel_sync_operation_workbook_op"),
        Index("idx_excel_sync_operations_created", "created_at"),
    )
