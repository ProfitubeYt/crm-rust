"""Transaction annotations: per-receipt bookkeeping fields + reference tables.

Adds the four annotation fields the client keeps by hand in their workbook
(details / person / period / article), each with its own provenance column, plus
the reference tables, the learned-rule table, the change journal and the import
batch registry.

Provenance is per field rather than per row on purpose: rules, the importer and
the agent each need to answer "may I overwrite *this* field", and one row-level
flag cannot express that.

Revision ID: 20260901_0026
Revises: 20260713_0025
Create Date: 2026-09-01
"""
from __future__ import annotations

from alembic import op
import sqlalchemy as sa
from sqlalchemy.engine.reflection import Inspector


revision = "20260901_0026"
down_revision = "20260713_0025"
branch_labels = None
depends_on = None


# Seeded from the client's workbook. The 'р' prefix is an expense, 'п' income.
_ARTICLES = [
    ("п1", "банк, ШЗП, П, МП", "income", 101),
    ("п2", "банк, дивиденды", "income", 102),
    ("п6", "возвр.авансов", "income", 106),
    ("п7", "фин.кред.опер.", "income", 107),
    ("п8", "вал.обм.опер.", "income", 108),
    ("п9", "прочие приходы", "income", 109),
    ("р1", "ФЗП,ФМП,МП", "expense", 201),
    ("р2", "комп.затр.сотр.", "expense", 202),
    ("р3", "МП,благотвор.", "expense", 203),
    ("р4", "опер.,АХ расх.", "expense", 204),
    ("р5", "опл.раб./мат.", "expense", 205),
    ("р7", "фин.кред.опер.", "expense", 207),
    ("р8", "вал.обм.опер.", "expense", 208),
    ("р9", "прочие расходы", "expense", 209),
]

_PERSONS = [
    ("СТ", 1), ("ДиК", 2), ("ФаА", 3), ("КаР", 4), ("СтК", 5), ("ХуА", 6),
    ("НуМ", 7), ("АлК", 8), ("ЕлИ", 9), ("ВлЛ", 10), ("ЮлК", 11),
]

_ANNOTATION_COLUMNS = [
    ("annotation_details", sa.String(length=500)),
    ("annotation_person_code", sa.String(length=32)),
    ("annotation_period", sa.Date()),
    ("annotation_article_code", sa.String(length=16)),
    ("annotation_details_src", sa.String(length=12)),
    ("annotation_person_src", sa.String(length=12)),
    ("annotation_period_src", sa.String(length=12)),
    ("annotation_article_src", sa.String(length=12)),
    ("annotation_updated_at", sa.DateTime(timezone=False)),
    ("annotation_updated_by", sa.BigInteger()),
    ("annotation_import_hash", sa.String(length=64)),
]

_SRC_FIELDS = (
    "annotation_details_src",
    "annotation_person_src",
    "annotation_period_src",
    "annotation_article_src",
)


def _has_table(inspector: Inspector, table: str) -> bool:
    try:
        return inspector.has_table(table)
    except Exception:
        return False


def _has_column(inspector: Inspector, table: str, column: str) -> bool:
    try:
        return any(col["name"] == column for col in inspector.get_columns(table))
    except Exception:
        return False


def _has_index(inspector: Inspector, table: str, index_name: str) -> bool:
    try:
        return any(ix["name"] == index_name for ix in inspector.get_indexes(table))
    except Exception:
        return False


