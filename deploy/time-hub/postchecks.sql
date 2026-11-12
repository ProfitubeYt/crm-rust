-- Read-only checks after migrations 20261112_0034 and 20261112_0035.
--
-- Changes nothing: the session is read-only and every statement is a SELECT.
-- Run the same way as prechecks.sql. Each answer says what it must be.

\set ON_ERROR_STOP off
\pset pager off
SET default_transaction_read_only = on;
SET statement_timeout = '20s';

\echo '== schema revision: 20261112_0035'
SELECT version_num FROM alembic_version;

\echo '== the three columns: checks_access boolean NOT NULL default true, global_admin boolean NOT NULL default false, time_role varchar(20) NULL'
SELECT column_name, data_type, character_maximum_length AS length, is_nullable, column_default
FROM information_schema.columns
WHERE table_schema = 'public' AND table_name = 'users'
  AND column_name IN ('checks_access', 'time_role', 'global_admin')
ORDER BY column_name;

\echo '== the Time role check: one row'
SELECT conname FROM pg_constraint
WHERE conrelid = 'public.users'::regclass AND conname = 'check_user_time_role';

\echo '== nobody lost Checks, nobody got Time: both 0'
SELECT count(*) FILTER (WHERE NOT checks_access) AS without_checks,
       count(*) FILTER (WHERE time_role IS NOT NULL) AS with_time
FROM users;

\echo '== CRM administrators are exactly the Checks administrators: both 0'
SELECT count(*) FILTER (WHERE global_admin AND lower(role) <> 'admin') AS admin_without_the_role,
       count(*) FILTER (WHERE NOT global_admin AND lower(role) = 'admin') AS role_without_admin
FROM users;

\echo '== the CRM administrators by name: the same list prechecks.sql printed'
SELECT id, username, display_name, is_active
FROM users WHERE global_admin ORDER BY id;

\echo '== at least one active CRM administrator: true'
SELECT count(*) > 0 AS has_active_admin FROM users WHERE global_admin AND is_active;
