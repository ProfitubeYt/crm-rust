-- Read-only pre-checks for migrations 20261112_0034 and 20261112_0035.
--
-- Changes nothing: the session is read-only and every statement is a SELECT.
-- Meant for the SELECT-only role over the local tunnel described in
-- docs/handoff/SHARED_SSH_ACCESS.md, one connection:
--
--   PGPASSFILE=<private file> psql -X -w -h 127.0.0.1 -p 9990 -U hoplite_ro \
--       -d receipt_parser_db -f deploy/time-hub/prechecks.sql
--
-- A statement the role may not run fails on its own and the rest continue.
-- The output names accounts; keep it out of git.
--
-- The shared role reads only the non-secret columns of `users` (see the last
-- answer), so two things an administrator checks with full access instead,
-- inside the approved window:
--
--   SELECT id, username, totp_enabled, force_2fa FROM users WHERE lower(role) = 'admin';
--   SELECT count(*) AS accounts, count(telegram_id) AS with_telegram_id FROM users;
--
-- The first says whether a future CRM administrator still has to set up 2FA
-- (the panel sends them to the desktop sign-in for that); the second, how many
-- accounts already carry the Telegram ID the Time bot identifies people by.

\set ON_ERROR_STOP off
\pset pager off
SET default_transaction_read_only = on;
SET statement_timeout = '20s';

\echo '== who, where, read-only'
SELECT current_user, current_database(),
       current_setting('server_version') AS server_version,
       current_setting('default_transaction_read_only') AS read_only;

\echo '== schema revision (expected 20261112_0033 before the Time migrations)'
SELECT version_num FROM alembic_version;

\echo '== columns the migrations add: none may exist yet'
SELECT coalesce(string_agg(column_name, ', ' ORDER BY column_name), '(none)') AS already_present
FROM information_schema.columns
WHERE table_schema = 'public' AND table_name = 'users'
  AND column_name IN ('checks_access', 'time_role', 'global_admin');

\echo '== accounts by Checks role and state'
SELECT role, is_active, count(*) AS accounts
FROM users GROUP BY role, is_active ORDER BY role, is_active;

\echo '== 0035 marks exactly these accounts as CRM administrators (lower(role) = admin, active or not)'
SELECT id, username, display_name, is_active
FROM users WHERE lower(role) = 'admin' ORDER BY id;

\echo '== roles the code reads as admin but 0035 would not mark (stray spaces): expected none'
SELECT id, username, role FROM users
WHERE lower(btrim(role)) = 'admin' AND lower(role) <> 'admin' ORDER BY id;

\echo '== role values other than admin/operator: expected none'
SELECT role, count(*) AS accounts FROM users
WHERE lower(btrim(coalesce(role, ''))) NOT IN ('admin', 'operator') GROUP BY role ORDER BY role;

\echo '== what this role may read of users'
SELECT coalesce(string_agg(column_name, ', ' ORDER BY column_name), '(table-level or none)') AS readable_columns
FROM information_schema.column_privileges
WHERE grantee = current_user AND table_schema = 'public' AND table_name = 'users' AND privilege_type = 'SELECT';
