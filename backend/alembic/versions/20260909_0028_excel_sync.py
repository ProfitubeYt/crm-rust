"""Revocable Excel grants and atomically deduplicated write operations.

Revision ID: 20260909_0028
Revises: 20260902_0027
"""
from alembic import op
import sqlalchemy as sa

revision = "20260909_0028"
down_revision = "20260902_0027"
branch_labels = None
depends_on = None


def upgrade() -> None:
    inspector = sa.inspect(op.get_bind())
    if not inspector.has_table("excel_sync_grants"):
        op.create_table(
            "excel_sync_grants",
            sa.Column("id", sa.Uuid(), primary_key=True),
            sa.Column("user_id", sa.Integer(), sa.ForeignKey("users.id", ondelete="CASCADE"), nullable=False),
            sa.Column("name", sa.String(120), nullable=False),
            sa.Column("token_hash", sa.String(64), nullable=False, unique=True),
            sa.Column("workbook_id", sa.String(128), nullable=True),
            sa.Column("permissions_version", sa.Integer(), nullable=False),
            sa.Column("scope", sa.JSON(), nullable=True),
            sa.Column("expires_at", sa.DateTime(timezone=False), nullable=False),
            sa.Column("revoked_at", sa.DateTime(timezone=False), nullable=True),
            sa.Column("created_at", sa.DateTime(timezone=False), server_default=sa.func.now(), nullable=False),
            sa.Column("last_used_at", sa.DateTime(timezone=False), nullable=True),
        )
        op.create_index("idx_excel_sync_grants_user", "excel_sync_grants", ["user_id"])
        op.create_index("idx_excel_sync_grants_expiry", "excel_sync_grants", ["expires_at"])
    if not inspector.has_table("excel_sync_operations"):
        op.create_table(
            "excel_sync_operations",
            sa.Column("id", sa.Uuid(), primary_key=True),
            sa.Column("grant_id", sa.Uuid(), sa.ForeignKey("excel_sync_grants.id", ondelete="CASCADE"), nullable=False),
            sa.Column("user_id", sa.Integer(), sa.ForeignKey("users.id", ondelete="CASCADE"), nullable=False),
            sa.Column("workbook_id", sa.String(128), nullable=False),
            sa.Column("operation_id", sa.Uuid(), nullable=False),
            sa.Column("request_hash", sa.String(64), nullable=False),
            sa.Column("response", sa.JSON(), nullable=False),
            sa.Column("created_at", sa.DateTime(timezone=False), server_default=sa.func.now(), nullable=False),
            sa.UniqueConstraint("grant_id", "operation_id", name="uq_excel_sync_operation_grant_op"),
            sa.UniqueConstraint("user_id", "workbook_id", "operation_id", name="uq_excel_sync_operation_workbook_op"),
        )
        op.create_index("idx_excel_sync_operations_created", "excel_sync_operations", ["created_at"])


def downgrade() -> None:
    inspector = sa.inspect(op.get_bind())
    for name in ("excel_sync_operations", "excel_sync_grants"):
        if inspector.has_table(name):
            op.drop_table(name)
