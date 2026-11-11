# Deployment: 12 November 2026 (Asia/Tashkent)

## Integrated source and artifacts

- Backend branch: b6419c6ffe557e7eeca5d626aaf9f1ce8c1e651b.
- UI branch imported from checksum-verified bundle: 83f5bddbbbf426065bfa02f0666e86141b8ab85b.
- Integration commit: b5bbc0d. Runtime backend image: parcer-backend:20261112-b5bbc0d, built over the previously running immutable image. Requirements were identical, so existing Python/TDLib dependencies were retained.
- Desktop version bumped to 2.0.1; Windows GNU release executable, embedded Telegram assets, WebView2Loader.dll, icon/version resource and Inno Setup installer built locally.
- Installer SHA-256: 85157fe5b4c6a6e740250af94ee01b8289b508dd9697ff85fd7594962e7b8377. The installer includes private system-access configuration and must not be published to an unprotected public download. Its credential was checked against the server configuration.

## Fresh verification

- Windows: cargo test -p tbs-core --locked: 116 unit tests passed.
- Windows: cargo test -p tbs-app --features devtools --locked: 264 passed. Live suites without their server configuration are not live coverage.
- cargo build -p tbs-app --release --locked succeeded (2.0.0 integration test build, then version-only rebuild as 2.0.1). The PE resource identifies TBS inform CRM 2.0.1.
- Backend: isolated container, no network/production mounts, SQLite fixtures: 1092 passed, 1 skipped (optional oletools VBA inspection unavailable). Test AUTH_DISABLE_LOGIN_2FA=false was explicit because the inherited live image defaults to true. Existing production authentication settings were retained, not changed by this deployment.
- Post-deploy smoke checks passed against live code: transaction response includes stored_transaction_type/excel_bank and preserves stored type; workbook builder generates a configured file changing only one ZIP member with byte-identical VBA, without DB writes; unauthenticated workbook request is denied with 403.
- Real Excel/VBA execution and interactive Windows login/table operation have not been verified by this deployment.

## Production rollout

- Host: 64.188.106.221; application: /opt/receipt-parser.
- Before/after transaction count: 1932, unchanged.
- Migration 20260923_0031 -> 20261111_0032 completed; adds nullable workbook-name column.
- Backend, three Celery workers and auth_bot switched to the new image. PostgreSQL, Redis, Caddy and neighboring funpay-bot were not recreated. Worker healthchecks pass; public HTTPS /health reports healthy/database connected.
- Live .env changes only: BACKEND_IMAGE selects the new image; EXCEL_SYNC_PUBLIC_URL=https://64.188.106.221.nip.io. APP_VERSION remained the pre-existing value, so health still reports 1.5.1; the image ID and migration identify this deployment.
- Source copied only under backend; production compose, Caddy routes, security configuration and persistent volumes were preserved.
- The desktop installer is saved locally under output/deployment/installer and on the server in the mode-700 release directory private-artifacts (file mode 600). Existing desktop installations require installing 2.0.1; no GitHub update release was published and no customer computer was automatically changed.

## Backup and rollback

- Root-only backup directory: /opt/parcer-backups/20261112-b5bbc0d.
- Contains initial DB dump, fresh pre-migration DB dump, globals SQL, original application/configuration archive, original container metadata, exact image overrides and rollback.sh. pg_restore --list successfully read the archive; this was not a full restore rehearsal.
- Original application image retained as parcer-backend:rollback-20261112-b5bbc0d.
- Initial DB/config backups downloaded to output/deployment on the owner's computer, outside git.
- Runtime rollback: bash /opt/parcer-backups/20261112-b5bbc0d/rollback.sh. It restores original source/.env and exact original application images, retains the compatible nullable DB column and DOES NOT restore the database (to avoid losing later operations).
- Full database restoration is a separately coordinated recovery operation, not an automatic rollback step.
- Unused build cache older than 24h was pruned before the build; current images, volumes and backups were preserved. Free disk after rollout was approximately 1.8 GB.

## Remaining behavioral difference

As documented in BACKEND_PROGRESS.md, the server export's Operation text/sort still uses inferred type/source bank, whereas the workbook and new Rust display prefer stored type/assigned bank for receipts without a bank headline. This semantic export change was not included automatically in the rollout.
