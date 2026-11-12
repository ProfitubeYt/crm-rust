"""Application provenance on transactions.

Revision ID: 20261112_0033
Revises: 20261111_0032
"""
from alembic import op
import sqlalchemy as sa

revision = "20261112_0033"
down_revision = "20261111_0032"
branch_labels = None
depends_on = None

TABLE = "transactions"
SOURCES = "('manual', 'import', 'dict', 'rule', 'similar', 'ai')"
CHECKS = {
    "check_application_src": f"application_src IS NULL OR application_src IN {SOURCES}",
    "check_application_confidence": (
        "application_confidence IS NULL OR "
        "(application_confidence >= 0 AND application_confidence <= 1)"
    ),
}


def upgrade() -> None:
    inspector = sa.inspect(op.get_bind())
    columns = {item["name"] for item in inspector.get_columns(TABLE)}
    checks = {item.get("name") for item in inspector.get_check_constraints(TABLE)}
    # Both columns start NULL for every existing receipt: an application stored
    # before this revision has an unknown origin, which is the protected state.
    # Nothing is backfilled here; no value is read or changed.
    with op.batch_alter_table(TABLE) as batch:
        if "application_src" not in columns:
            batch.add_column(sa.Column("application_src", sa.String(12), nullable=True))
        if "application_confidence" not in columns:
            batch.add_column(sa.Column("application_confidence", sa.Numeric(4, 3), nullable=True))
        for name, condition in CHECKS.items():
            if name not in checks:
                batch.create_check_constraint(name, condition)


def downgrade() -> None:
    inspector = sa.inspect(op.get_bind())
    columns = {item["name"] for item in inspector.get_columns(TABLE)}
    checks = {item.get("name") for item in inspector.get_check_constraints(TABLE)}
    # Only the origin of each application is lost; the applications stay.
    with op.batch_alter_table(TABLE) as batch:
        for name in CHECKS:
            if name in checks:
                batch.drop_constraint(name, type_="check")
        if "application_confidence" in columns:
            batch.drop_column("application_confidence")
        if "application_src" in columns:
            batch.drop_column("application_src")
