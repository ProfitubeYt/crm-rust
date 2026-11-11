"""Allow explicitly issued, revocable Excel grants without a time limit.

Revision ID: 20260910_0029
Revises: 20260909_0028
"""
from alembic import op
import sqlalchemy as sa

revision = "20260910_0029"
down_revision = "20260909_0028"
branch_labels = None
depends_on = None


def _expires_at_nullable() -> bool:
    columns = sa.inspect(op.get_bind()).get_columns("excel_sync_grants")
    return next(column["nullable"] for column in columns if column["name"] == "expires_at")


def upgrade() -> None:
    if not _expires_at_nullable():
        op.alter_column(
            "excel_sync_grants", "expires_at",
            existing_type=sa.DateTime(timezone=False), nullable=True,
        )


def downgrade() -> None:
    if not _expires_at_nullable():
        return
    permanent = op.get_bind().execute(sa.text(
        "SELECT 1 FROM excel_sync_grants WHERE expires_at IS NULL LIMIT 1"
    )).first()
    if permanent:
        # Never silently expire grants or delete their exactly-once ledger.
        raise RuntimeError(
            "Cannot downgrade while Excel grants have no expiry; assign explicit "
            "expiry dates first. Grant rows and operation history were not changed."
        )
    op.alter_column(
        "excel_sync_grants", "expires_at",
        existing_type=sa.DateTime(timezone=False), nullable=False,
    )
