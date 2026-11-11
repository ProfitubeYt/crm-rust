"""Give a user account an expiry date.

The desktop app is licensed per person, and until now nothing recorded until
when. `locked_until` looks like it would serve — it does not: that is the
lockout after failed logins, a security response measured in minutes, not an
entitlement measured in months. Overloading it would have made a lockout look
like an expired licence and an expiry look like a break-in.

Nullable on purpose: an account with no date set is not expired, it is simply
unlimited, which is what every existing account is today.

Revision ID: 20260902_0027
Revises: 20260901_0026
"""
from alembic import op
import sqlalchemy as sa

revision = "20260902_0027"
down_revision = "20260901_0026"
branch_labels = None
depends_on = None


def _has_column(inspector, table: str, column: str) -> bool:
    return column in {col["name"] for col in inspector.get_columns(table)}


def upgrade() -> None:
    # Guarded like every migration in this project: the chain does not run
    # cleanly from zero, so a schema built with create_all() then stamped may
    # already carry the column.
    inspector = sa.inspect(op.get_bind())

    if not _has_column(inspector, "users", "access_until"):
        op.add_column("users", sa.Column("access_until", sa.DateTime(timezone=True), nullable=True))
    if not _has_column(inspector, "users", "access_note"):
        op.add_column("users", sa.Column("access_note", sa.String(200), nullable=True))


def downgrade() -> None:
    inspector = sa.inspect(op.get_bind())
    if _has_column(inspector, "users", "access_note"):
        op.drop_column("users", "access_note")
    if _has_column(inspector, "users", "access_until"):
        op.drop_column("users", "access_until")
