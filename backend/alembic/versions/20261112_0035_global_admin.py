"""CRM administration as a right of its own on the account.

Until this revision the right to administer accounts was the Checks role
``admin``: whoever ran Checks also created users and handed out access. With
more than one workspace that right belongs to the CRM as a whole, so it gets a
column of its own, separate from every workspace role.

- ``global_admin`` is NOT NULL with a FALSE server default: an account inserted
  by code or a client that has never heard of it is not a CRM administrator.
- Every account whose role is ``admin`` at this moment gets TRUE, active or
  not. Those are exactly the accounts that could administer before, so nobody
  loses the right and nobody gains it.

No role, tab, folder, source or workspace value is read or changed, and the
previous release runs unchanged against the new column.

Revision ID: 20261112_0035
Revises: 20261112_0034
"""
from alembic import op
import sqlalchemy as sa

revision = "20261112_0035"
down_revision = "20261112_0034"
branch_labels = None
depends_on = None

TABLE = "users"
COLUMN = "global_admin"


def upgrade() -> None:
    # Guarded like every migration in this project: a schema built with
    # create_all() and then stamped may already carry the column, and then
    # whatever it holds was put there on purpose.
    inspector = sa.inspect(op.get_bind())
    if COLUMN in {item["name"] for item in inspector.get_columns(TABLE)}:
        return
    with op.batch_alter_table(TABLE) as batch:
        batch.add_column(sa.Column(COLUMN, sa.Boolean(), nullable=False, server_default=sa.false()))
    users = sa.table(TABLE, sa.column("role", sa.String()), sa.column(COLUMN, sa.Boolean()))
    op.execute(
        users.update().where(sa.func.lower(users.c.role) == "admin").values({COLUMN: sa.true()})
    )


def downgrade() -> None:
    # Dropping the column forgets who was a CRM administrator; the previous
    # code reads that right from the Checks role again.
    inspector = sa.inspect(op.get_bind())
    if COLUMN not in {item["name"] for item in inspector.get_columns(TABLE)}:
        return
    with op.batch_alter_table(TABLE) as batch:
        batch.drop_column(COLUMN)