def upgrade() -> None:
    bind = op.get_bind()
    inspector = Inspector.from_engine(bind)
    is_postgres = bind.dialect.name == "postgresql"

    # --- 1. Annotation columns on transactions -----------------------------
    for name, type_ in _ANNOTATION_COLUMNS:
        if not _has_column(inspector, "transactions", name):
            op.add_column("transactions", sa.Column(name, type_, nullable=True))

    inspector = Inspector.from_engine(bind)

    if is_postgres:
        # SQLite cannot ALTER TABLE ADD CONSTRAINT; on that dialect the checks
        # come from __table_args__ at create_all time instead.
        for field in _SRC_FIELDS:
            op.execute(
                f"ALTER TABLE transactions DROP CONSTRAINT IF EXISTS check_{field}"
            )
            op.execute(
                f"ALTER TABLE transactions ADD CONSTRAINT check_{field} "
                f"CHECK ({field} IS NULL OR {field} IN ('manual','import','rule','agent'))"
            )

        op.execute(
            "CREATE INDEX IF NOT EXISTS idx_transactions_annotation_article "
            "ON transactions (annotation_article_code) "
            "WHERE annotation_article_code IS NOT NULL"
        )
        op.execute(
            "CREATE INDEX IF NOT EXISTS idx_transactions_annotation_person "
            "ON transactions (annotation_person_code) "
            "WHERE annotation_person_code IS NOT NULL"
        )
        op.execute(
            "CREATE INDEX IF NOT EXISTS idx_transactions_annotation_period "
            "ON transactions (annotation_period, annotation_article_code) "
            "WHERE annotation_period IS NOT NULL"
        )
        # Mirrors idx_transactions_raw_message_trgm: without it, free-text search
        # over the annotation silently degrades into a seq scan as rows pile up.
        op.execute("CREATE EXTENSION IF NOT EXISTS pg_trgm")
        # Built CONCURRENTLY: a plain CREATE INDEX takes ACCESS EXCLUSIVE on
        # `transactions` for the whole build, which would stall the API mid-deploy.
        # CONCURRENTLY cannot run inside a transaction, hence the autocommit block.
        with op.get_context().autocommit_block():
            op.execute(
                "CREATE INDEX CONCURRENTLY IF NOT EXISTS "
                "idx_transactions_annotation_details_trgm "
                "ON transactions USING gin (annotation_details gin_trgm_ops)"
            )

    # --- 2. Reference tables ------------------------------------------------
    if not _has_table(inspector, "annotation_articles"):
        op.create_table(
            "annotation_articles",
            sa.Column("id", sa.Integer(), primary_key=True, autoincrement=True),
            sa.Column("code", sa.String(length=16), nullable=False),
            sa.Column("label", sa.String(length=200), nullable=False),
            sa.Column("direction", sa.String(length=8), nullable=False),
            sa.Column("sort_order", sa.Integer(), nullable=False, server_default="0"),
            sa.Column("is_active", sa.Boolean(), nullable=False, server_default=sa.true()),
            sa.Column("is_auto", sa.Boolean(), nullable=False, server_default=sa.false()),
            sa.Column("created_at", sa.DateTime(timezone=True), server_default=sa.func.now(), nullable=False),
            sa.Column("updated_at", sa.DateTime(timezone=True), server_default=sa.func.now(), nullable=True),
            sa.CheckConstraint("direction IN ('expense', 'income')", name="check_annotation_article_direction"),
        )
        op.create_index("uq_annotation_articles_code", "annotation_articles", ["code"], unique=True)
        op.create_index("idx_annotation_articles_active", "annotation_articles", ["is_active", "sort_order"])

    if not _has_table(inspector, "annotation_persons"):
        op.create_table(
            "annotation_persons",
            sa.Column("id", sa.Integer(), primary_key=True, autoincrement=True),
            sa.Column("code", sa.String(length=32), nullable=False),
            sa.Column("full_name", sa.String(length=200), nullable=True),
            sa.Column("sort_order", sa.Integer(), nullable=False, server_default="0"),
            sa.Column("is_active", sa.Boolean(), nullable=False, server_default=sa.true()),
            sa.Column("is_auto", sa.Boolean(), nullable=False, server_default=sa.false()),
            sa.Column("created_at", sa.DateTime(timezone=True), server_default=sa.func.now(), nullable=False),
            sa.Column("updated_at", sa.DateTime(timezone=True), server_default=sa.func.now(), nullable=True),
        )
        op.create_index("uq_annotation_persons_code", "annotation_persons", ["code"], unique=True)
        op.create_index("idx_annotation_persons_active", "annotation_persons", ["is_active", "sort_order"])

    # --- 3. Learned rules ---------------------------------------------------
    if not _has_table(inspector, "annotation_rules"):
        op.create_table(
            "annotation_rules",
            sa.Column("id", sa.Integer(), primary_key=True, autoincrement=True),
            sa.Column("operator_key", sa.String(length=500), nullable=False),
            sa.Column("field", sa.String(length=16), nullable=False),
            sa.Column("value", sa.String(length=500), nullable=False),
            sa.Column("support", sa.Integer(), nullable=False, server_default="0"),
            sa.Column("purity", sa.Numeric(4, 3), nullable=False),
            sa.Column("is_enabled", sa.Boolean(), nullable=False, server_default=sa.true()),
            sa.Column("is_pinned", sa.Boolean(), nullable=False, server_default=sa.false()),
            sa.Column("computed_at", sa.DateTime(timezone=False), nullable=True),
            sa.Column("created_by_user_id", sa.BigInteger(), nullable=True),
            sa.CheckConstraint("field IN ('details', 'person', 'article')", name="check_annotation_rule_field"),
        )
        op.create_index("uq_annotation_rules", "annotation_rules", ["operator_key", "field"], unique=True)
        op.create_index("idx_annotation_rules_enabled", "annotation_rules", ["is_enabled", "field"])

    # --- 4. Change journal --------------------------------------------------
    if not _has_table(inspector, "transaction_annotation_events"):
        op.create_table(
            "transaction_annotation_events",
            sa.Column("id", sa.BigInteger().with_variant(sa.Integer, "sqlite"), primary_key=True, autoincrement=True),
            sa.Column("transaction_id", sa.BigInteger(), nullable=False),
            sa.Column("field", sa.String(length=16), nullable=False),
            sa.Column("before_value", sa.String(length=500), nullable=True),
            sa.Column("after_value", sa.String(length=500), nullable=True),
            sa.Column("before_src", sa.String(length=12), nullable=True),
            sa.Column("source", sa.String(length=12), nullable=False),
            sa.Column("actor_user_id", sa.BigInteger(), nullable=True),
            sa.Column("agent_run_id", sa.Uuid(as_uuid=True), nullable=True),
            sa.Column("import_batch_id", sa.Uuid(as_uuid=True), nullable=True),
            sa.Column("created_at", sa.DateTime(timezone=False), server_default=sa.func.now(), nullable=False),
        )
        op.create_index("idx_tx_annotation_events_tx", "transaction_annotation_events", ["transaction_id"])
        op.create_index("idx_tx_annotation_events_batch", "transaction_annotation_events", ["import_batch_id"])
        op.create_index("idx_tx_annotation_events_src", "transaction_annotation_events", ["source", "created_at"])

    # --- 5. Import batches --------------------------------------------------
    if not _has_table(inspector, "annotation_import_batches"):
        op.create_table(
            "annotation_import_batches",
            sa.Column("id", sa.Uuid(as_uuid=True), primary_key=True),
            sa.Column("file_name", sa.String(length=255), nullable=True),
            sa.Column("file_sha256", sa.String(length=64), nullable=False),
            sa.Column("source", sa.String(length=24), nullable=True),
            sa.Column("status", sa.String(length=16), nullable=False, server_default="pending"),
            sa.Column("row_count", sa.Integer(), server_default="0"),
            sa.Column("matched", sa.Integer(), server_default="0"),
            sa.Column("applied", sa.Integer(), server_default="0"),
            sa.Column("conflicts", sa.Integer(), server_default="0"),
            sa.Column("ambiguous", sa.Integer(), server_default="0"),
            sa.Column("unmatched", sa.Integer(), server_default="0"),
            sa.Column("report_json", sa.Text(), nullable=True),
            sa.Column("created_by_user_id", sa.BigInteger(), nullable=True),
            sa.Column("created_at", sa.DateTime(timezone=False), server_default=sa.func.now(), nullable=False),
            sa.Column("finished_at", sa.DateTime(timezone=False), nullable=True),
            sa.CheckConstraint(
                "status IN ('pending', 'dry_run', 'done', 'failed')",
                name="check_annotation_import_status",
            ),
        )
        op.create_index(
            "idx_annotation_import_batches_sha", "annotation_import_batches", ["file_sha256", "status"]
        )

    # --- 6. Seed the references (only when empty) ---------------------------
    articles_count = bind.execute(sa.text("SELECT COUNT(*) FROM annotation_articles")).scalar() or 0
    if articles_count == 0:
        op.bulk_insert(
            sa.table(
                "annotation_articles",
                sa.column("code", sa.String),
                sa.column("label", sa.String),
                sa.column("direction", sa.String),
                sa.column("sort_order", sa.Integer),
            ),
            [
                {"code": c, "label": lbl, "direction": d, "sort_order": o}
                for c, lbl, d, o in _ARTICLES
            ],
        )

    persons_count = bind.execute(sa.text("SELECT COUNT(*) FROM annotation_persons")).scalar() or 0
    if persons_count == 0:
        op.bulk_insert(
            sa.table(
                "annotation_persons",
                sa.column("code", sa.String),
                sa.column("sort_order", sa.Integer),
            ),
            [{"code": c, "sort_order": o} for c, o in _PERSONS],
        )

    # Note: annotation_period is deliberately NOT backfilled here. "period equals
    # the month of the operation" is a guess, and the client's own data shows the
    # two diverge. The rule pass fills it later with an explicit 'rule' source,
    # so a later import can still correct it.


def downgrade() -> None:
    bind = op.get_bind()
    inspector = Inspector.from_engine(bind)
    is_postgres = bind.dialect.name == "postgresql"

    for table in (
        "annotation_import_batches",
        "transaction_annotation_events",
        "annotation_rules",
        "annotation_persons",
        "annotation_articles",
    ):
        if _has_table(inspector, table):
            op.drop_table(table)

    if is_postgres:
        for index in (
            "idx_transactions_annotation_details_trgm",
            "idx_transactions_annotation_period",
            "idx_transactions_annotation_person",
            "idx_transactions_annotation_article",
        ):
            op.execute(f"DROP INDEX IF EXISTS {index}")
        for field in _SRC_FIELDS:
            op.execute(f"ALTER TABLE transactions DROP CONSTRAINT IF EXISTS check_{field}")

    inspector = Inspector.from_engine(bind)
    for name, _type in _ANNOTATION_COLUMNS:
        if _has_column(inspector, "transactions", name):
            op.drop_column("transactions", name)
