# Open tasks

Only open work; nothing below is implemented. Server state:
[docs/DEPLOYMENT.md](docs/DEPLOYMENT.md); structure: [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

For every item: Russian interface; local development on synthetic data; no
production change, Cloudflare provisioning or paid AI call without the owner's
approval and separately supplied credentials; no credentials, customer data or
access-bearing setups in git.

## Priority 1 — sign-in, activation, updates, storage

- **Authentication before the Hub** (owner-requested; first check what the current
  sign-in already does, the scope is to be confirmed with the owner). Recorded
  requirements: a login-first screen; after sign-in the Hub opens at once with
  only the authorized workspaces and no second login per workspace. The
  computer remembers the login, never a plaintext credential. Keep existing session
  persistence, expiry, sign-out and 2FA; existing accounts keep their passwords.
- **Single-use account activation.** A CRM administrator creates the account by
  login and assigns workspaces. First use takes login + an expiring single-use
  activation code, then the user sets and confirms a password. Codes are random,
  short-lived, stored hashed, rate-limited, revocable and atomically single-use
  (no two claimants). The server decides the flow; an unauthenticated lookup must
  not reveal an account's password state. Administrators regenerate codes and reset
  credentials without ever reading a password. Activation is separate from password
  reset. Extend the existing account system; no second one, no public
  self-registration. Grant changes and deactivation reach running clients and clear
  cached content that is no longer allowed.
- **GitHub updater and notifications** (owner-requested). The installed client
  learns about new releases from GitHub and tells the user. Today the client reads
  releases of `asintiko/parcer20-updates` and takes only
  `TBSparcer-Native-Setup-*.exe`; the design of the new source and of the
  notifications is still to be agreed. Keep the `tbsparcer` identity, the per-user
  install and saved settings; never publish a setup that contains
  `client-access.json`; verify a real 2.0.3 → new upgrade on Windows first.
- **R2 originals.** Private Cloudflare R2 bucket for the exact original text and
  attachments of receipts: content hashes, source identity, idempotent keys, an
  upload outbox that survives redelivery, crashes and outages. Retrieval is
  authorized by the caller's current workspace and data scopes and never hands out
  bucket credentials. `raw_message` has consumers (parsing, «Операция»,
  reprocessing): inventory them and migrate in phases; never blank it or delete a
  local original before a verified upload and a durable reference. Originals do not
  expire. Publish the retrieval contract for the AI work below.
- **R2 backups.** Weekly consistent PostgreSQL dump plus Time's SQLite and a
  generation manifest; checksums; an isolated restore test; keep the latest **two**
  validated generations and delete an older one only after a newer one is uploaded
  and verified. A failed run deletes nothing. Live PostgreSQL stays on the server.
- Check current Cloudflare limits and pricing before promising that indefinite
  retention fits a free tier. Propose bucket and credentials; the owner provisions.

## Priority 2 — finish the 2.1.0 client rollout

- Owner acceptance on production: real sign-in, Checks and Excel synchronization,
  intended Time grants (no production account was used in any test).
- Upgrade a designated Windows 2.0.3 installation to 2.1.0, confirm the session and
  settings survive, then decide on publishing to the update channel.
- Windows/WebView2 checks not yet made: a populated closed month, the dark theme,
  a permission revoked while the page is open, the bar's offline state.
- Time Telegram bot: new protected token, privacy mode off, verified negative group
  ID, identity bindings. The bot stays off until then; TBS Auth is separate.
- Server disk housekeeping (about 1.3 GB free on 2026-11-15), nothing deleted blindly.

## Priority 3 — shared Checks AI service and main chat

- «AI ассистент» in the Checks menu, for every Checks user within their existing
  permissions. One backend AI service and tool registry, shared with Priority 4:
  OpenAI-compatible, server-side credentials, streaming, cancellation, bounded
  context and cost. Provider, model and key come later: mock provider until then.
- Typed, parameterized domain tools only (search, aggregate, inspect evidence,
  propose reference/annotation changes, reconcile, prepare exports), each passing
  the existing role, scope and locked-period checks. No arbitrary SQL, shell,
  bucket access or administrator authority for the model.
- Data changes and bulk actions need explicit confirmation of the exact diff, bound
  to the user, an immutable proposal, the data version and an expiry; re-checked at
  execution; journaled with targeted undo. Viewing and filtering need none.
- Output as typed artifacts (table, chart, sanitized SVG); model HTML/JS/SVG and
  receipt text are untrusted input.
- Reconciliation against originals goes through the R2 retrieval contract, reports
  evidence and uncertainty, and never edits records by itself.

## Priority 4 — table AI copilot

- Compact mini-chat in the Checks transactions screen on the **same** AI service and
  approval path; no second engine, no key in the client.
- Semantic actions keyed by stable receipt/column/filter identifiers that call the
  same state transitions a person triggers. Never OS mouse/keyboard automation.
- A separate labelled in-app pointer that highlights the real target and shows
  success only after the state confirms it; it never moves the user's cursor. Stop
  and manual input win; re-check rows and scope after every async answer; handle
  DPI, virtualized rows, pinned columns and both themes.

## Smaller open items

- Document the export «Операция» semantics (inferred vs stored type, assigned-bank
  fallback) before changing exported content.
- Review findings deferred on 2026-11-14: the hosted page's message handler does not
  check the sender's origin; proxied bodies are buffered without a size cap (fix
  before Time accepts uploads); with `LAUNCH_GATE_ENABLED`, `/api/auth/refresh` is
  not exempt from the gate; count Time's five-second polling against
  `RATE_LIMIT_PER_MINUTE` per office address.
- Open a server-built workbook in desktop Excel and run its macros
  ([docs/EXCEL_SYNC.md](docs/EXCEL_SYNC.md)); not done as of 2026-11-11.
