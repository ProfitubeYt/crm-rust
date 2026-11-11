"""Persist the Excel worksheet bank and independent P2P confirmation.

Revision ID: 20260915_0030
Revises: 20260910_0029
"""
from alembic import op
import sqlalchemy as sa

revision = "20260915_0030"
down_revision = "20260910_0029"
branch_labels = None
depends_on = None

CONSTRAINT = "check_transaction_excel_bank"


def upgrade() -> None:
    inspector = sa.inspect(op.get_bind())
    columns = {item["name"] for item in inspector.get_columns("transactions")}
    checks = {item["name"] for item in inspector.get_check_constraints("transactions")}
    # Batch mode is ALTER TABLE on PostgreSQL and also permits isolated SQLite
    # migration tests. Existing records remain NULL; no bank is guessed here.
    with op.batch_alter_table("transactions") as batch:
        if "excel_bank" not in columns:
            batch.add_column(sa.Column("excel_bank", sa.String(20), nullable=True))
        if "p2p_confirmed" not in columns:
            batch.add_column(sa.Column("p2p_confirmed", sa.Boolean(), nullable=True))
        if CONSTRAINT not in checks:
            batch.create_check_constraint(CONSTRAINT, "excel_bank IN ('CardXabar', 'HUMO Card')")


def downgrade() -> None:
    bind = op.get_bind()
    inspector = sa.inspect(bind)
    columns = {item["name"] for item in inspector.get_columns("transactions")}
    added = {"excel_bank", "p2p_confirmed"} & columns
    if not added:
        return
    populated = " OR ".join(f"{column} IS NOT NULL" for column in sorted(added))
    assigned = bind.execute(sa.text(
        f"SELECT 1 FROM transactions WHERE {populated} LIMIT 1"
    )).first()
    if assigned:
        raise RuntimeError(
            "Cannot downgrade while Excel receipts have a bank assignment or P2P confirmation; "
            "clear or migrate those values explicitly first. No receipt was changed."
        )
    checks = {item["name"] for item in inspector.get_check_constraints("transactions")}
    with op.batch_alter_table("transactions") as batch:
        if CONSTRAINT in checks:
            batch.drop_constraint(CONSTRAINT, type_="check")
        for column in sorted(added):
            batch.drop_column(column)
