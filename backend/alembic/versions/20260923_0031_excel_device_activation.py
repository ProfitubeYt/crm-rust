"""Per-computer Excel grants issued by a workbook activation password.

Revision ID: 20260923_0031
Revises: 20260915_0030
"""
from alembic import op
import sqlalchemy as sa

revision = "20260923_0031"
down_revision = "20260915_0030"
branch_labels = None
depends_on = None


def upgrade() -> None:
    inspector = sa.inspect(op.get_bind())
    columns = {item["name"] for item in inspector.get_columns("excel_sync_grants")}
    # Existing grants stay unbound (NULL): computers already using them keep
    # working until the owner revokes them.
    with op.batch_alter_table("excel_sync_grants") as batch:
        if "device_id" not in columns:
            batch.add_column(sa.Column("device_id", sa.String(128), nullable=True))
        if "device_name" not in columns:
            batch.add_column(sa.Column("device_name", sa.String(120), nullable=True))
    if "excel_sync_activations" not in inspector.get_table_names():
        op.create_table(
            "excel_sync_activations",
            sa.Column("workbook_id", sa.String(128), primary_key=True),
            sa.Column("user_id", sa.Integer(), sa.ForeignKey("users.id", ondelete="CASCADE"), nullable=False),
            sa.Column("password_hash", sa.String(128), nullable=False),
            sa.Column("salt", sa.String(64), nullable=False),
            sa.Column("permissions_version", sa.Integer(), nullable=False),
            sa.Column("scope", sa.JSON(), nullable=True),
            sa.Column("failed_attempts", sa.Integer(), nullable=False, server_default="0"),
            sa.Column("locked_until", sa.DateTime(timezone=False), nullable=True),
            sa.Column("created_at", sa.DateTime(timezone=False), server_default=sa.func.now(), nullable=False),
            sa.Column("updated_at", sa.DateTime(timezone=False), server_default=sa.func.now(), nullable=False),
        )
        op.create_index("idx_excel_sync_activations_user", "excel_sync_activations", ["user_id"])


def downgrade() -> None:
    inspector = sa.inspect(op.get_bind())
    bound = op.get_bind().execute(sa.text(
        "SELECT 1 FROM excel_sync_grants WHERE device_id IS NOT NULL AND revoked_at IS NULL LIMIT 1"
    )).first()
    if bound:
        # Dropping the binding would let a computer grant work anywhere.
        raise RuntimeError(
            "Cannot downgrade while computer-bound Excel grants are active; revoke them first."
        )
    if "excel_sync_activations" in inspector.get_table_names():
        op.drop_index("idx_excel_sync_activations_user", table_name="excel_sync_activations")
        op.drop_table("excel_sync_activations")
    columns = {item["name"] for item in inspector.get_columns("excel_sync_grants")}
    with op.batch_alter_table("excel_sync_grants") as batch:
        for name in ("device_name", "device_id"):
            if name in columns:
                batch.drop_column(name)
