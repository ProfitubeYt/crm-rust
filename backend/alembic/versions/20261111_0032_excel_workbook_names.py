"""Names of the Excel workbooks the server prepares.

Revision ID: 20261111_0032
Revises: 20260923_0031
"""
from alembic import op
import sqlalchemy as sa

revision = "20261111_0032"
down_revision = "20260923_0031"
branch_labels = None
depends_on = None

TABLE = "excel_sync_activations"


def upgrade() -> None:
    columns = {item["name"] for item in sa.inspect(op.get_bind()).get_columns(TABLE)}
    # A workbook the server prepares is its password record: the record exists
    # before any grant and carries the name the owner gave. Records of
    # workbooks that were connected by hand stay NULL.
    if "name" not in columns:
        with op.batch_alter_table(TABLE) as batch:
            batch.add_column(sa.Column("name", sa.String(120), nullable=True))


def downgrade() -> None:
    columns = {item["name"] for item in sa.inspect(op.get_bind()).get_columns(TABLE)}
    # Only the names are lost. The records stay, so the previous release still
    # activates a prepared workbook by its password and lists its computers.
    if "name" in columns:
        with op.batch_alter_table(TABLE) as batch:
            batch.drop_column("name")
