"""Workspace access on user accounts.

The Hub now opens more than one workspace on one account. Until this revision
an account was a Checks account and nothing else, so the two columns record
exactly that and nothing more:

- ``checks_access`` is NOT NULL with a TRUE server default. Every existing row
  reads TRUE without being rewritten, and an account inserted by code or a
  client that has never heard of workspaces still gets Checks, as before.
- ``time_role`` is NULL for everyone: no account receives the Time workspace
  from this migration. It is granted by an administrator, one account at a time.

Nothing is backfilled, no role, tab, folder or source restriction is read or
changed, and the previous release runs unchanged against the new columns.

Revision ID: 20261112_0034
Revises: 20261112_0033
"""
from alembic import op
import sqlalchemy as sa

revision = "20261112_0034"
down_revision = "20261112_0033"
branch_labels = None
depends_on = None

TABLE = "users"
TIME_ROLE_CHECK = "check_user_time_role"
TIME_ROLE_CONDITION = "time_role IS NULL OR time_role IN ('employee', 'viewer', 'manager')"


def upgrade() -> None:
    # Guarded like every migration in this project: a schema built with
    # create_all() and then stamped may already carry the columns.
    inspector = sa.inspect(op.get_bind())
    columns = {item["name"] for item in inspector.get_columns(TABLE)}
    checks = {item.get("name") for item in inspector.get_check_constraints(TABLE)}
    with op.batch_alter_table(TABLE) as batch:
        if "checks_access" not in columns:
            batch.add_column(
                sa.Column("checks_access", sa.Boolean(), nullable=False, server_default=sa.true())
            )
        if "time_role" not in columns:
            batch.add_column(sa.Column("time_role", sa.String(20), nullable=True))
        if TIME_ROLE_CHECK not in checks:
            batch.create_check_constraint(TIME_ROLE_CHECK, TIME_ROLE_CONDITION)


def downgrade() -> None:
    # Dropping the columns forgets who was granted Time and who was taken off
    # Checks; with the previous code every account is a Checks account again.
    inspector = sa.inspect(op.get_bind())
    columns = {item["name"] for item in inspector.get_columns(TABLE)}
    checks = {item.get("name") for item in inspector.get_check_constraints(TABLE)}
    with op.batch_alter_table(TABLE) as batch:
        if TIME_ROLE_CHECK in checks:
            batch.drop_constraint(TIME_ROLE_CHECK, type_="check")
        if "time_role" in columns:
            batch.drop_column("time_role")
        if "checks_access" in columns:
            batch.drop_column("checks_access")
