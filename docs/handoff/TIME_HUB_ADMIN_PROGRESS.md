> Continuation handoff: this file preserves the previous session's evidence. Read TIME_RESUME_STATUS.md and docs/tasks/RESUME_TIME_HUB_ADMIN_PROMPT.md for the current merged 2.0.3 baseline and unfinished work.

# CRM Hub: accounts, global administration, workspaces, Time bot — progress

First branch `work/time-hub-admin` from `main` `45d7e5d`; plan: `docs/handoff/TIME_HUB_ADMIN_PLAN.md`.
The newest session is first. Nothing here is deployed.

## Release session — 2026-11-14, branch `vorflux/time-release` from `main` `21b3ebf`

The newest section; everything under it is history. Pull request:
<https://github.com/asintiko/crm-rust/pull/1> (draft). **Nothing is deployed. Production was
neither read nor changed in this session**: the sandbox has no key for the server, so the
last production read is still the read-only pre-check of 2026-11-12 13:30 UTC.

Platform of everything below: a Linux sandbox (x86-64, Docker 29.1.4, Compose v5.0.1), the
image `tbs-linux-dev:pinned` (Rust 1.99.1, MinGW) for the client, `timelist:local` rebuilt
from `time/`, Playwright 1.49.1 with Chromium 131 for the page. One thing ran on Windows: the
build, the tests and a first start on a hosted runner of GitHub Actions (below). **Nothing
opened a hosted page in WebView2.**

### Commits

| Commit | What |
|---|---|
| `0efe423` | The program's slim top bar for «Учёт времени» and «Администрирование CRM», no rail there; Checks keeps its rail. `build.rs` embeds resources only for a Windows target and finds the cross `windres`. The e2e run's output as evidence. |
| `e6d3940` | The same code with what the rail no longer needs taken out (no behaviour change). |
| `a29f2ac` | Client version 2.0.3 → 2.1.0 (`desktop/Cargo.toml`, `Cargo.lock`). |
| `9861953` | `.github/workflows/windows-client.yml`: build, test, setup, silent install and first start on a hosted Windows runner. Started by hand only, and offered by GitHub only once the file is on the default branch. |
| `887214b`, then removed | A temporary push trigger, with the owner's word, to run that workflow once from this branch. |
| `6624799` | The deployment document reworked: the owner's decisions, readable backups, both administrators, the bot. |
| after it | This record, `OPEN_TASKS.md`, the continuation prompt, `desktop/README.md`, the window pictures. |

### The top bar

`desktop/crates/tbs-app/src/app.rs`: `App::lay_out` gives a page `ctx.rail = 0` and
`ctx.bar = BAR` (40 px and a 1 px rule), and Checks its rail and `ctx.bar = 0`; `App::bar()`
is the home button «Главный экран», the workspace's name, the connection mark, the account
and «Выйти»; `view()` puts the bar over a page and the rail beside Checks.
`screens/hosted/mod.rs` gives the web view what the shell leaves: `x = ctx.rail`,
`y = ctx.bar`. The Time page was not changed: it had no hamburger of its own.

### Verified, with the actual output

| Check | Tree | Result |
|---|---|---|
| `scripts/hub/desktop-tests.sh test --workspace --locked` | `e6d3940` + 2.1.0 | 351 app + 141 core + 1 + 1 passed, 0 failed, exit 0 |
| `… check -p tbs-app --locked --target x86_64-pc-windows-gnu` | same | no error, no warning, exit 0. A type check, not a build |
| `cargo test --workspace --locked` in `time/` | `0efe423` (`time/` as on `main`) | 17 + 28 + 1 passed, exit 0 |
| `scripts/hub/backend-tests.sh` | same (`backend/` as on `main`) | 1349 passed, 1 skipped, exit 0 |
| `scripts/hub/e2e/run.sh` (rebuilt `timelist:local` healthy in Hub mode, rehearsal of 0034/0035, downgrade, upgrade, release 2.0.3 on the new schema) | same | 173 checks, 0 failed, exit 0. Output: `docs/handoff/evidence/time-hub-e2e-run-20261114.txt` |
| `run.sh page` (`page_shots.py`, Chromium 131 headless through the stand-in for the program) | same | 74 checks, 0 failed |
| The pictures of that run | same | looked at: manager 1024 light and dark, viewer 1024, viewer 1360 dark, closed month for both, standalone 1024 light and dark. No overlap, no cut text, no missing control |
| The client's real window: debug build 2.1.0, Xvfb, WebKitGTK, against the e2e stack | `0efe423` + the edits later committed as `e6d3940`, `a29f2ac` | all seven checks below passed |
| Windows release build, cross-linked: `cargo build -p tbs-app --release --locked --target x86_64-pc-windows-gnu` | `6624799` | exit 0 |
| Setup compiled by Inno Setup 6 under Wine (`amake/innosetup`) | `6624799` | "Successful compile" |

The window run (pictures and their list: `docs/handoff/evidence/time-hub-window-20261114/`):

1. Time as manager and as viewer, light and dark, 1360 and 1024 × 640: the bar, no rail and no
   hamburger, the page fills the width under the bar and nothing of it is under the bar.
2. CRM administration as an administrator: the same bar, the list of 8 made-up accounts.
3. The home button returns to the start window with the session kept. Checks has its rail and
   no bar; a folded rail is still folded after Hub → Time → Hub → Checks.
4. «Выйти» in the bar returns to sign-in; the next account sees only its own entries and page.
5. `grant.py e2e_both time=-` while Time was open: within five seconds the start window with
   «Доступ к разделу «Учёт времени» закрыт администратором», only Checks offered.
6. The hints of the home button and of the connection mark open sideways inside the bar and
   are not covered by the web view (on WebKitGTK).
7. Resizing 1360 → 1024 × 640 → 1200 × 760 with the page open: the web view follows.

Not covered by the window run: the bar's offline state, a long account name, a page that is
loading or failed, revocation of CRM administration, light manager and dark viewer at 1024.
The bar's heading is drawn with a stand-in for Segoe UI Semibold on Linux.

The persistent SQLite storage of the Time image was not checked separately: the stack runs
Time on a volume, and nothing restarted the container to read the file back.

### The Windows build: what exists and what it is not

**Built and started on Windows by GitHub Actions.** Run <https://github.com/asintiko/crm-rust/actions/runs/43598951623>, commit `887214b` (the
sources of `081e9e4`), hosted runner `windows-2022` image 20261112.366.7: Windows Server 2022
Datacenter 10.0.20348, WebView2 runtime 131.0.2903.86, rustc 1.99.1 with host
`x86_64-pc-windows-gnu`, MinGW in `C:\mingw64`. 43 minutes, every step passed. Lines of its
log: `docs/handoff/evidence/time-hub-windows-run-20261114.txt`.

| Step | Result |
|---|---|
| `cargo test --workspace --locked` | 351 app + 139 core + 1 + 1 passed, 0 failed. Two core tests fewer than on Linux: `secret::tests` is compiled `not(windows)` |
| `cargo build -p tbs-app --release --locked` | 14m 59s. `tbsparcer.exe` 28297728 bytes, product «TBS inform CRM», file and product version 2.1.0; `WebView2Loader.dll` 160880 bytes |
| Setup by Inno Setup 6, without `client-access.json` | `TBSparcer-Native-Setup-2.1.0.exe`, 9797338 bytes, SHA-256 `234848f697dc59c6aa7a02895fb435cd7bb39d224d37b3c3880e91d2990bbb2b` |
| Silent install for the current user to `C:\tbs-installed` | `tbsparcer.exe`, `WebView2Loader.dll`, the uninstaller; no `security` folder |
| One start of the installed program | alive after 25 seconds, window «TBS inform CRM». Picture `W1-windows-runner-first-start-2.1.0.png`: the sign-in window, «Версия 2.1.0», «Сервер недоступен» |

What that run does **not** show:

- Nobody signed in, so **no hosted page was opened and WebView2 did not run**: the bar over
  Time and CRM administration, the first sizing, the hints over a WebView2 view, sign-out and
  user switch, the click on «Открыть» are all still unseen on Windows.
- «Сервер недоступен» was not looked into: the setup has no `client-access.json` and the
  runner is not in the office's network.
- An update over an installed 2.0.3 was not tried: the runner had no client.
- Windows Server 2022 on a hosted runner is not a staff desktop with Windows 10 or 11.

The setup is kept by GitHub as the artifact `windows-client` of that run until 2026-11-21. It
is not committed and **not published** to `asintiko/parcer20-updates`: every installed client
would be offered it. It has no `client-access.json` (the sandbox and the runner do not have
it, and it must not go through a chat), so it can update an installed client, which keeps its
own file, and is not enough for a first installation. Publish only after somebody has signed
in with it on a Windows desktop and opened both pages, with the owner's word. For a setup
with the file: `pwsh desktop/tools/build-installer.ps1` on the owner's machine.

Before that run the same sources were cross-linked in the sandbox (`cargo build -p tbs-app
--release --locked --target x86_64-pc-windows-gnu`, then `amake/innosetup` under Wine):
`tbsparcer.exe` 28173312 bytes, a setup of 9881272 bytes with SHA-256
`5387fe32…e93044`. Those files were never started and are replaced by the run above.

### Seen and not changed

- **A click that opens a page also lands in the page** (pictures 17 and 24). A click on
  «Открыть» in the start window selects the cell, or opens the drawer of the account, under
  the same point of the page that opens. Seen with `xdotool` on WebKitGTK; not known for a
  real mouse or for WebView2, and not compared with an earlier commit. It selects, it does
  not write. To check on Windows first.
- In a closed month the empty day panel still says «Enter или ввод с клавиатуры — правка,
  Delete — очистить» (picture B4). Whether the cells refuse an edit there was not tried.
- The CRM administration page shows the account's name in its own header as well as in the
  bar (picture 18). The page is unchanged.

### The review findings left open: what was done with each

| Finding | State |
|---|---|
| Time page header wider than the page at 1360 with Linux fonts | Gone with the rail: at 1360 and at 1024 the header fits (pictures 01, 08, 14). Windows fonts not checked |
| Three other comparisons of a naive `utcnow()` with a `timezone=True` lock column (`api/dependencies.py:811`, `api/routes/security.py:892, 943`, `api/routes/telegram_client.py:448, 1245`: the launch password and a chat's password) | Deferred. The same code runs in production as 2.0.3 and is outside this change; while a lock holds, the answer may be 500 instead of "locked", and access stays closed. To be corrected with its own tests on PostgreSQL |
| The page's message handler does not check the sender's address | Deferred: it can only be verified on WebView2. Navigation is held to one document and the page's CSP blocks frames |
| The sign-in form does not number its attempts | Deferred: same person and same credentials; the cross-account case is closed in the API client |
| The Time page does not post `access-lost` for the plain-text `inactive_user` | Deferred: the shell handles that refusal itself; a change of the page would need the page run again |
| Proxied bodies are buffered whole without a size cap | Deferred: the gateway serves signed-in staff only and Time's own answers are small. To do before Time takes uploads |
| With `LAUNCH_GATE_ENABLED`, `/api/auth/refresh` is not exempt from the gate | Deferred: older behaviour of 2.0.3, the gate is a deployment setting; not part of this change |
| `RATE_LIMIT_PER_MINUTE` against a page reading `/api/time/meta` every 5 seconds | Deferred as a deployment check: 12 requests a minute for each open page; count the users behind one office address against the server's limit before the rollout |

### Deployment: where it stands

`docs/handoff/TIME_HUB_ADMIN_DEPLOYMENT.md` is the plan to hand to the executor. Its new
commands (the administrators' list with a fingerprint of the sign-in data, the backup read
back with `pg_restore`, the comparison after the migration) were tried on the local stack's
PostgreSQL 15 only. Still missing before anyone may start: an executor with a root shell and
a window; the live read-only pre-check and the backups on the server; the 2FA state of
`admin` and `stlee`; a reissued bot token, privacy mode off and the group's verified negative
ID (the bot may stay off for the first rollout); a sign-in and both pages tried in the client on
a Windows desktop.

## UI and release session — 2026-11-12, second part of the work on `work/time-hub-admin-finish`, cut short by credits

Historical: its "Not done" list and its design sketch are answered by the Release session
above. The owner's decisions in it stand.
**Nothing is deployed. No production write, migration or deployment has been made at any
time.** The last production read is the read-only pre-check of 13:30 UTC in the Finish
session's table below. Commits of this part: `fa69c1b` (the Time page, WIP) and the handoff
commits after it, on top of `0664f66`.

### The owner's latest decisions (the owner's handoff request, 2026-11-12)

- Production deployment is approved after the Time UI adaptation, preflight checks and
  verified backups. Not executed.
- Both `admin` and `stlee` may retain global CRM administration.
- Remove the inner Time hamburger and adapt the embedded layout. Asked how, the owner chose
  a slim top bar of the program instead of the left column for Time and CRM administration;
  Checks keeps its rail.
- The bot token is supplied separately as a protected secret (the owner's file
  `C:/Users/heper/.config/parcer-secrets/time-bot.env`; the server reads `TIME_BOT_TOKEN`).
  Never commit or print it.
- Bot `@tbstimebot` was valid, but privacy mode was still enabled at the last check. Owner's
  step: BotFather `/setprivacy` → `@tbstimebot` → Disable, then take the real negative group
  ID from a group update.
- Owner's Telegram ID: `1957390574`.
- Supplied group ID `1276772106` was NOT verified: Telegram returned "chat not found". Its
  prefix is not to be guessed and arbitrary groups are not to be accepted. Attendance
  polling stays off until a verified ID exists.
- Shared SSH access (`hoplite-ro`) is read-only, not deployment access.
- From earlier in the conversation: find out through an authorised mechanism whether both
  administrators have 2FA, without revealing secrets; keep enrollment and recovery working;
  nobody may be locked out.

Changes to these facts verified since: **none.** The agent queried neither the bot nor the
group. One thing to act on: the owner's later handoff message exposed the bot token in chat.
The agent did not use or repeat it; a scan by format finds no token-shaped string in the
committed tree or in the sandbox's files. Treat that token as exposed and reissue it.
Assumption to confirm with the owner: Time and the panel may go live before the bot works.

### Done

Time page, `time/crates/server/assets/` (`index.html`, `app.css`, `app.js`), commit `fa69c1b`:

- In the program's window (`window.TIMELIST_HUB`, class `hub` on `body`) the page hides its
  own name, account button and sign-out. By itself in a browser it keeps them.
- One header line of 44 px: the six sections as labelled tabs, the period, the read-only and
  closed-month marks, the bot's state. Tabs never become a menu: they move to a line of their
  own under 940 px in the program and under 1240 px in a browser. Under 1100 px the line is
  tighter and the two marks use short wording.
- The day panel's width follows the window (280–348 px). The absence button reads
  «+ Отсутствие».
- A viewer (`meta.hub.can_edit` false): the mark «Только просмотр»; only controls carrying
  `data-ro` stay usable, the rest is hidden or disabled, also for what is drawn later
  (`applyReadonly`, a `MutationObserver`); `api()` sends nothing but reads and `/parse`.
  The server's authorisation is unchanged.
- **The Time page has no hamburger of its own and had none at `0664f66`.** The hamburger on
  the Time screen is the desktop program's rail toggle: see "Not done".

Tooling under `scripts/hub/e2e/`: `page_host.py` (serves the page to an ordinary browser the
way the program serves it to its web view: manager on 8101, viewer on 8102), `page_shots.py`
(`--out`, `--standalone`, `--only sizes|viewer|manager|pictures`) with `page-shots.Dockerfile`
(Playwright 1.49.1), `run.sh page`, and in `compose.yml` the `pagehost` service and
`RATE_LIMIT_PER_MINUTE: ${E2E_RATE_LIMIT:-200}`.

### Verified, and only this

- Final run, 15:27–15:28 UTC, **Chromium 131.0.6778.33, headless, Linux, through the stand-in
  for the program** (`page_host.py`): **74 checks, 0 failed**, exit 0. Log:
  `output/hub-e2e/logs/page-all.log` (sandbox only). The page files were last written before
  the run and committed unchanged as `fa69c1b`, so the run tested the committed files.
  It covers: header and toolbar by window width for manager and viewer; what a viewer is
  offered, section by section; that a manager still edits; pictures.
- It is not the desktop client's window, not WebKitGTK, not Windows and not WebView2.
- 30 pictures of that run are in `output/hub-e2e/page/` (sandbox only): manager, viewer,
  standalone, both themes, closed month. **Nobody looked at them after the final run**; the
  result above is measurements only. The standalone page has pictures only, no measurements.
- `node --check time/crates/server/assets/app.js` passes.

### Not done, not verified

- **The program's slim top bar is not written.** The desktop shell is unchanged, so on the
  Time and CRM administration screens the left rail and its hamburger are still there. This
  is the main unfinished part of the owner's request. The design is below.
- The new page was not opened in the desktop client's window and the client was not rebuilt.
- **No suite was rerun after `fa69c1b`**: not the desktop suite (its `hosted/assets.rs` tests
  read the page's files), not the Windows type check, not Time's own tests, not the backend
  suite, not `scripts/hub/e2e/run.sh`. The figures in the Finish session's table are for the
  tree before it.
- The Time image was not rebuilt with the new page. No version bump, no native Windows build,
  no installer.
- No production preflight after 13:30 UTC, no backup taken, no deployment, no migration.
- The deployment document is the plan of the Finish session with the approval noted in its
  section 10; the bot, the 2FA query and the hand-over to an administrator are not worked in.

### The design for the program's top bar (a sketch: nothing of it is in the code or compiled)

Every identifier below exists in `desktop/crates/tbs-app/src/` at this commit (checked by
search, not by a compiler).

- `ctx.rs`: add `pub bar: f32` beside `pub rail: f32`.
- `app.rs`: a constant for the bar's height (40 and a 1 px rule). A helper that sets, for a
  workspace that is a page (`self.workspace != Subsystem::Receipts`), `ctx.rail = 0.0` and
  `ctx.bar` to that height, and for Checks `ctx.rail = rail_width(prefs.rail_collapsed)` and
  `ctx.bar = 0.0`. Call it in `enter_main()` and for `Message::ToggleRail`.
- `app.rs`, a new `bar()` view: a row styled like the rail with `ui::btn_icon(Icon::Home20, …,
  ui::Kind::Ghost, Some(Message::Hub))` under `ui::hint_on(…, HUB_HINT)`, `ui::divider_v()`,
  `ui::heading(self.workspace.name())`, `ui::fill()`, the online mark and account as in
  `rail()`, and a sign-out button sending `Message::Logout`; `ui::divider()` under it.
- `app.rs`, `Phase::Main`: the bar above the page for a page workspace, the rail beside it
  for Checks.
- `screens/hosted/mod.rs`, `page_area` (now `x: ctx.rail, y: 0.0`): `y: ctx.bar` and
  `height: ctx.window.height - ctx.bar`.
- Tests in `app.rs` to change: `time_opens_as_a_page…` and
  `administration_opens_for_an_administrator…`; the rail tests of Checks stay.
- `WORK_MIN_WINDOW` is 1024 × 640 and `RAIL_WIDTH` 208, so the page gains the rail's width
  and loses about 41 px of height: at least 1024 × 599.

### Found on the way, not changed

- The backend limits requests per address (`RATE_LIMIT_PER_MINUTE`, 200 in the stack) and in
  the program's window the Time page reads `/api/time/meta` every 5 seconds: 12 requests a
  minute for each open page. Many pages behind one office address could reach the limit.
- In this environment tool output over about 2,000 tokens is replaced by a summary: keep
  outputs short.
- `.hoplite/` (chat attachments, the SSH key among them) is not ignored by git: never
  `git add -A`.

## Finish session — 2026-11-12, branch `work/time-hub-admin-finish`

Baseline: `main` at `4e644ebc29ac505b937b30966130e58920a51d07` (everything through `10f7ea8`
plus the handoff correction). **Nothing is deployed.** Production was read once more with the
SELECT-only role and not changed. The sections below this one are earlier sessions' records.
The plan put to the owner for approval is `docs/handoff/TIME_HUB_ADMIN_DEPLOYMENT.md`.

Sandbox: 2 CPUs, 8 GB, Docker 29.1.3. Rebuilt here: `tbs-linux-dev:pinned`, `timelist:local`
(about six minutes), the volume `hub-backend-venv`.

| Commit | What it holds |
|---|---|
| `f4f6e43` | the e2e scripts, first run of the stack and the rehearsal, the lockout fix and its test |
| `5284b5d` | the window run and the Linux web view fixes, sessions carried through the rehearsal, the recorded run |
| the commit with this text | the deployment document, the 2.0.3 client check, the continuation prompt |

### What was run, and what came out (UTC)

| Step | Result |
|---|---|
| Desktop suite through `scripts/hub/desktop-tests.sh` (the wrapper itself, never run before), 13:50, and again at 14:26 after the changes for the window run | **348 app tests and 141 core tests passed** both times, exit 0. `live_backend` and `live_writes` report one passed test each by returning early; they are not coverage |
| Windows code paths: `scripts/hub/desktop-tests.sh check -p tbs-app --locked --target x86_64-pc-windows-gnu`, 14:00, and again at 14:26 after the changes | `Finished`, exit 0 both times. (The very first run, six minutes forty, lost the Docker client's wait stream and reported 125 although the container exited 0; it was run again.) |
| Production pre-checks, `deploy/time-hub/prechecks.sql`, read-only, one connection, 13:30 | unchanged: PostgreSQL 15.16, schema `20261112_0033`, none of the three columns, two active `admin` accounts and one active `operator`, no other role values |
| Whole backend suite, `scripts/hub/backend-tests.sh`, 14:02–14:05, on the tree with the lockout fix; no backend file changed after it | **1349 passed, 1 skipped**, 193 s |
| Local stack, everything in one command, `scripts/hub/e2e/run.sh`, 14:26–14:30, PostgreSQL 15.19 | migration rehearsal: every step passed (9 + 3 + 13 + 9 checks); end-to-end: **173 checks, 0 failed**; exit 0. Output: `docs/handoff/evidence/time-hub-e2e-run-20261112.txt` (Docker's progress lines removed) |
| A real window, Linux WebKitGTK under Xvfb, against that stack, 14:08–14:25, and the 2.0.3 client at 14:40 | Time and the admin panel opened from the Hub; revocation and deactivation while open; sign-out; three accounts in one run; the 2.0.3 client on the new backend. Two faults of the Linux build found and fixed. See below |

Limits of the Windows result: it is a type check of the `cfg(windows)` code against the
`x86_64-pc-windows-gnu` target on Linux. Nothing was linked, no `.exe` or installer was made,
nothing ran on Windows or with WebView2. `build.rs` calls `windres` under its Windows name,
which the Linux image does not have, so the icon and version resource were skipped with the
warning `windres failed, the executable has no icon`.

### Migration rehearsal (`run.sh rehearse`)

The database is built by the code of `f5eede4` (`git archive`, `create_all`, `pg_trgm`) with
four made-up accounts shaped like the server's (two active administrators, one operator, plus
one disabled administrator) and stamped `20261112_0033`. Then, in order:

1. `prechecks.sql`: revision 0033, none of the columns.
2. Release 2.0.3 on its own schema: sign-in, `/api/auth/me` without any Hub field, receipts,
   account administration. 9 checks. The administrator's session is kept for later steps.
3. `alembic upgrade head`: `0033 -> 0034 -> 0035`.
4. `postchecks.sql`: three columns with the stated types and defaults, the check constraint,
   nobody without Checks, nobody with Time, CRM administrators exactly the three `admin` rows
   (the disabled one included), one active administrator at least.
5. The current code on the new schema: **the session opened under 2.0.3 in step 2 goes on
   working**, is told it is a CRM administrator with Checks and administers accounts without
   signing in again. A session opened here is kept for the next step.
6. Release 2.0.3 on the new schema, which is what a rollback of the image alone runs on: the
   9 checks; **the session opened under the current code goes on and its refresh token is
   exchanged**; two accounts are created through the 2.0.3 API. **The administrator it
   created has `global_admin = false`**: the old code does not know the column. A demotion
   made by the old code likewise leaves `global_admin = true`. After a rollback and a second
   roll-forward the two have to be compared before anyone is let back in; the deployment
   document carries the query.
7. `alembic downgrade 20261112_0033`: both steps down, `prechecks.sql` shows 0033 and no column.
8. Release 2.0.3 on the downgraded schema: 9 checks.
9. `alembic upgrade head` again, `postchecks.sql`: as in 4, now with four administrators (the
   one 2.0.3 created has the role, so 0035 marks it).

So neither the deployment nor a rollback of the image signs anyone out: sessions live in
Redis, which neither touches, and each code reads the other's tokens.

### End-to-end checks (`run.sh start`, `run.sh checks`)

The stack: the current backend from the pinned Python image on PostgreSQL 15 and Redis 7, the
real Time server from `timelist:local` under `deploy/time-hub/docker-compose.time.yml`
unchanged, Caddy 2 with `deploy/Caddyfile` (only the site address replaced by `:80`), and
`fake_telegram.py` in place of Telegram. The network is internal; Caddy alone publishes
`127.0.0.1:18080`. `timelist` reports healthy, so its `HEALTHCHECK` does pass in Hub mode.

What the 173 checks cover, all through the running services:

- Caddy and the gates: `/health`; the system-access gate with and without Caddy; `/admin`
  308, `/admin/` with its policy, nothing but the three files; `/internal/time/telegram/{id}`
  not routed through Caddy, refused without the key, with a wrong key and with a proxy's
  header, answered from inside with the key; Time itself refuses without the gateway's token
  and has its password sign-in switched off.
- A session opened under 2.0.3 before the migration goes on through Caddy and its refresh
  token is exchanged.
- Accounts from before the migration: the administrator is a CRM administrator with Checks,
  the operator is neither an administrator nor in Time, the disabled administrator cannot
  sign in, nobody has Time.
- Eight accounts created through `/api/admin/users` (Checks only, Time viewer, Time manager,
  both, neither, CRM administrator without a workspace, Time employee, disabled): the
  `/api/auth/me` fields of each, and for each the answers of the Checks list, the Checks
  export, the Excel grants, Time's page data, a Time edit, the Time workbook and CRM
  administration.
- Time through the gateway: the journal names the account and not what the client claimed;
  a closed month refuses edits (409) and stays as it was when an absence is entered over it.
- Revocation and grant on a live token, for Time, Checks and CRM administration; an
  administrator cannot demote or disable themselves.
- Sessions: listing, refresh exchange, a used refresh token refused, an expired and a forged
  access token, sign-out, an administrator ending an account's sessions, deactivation.
- The bot, with synthetic senders and groups: an employee account's message is kept (🤷 until
  the sender is bound, ✍ after); an unknown sender, an account that lost Time and a disabled
  account get 🤷 and nothing is kept; another group is ignored; `/id` and `/me` answer.
- Sign-in protection: lockout after five wrong passwords and unlock; a second factor set up,
  confirmed and required; the administrators' second-factor policy reaches a CRM
  administrator with no workspace; the limit of ten sign-ins a minute answered 429 twice and
  was waited out.

Not covered by them: a real Telegram, TLS (Caddy listens on plain `:80` here), the `frontend`
container (absent, so anything Caddy sends there answers 502), TDLib and the Celery workers.

Also seen on this stack: Time started without a bot token is healthy and says in its log
that the bot is off and only the interface works, so Time can go up before the bot exists.
It held 2 MiB without the bot and 6 MiB with it and the checks' data; its limit is 256 MiB.

### Found by the end-to-end run and fixed

**A locked account answered 500 at its next sign-in.** `users.locked_until` is a timestamp
with a time zone on PostgreSQL, `services/user_service.authenticate` compared it with a naive
`utcnow()`, and Python refuses that comparison. On SQLite, where the unit tests run, the value
comes back naive and the comparison works. The code is the same in `f5eede4`, and the
server's column was created `timezone=True` by migration `20260216_0004`, so release 2.0.3 as
deployed should have it too: after five wrong passwords an account answers 500 to every
sign-in, right password or not, lock run out or not, until an administrator unlocks it (or
the column is cleared in the database). That was not tried against production. Fixed in
`backend/services/user_service.py` (the stored value is read as naive UTC before comparing),
with `backend/tests/test_login_lockout.py`, whose first test fails without the fix; the
end-to-end check then passed.

The same kind of comparison, a naive `utcnow()` against a column declared `timezone=True`, is
in `api/dependencies.py:811` and `api/routes/security.py` (launch password lock) and in
`api/routes/telegram_client.py` (chat password lock). They are outside this work, were not
run here and were not changed.

Two things in the checks themselves were wrong at first and are worth knowing: Time does not
refuse an absence over a closed month, it freezes the month (by design, `locked_month_is_frozen`);
and `/api/auth/logout` also ends the session of the refresh cookie the request carries, so one
HTTP client object standing for several people must not keep cookies.

### A real window (`desktop/tools/window-check.py`)

**This is the Linux build with WebKitGTK on an Xvfb screen, not Windows and not WebView2.**
The client was built with `cargo build -p tbs-app --locked` in `tbs-linux-dev:pinned` plus
`xdotool gnome-keyring imagemagick dbus-x11 socat`, and driven with the mouse and keyboard
against the stack above. The client takes plain `http` only on loopback, so inside its
container `socat` forwarded `127.0.0.1:18080` to Caddy and `SERVER=http://127.0.0.1:18080`
(new in `window-check.py`) pointed the client there instead of the stub.

What was seen, in one run of the program with three accounts
(`docs/handoff/evidence/time-hub-window-20261112/`, cropped to the window):

- Launch and sign-in land on the start window (the Hub); a saved session comes back to it.
  `e2e_both` (Checks operator, Time manager) is offered «Чеки» and «Учёт времени» and no
  administration.
- «Учёт времени» opens the Time page in the work window: the month with the employee and the
  marks the stand-in bot produced, the bot's name in the header; a click on a day opens its
  panel with the mark, the calculation and the Telegram messages. So the page's files, its
  requests through the shell to `/api/time` and its input all work in a real web view.
- Time taken away while its page was open (`scripts/hub/e2e/grant.py e2e_both time=-`):
  within 36 seconds the page was closed, the start window was back and listed «Чеки» only;
  the page's web folder was gone.
- `e2e_crm`, a CRM administrator with no workspace, is offered «Администрирование CRM» alone;
  it opens the panel with all fourteen accounts, their access and state, signed in by the
  shell's session, no sign-in form.
- CRM administration taken away while the panel was open: the panel was closed and the start
  window showed «Нет доступных пространств» with «Доступ выдаёт администратор CRM».
- Sign-out, then `e2e_viewer` (Time viewer): only «Учёт времени»; the page opens. The account
  disabled while the page was open: page closed, sign-in form.
- `e2e_both` again: Checks, start window, Time, start window, Time. No page of an earlier
  account was shown to a later one.
- **The release 2.0.3 client against the new backend**, which is what every installed client
  will be on the day of the deployment: built from `f5eede4`, signed in as an administrator
  from before the migration on a stack migrated and not otherwise touched. Receipts, the
  settings and Settings → Пользователи (the four accounts, through `/api/admin/users`) work
  as before (`06-release-2.0.3-client-on-the-new-backend.png`).

Found and fixed, both in the Linux build only (`cfg(target_os = "linux")`; Windows code is
the same as before, and the suite and the Windows check were run again after it):

- **Any hosted page ended the program**: `GTK has not been initialized`. wry's Linux web view
  is WebKitGTK, and winit neither starts GTK nor runs its loop. `hosted/host.rs` now starts
  GTK before the first view and gives GTK its turn every 30 ms while a hosted page exists
  (`Msg::Pump`); `gtk` is a Linux-only dependency of `tbs-app` (one line in `Cargo.lock`,
  the version wry already used). The Telegram section's web view has the same gap on Linux
  and was not touched.
- **The second page ended the program** with `BadWindow`: the app swaps the work window for
  the start window, and GTK took the closed view's X window down only later, after its parent
  was gone. A view is now let go with GTK's teardown done at once (`release`).

Seen and left as they are:

- **A viewer sees the editing controls.** The Time page sets `body.readonly` for a viewer
  (`app.js`) and no rule in `app.css` uses it; the only sign is «· просмотр» next to the
  name. The server refuses every write (403, checked above) and the page then says
  «Недостаточно прав: … только просмотр», so nothing can be changed; it is a matter of what
  the page offers. Not changed in this session.
- At the default 1360-pixel window the Time page's header is wider than the page area with
  this image's fonts (DejaVu, Noto): the bot's name and the account's name on the right are
  cut. In a wider window they fit. Not checked with Windows fonts.
- After one of three closings the page's web folder was there again with WebKitGTK's own
  bookkeeping in it (`resourcemonitorthrottler`, an empty `storage`), written as the web view
  shut down; nothing of the page. How WebView2 behaves there is not known from this run.
- Xvfb's default 100 dots an inch made wry size a page's first frame 4% too large until the
  window was resized; `window-check.py` now starts Xvfb with `-dpi 96`.
- Not tried in the window: the Time workbook's save dialog (no desktop portal in the image),
  the launch password, the new client's own settings (accounts, sessions), the Telegram section.

### The deployment document

`docs/handoff/TIME_HUB_ADMIN_DEPLOYMENT.md`: what changes, what the owner supplies, the
steps with the answer each must give, health checks, rollback by layer. Its commands were
tried where a local stand-in exists: the health checks and SQL statements against the stack,
`create_admin.py --promote` on PostgreSQL, the image `save`/`load` (39 MB), `pg_dump` and
`pg_restore --list`, the release archive and the overlay build on a stand-in image holding
the 2.0.3 code, and the Caddy step on a bind-mounted file. Two corrections came out of
that: the panel answers `GET` and not `HEAD` (405), and a Caddyfile replaced by rename is not
seen by the running container, so the file is written in place. What could not be tried is
named in its section 3: the server's own layout, the server's real image, TLS.

### Still open

- Everything the owner has to supply or decide is in the deployment document, section 2.
- Not verified anywhere: the client on Windows with WebView2 (hosted pages, their folders,
  the first sizing), a native Windows build and installer, a real Telegram bot and group, the
  server's own layout beyond what the repository and the 2.0.3 record say.
- Known and not changed: the viewer's editing controls and the header width above; the three
  other naive-against-aware lock comparisons; the items the code review left open (below).

### Sandbox rules met in this session

- `python3 - <<EOF` and `sh -n` on a script written in the session are refused; a script file
  that is run is not.
- A long command is not killed here, it goes on in the background; `setsid nohup … &` with a
  log outside the checkout worked for the builds and suites.
- `*.log` is ignored by git: evidence is kept as `.txt`.
- `checks.py` creates its accounts and needs the stack fresh; `run.sh` does everything in
  order and removes the stack, `run.sh up|rehearse|start|checks|down` do it step by step.

## Resume session — 2026-11-12, branch `work/time-hub-admin-resume`

Historical from here down. At the time, the next session was to pick up from
`docs/tasks/CONTINUE_TIME_HUB_ADMIN_PROMPT.md`.

**Baseline.** `main` at `7c4e4f48a12791c9beddcb07fdf164048db6955b`: release 2.0.3 (`f5eede4`)
and the Time work (`22b8417`) merged. Nothing is deployed. Production was read once with the
SELECT-only role (pre-checks below) and not changed. The session ended early, on credits.

| Commit | What it holds |
|---|---|
| `feb2d2b` | WIP 1: desktop Hub entries and hosted pages, admin panel shell mode, production pre-checks, backend test runner |
| `924a2b7` | WIP 2: fixes from the code review, deployment overlay, e2e stack skeleton |
| the commit with this text | tests for the review fixes, `scripts/hub/desktop-tests.sh`, this record, the continuation prompt |

### Decisions

Desktop:

- `GET /api/auth/me` is read with three optional fields, `workspaces`, `workspace_roles`,
  `global_admin`. A server that does not send them (production 2.0.3) is a Checks server:
  Checks as before, Time and CRM administration never inferred. A field in a shape the build
  cannot read counts as not sent, so it cannot fail a sign-in.
- The Hub lists by `workspaces`. The Time role `employee` has no interface (the server leaves
  `time` out of `workspaces` for it); it only lets a person mark attendance through the bot.
- Time and the admin panel are hosted pages (`desktop/crates/tbs-app/src/screens/hosted/`):
  the page files are compiled in (`build.rs`, from `time/crates/server/assets` and
  `backend/admin_panel`), each page has its own scheme (`tbstime`, `tbsadmin`) and may reach
  only `/api/time` or `/api/admin` through the shell's signed-in API client. The page holds
  no password, token or cookie; of its headers only `Content-Type` goes to the server. It can
  tell the shell two things: `access-lost`, and `export` with a `YYYY-MM` month (Time only).
- The server decides access on every request (gateway, admin API). The client follows it:
  events from refused requests, `/api/auth/me` every 30 s and on entering the Hub or a workspace.
- One web-view folder per page, account and server under `<data>/web/`; removed on sign-out
  and on revocation, whether or not the page is open.
- The API client has a session epoch: an answer that belongs to a session which has since
  ended or been replaced (a late 401/403, renewed tokens, an overtaken sign-in) is dropped.

Deployment (prepared, not rehearsed, not approved):

- `deploy/time-hub/docker-compose.time.yml` is an overlay: one container `timelist`, one
  volume `timelist_data` (the SQLite file), no published port. The main compose file is not edited.
- `.env` on the server gets five values: `TIMELIST_INTERNAL_URL=http://timelist:8080`,
  `TIMELIST_API_TOKEN`, `TIME_HUB_KEY` (read by the backend through its `env_file`), and
  `TIME_BOT_TOKEN`, `TIME_BOT_CHAT_ID`, which the overlay passes to Time as
  `TELEGRAM_BOT_TOKEN`/`TELEGRAM_CHAT_ID` (the backend has a `TELEGRAM_BOT_TOKEN` of its own).
- `deploy/Caddyfile` gets one route, `/admin` and `/admin/*` to the backend. `/internal/*` is
  not routed to the backend; that it stays so has to be shown in the rehearsal.
- Backend image: `backend/Dockerfile.overlay` over the live image is enough, because
  `backend/requirements.txt` is unchanged since `f5eede4` (checked with `git diff`).
- The Time image has to be built off the server (a Rust release build; the server has about
  2.9 GB of memory) and loaded there with `docker save` / `docker load`.
- Migrations 0034 and 0035 only add: `checks_access` true for everyone, `time_role` NULL for
  everyone, `global_admin` true where `lower(role) = 'admin'`. Rollback is the previous image,
  the previous Caddyfile and stopping `timelist`; the columns stay and no database is restored.

### What is done, and how far it was checked

| Item | State |
|---|---|
| Hub shows Checks / Time / CRM administration by grant; no-workspace state; old backend keeps Checks | done, unit tests |
| Time and admin panel as hosted pages; narrow page-to-shell contract; per-account folders | done, unit tests; **never opened in a real window** |
| Revocation, deactivation, expired session, sign-out, user switch close the page and drop caches | done, unit tests |
| Admin panel shell mode (`backend/admin_panel/admin.js`), Time page hook (`time/crates/server/assets/app.js`) | done; **no automated test, not run in a browser this session** |
| Fixes from the code review (below) | done; see "Tests actually run" for the last commit |
| Production pre-checks `deploy/time-hub/prechecks.sql` | done, run once read-only |
| `deploy/time-hub/postchecks.sql`, compose overlay, Caddy route, `.env.example` | written, **not rehearsed** |
| `scripts/hub/e2e/` (`compose.yml`, `e2e.env`, `fake_telegram.py`) | written, **never run**; `prepare.py`, `checks.py`, `run.sh` not written |
| Migration rehearsal on PostgreSQL, end-to-end run, real-window run, Windows build | **not done** |
| `docs/handoff/TIME_HUB_ADMIN_DEPLOYMENT.md` | **not written** |

### Tests actually run (2026-11-12, UTC)

- Whole backend suite, 11:45, Python 3.11 image pinned in `backend/Dockerfile`, pinned
  requirements, SQLite in memory, dummy secrets, on the tree that became `feb2d2b`:
  **1346 passed, 1 skipped**, 266 s. A second run through `scripts/hub/backend-tests.sh` was
  cut by the shell timeout at about 85% with no failure shown; that is not a result. The suite
  was not run after `.env.example` and `deploy/Caddyfile` were edited.
- Desktop, `cargo test --workspace --locked` in `tbs-linux-dev:pinned`, on `feb2d2b` (twice,
  12:06 and 12:12): **340 app tests and 136 core tests passed**. `live_backend` and
  `live_writes` report one passed test each by returning early without a server; they are not
  integration coverage. An early run the same day showed 300 app tests; it was not a clean
  baseline of `7c4e4f4` (the clean one was refused by the Docker bind-mount rule and not repeated).
- Desktop on `924a2b7`: `cargo test --workspace --locked --no-run` compiled, exit 0.
- Desktop on the last commit: a whole-workspace run passed **348 app tests** and stopped at one failing core test. That test was new and its second half was itself wrong (its stand-in server kept answering 401 after a renewal, so the client rightly ended the session); the half was removed and the core tests were rerun alone: **141 passed**. The two `live_*` test files were not run again; they are not coverage.
- Time image: `docker build -t timelist:local time` succeeded (about eight minutes on two CPUs).
  The container was not started; whether its `HEALTHCHECK` (`curl /api/meta`) passes in Hub
  mode, where the API wants the gateway's token, is **unverified**.
- Production, read-only over the pinned SSH tunnel: schema `20261112_0033`; none of
  `checks_access`, `time_role`, `global_admin` exists; two active accounts with role `admin`
  and one active `operator`; no other role values. Migration 0035 would therefore make exactly
  those two accounts CRM administrators. Their names are in the session's report to the owner,
  not in git; `prechecks.sql` prints them again. The shared role cannot read 2FA state or
  Telegram IDs; the two queries for an administrator are at the top of `prechecks.sql`.
- No real Telegram bot, no browser, no native window, no Windows build, no production change.

### Code review of `7c4e4f4..feb2d2b` (independent, read-only)

No critical finding; the page/shell boundary held (prefix checks, header filtering, method
list, message contract, Time cannot reach `/api/admin` and the panel cannot reach `/api/time`).
Fixed in `924a2b7`:

- High: with a launch password, a page that had not been started stayed on «Загрузка…».
- Medium: after losing CRM administration the settings kept showing accounts and sessions.
- Medium: a web view and its signals had no owner; a late view or a late "loaded" could attach
  to the next opening. Each opening now has a number carried by the view and its signals.
- Medium: "forget" emptied the web view only if the page was open; the folders are now removed.
- Low: answers about the account applied out of order; a refusal or a token renewal of an
  ended session applied to the next one; an overtaken sign-in kept as a sign-in; the workbook
  offered after its page was closed; a grant in an unreadable shape failed the sign-in;
  navigation allowed to any same-origin address; proxied answers without `nosniff`.

Left open, on purpose or for lack of time:

- The page's message handler does not check the sender's address (cannot be verified on
  WebView2 here; navigation is held to one document and frames are blocked by the page's CSP).
- The sign-in form does not number its attempts: an answer that arrives after "Back" still
  signs in (same person, same credentials). The cross-account case is closed in the API client.
- The Time page does not post `access-lost` for the plain-text refusal `inactive_user`; the
  shell handles that refusal itself.
- Proxied bodies are buffered whole without a size cap.
- Older behaviour, not changed: with `LAUNCH_GATE_ENABLED`, `/api/auth/refresh` is not exempt
  from the gate, so a saved session is dropped at every launch.

### E2E design, so it is not worked out again

`scripts/hub/e2e/compose.yml` mirrors the server's service names (`postgres`, `redis`,
`backend`, `caddy`) so that `deploy/time-hub/docker-compose.time.yml` applies on top unchanged:

    docker compose --project-directory . --env-file scripts/hub/e2e/e2e.env \
        -f scripts/hub/e2e/compose.yml -f deploy/time-hub/docker-compose.time.yml up -d

Its default network is internal (no internet for the backend, Time or the fake Telegram);
Caddy alone publishes `127.0.0.1:18080`. The backend runs from the pinned Python image with the
`hub-backend-venv` volume (created by `scripts/hub/backend-tests.sh`), without TDLib. Known
loose ends in the file: `SYSTEM_ACCESS_TOKEN` and `TIME_BOT_CHAT_ID` sit in the shared backend
environment and belong to the `tool` service only; the `run` volume is root-owned, so the
config step has to run as root.

Still to write:

- `prepare.py config`: `root-access.server.json` with the hash of `SYSTEM_ACCESS_TOKEN`
  (`services.root_access_config_service.hash_password_pbkdf2`, shape as in
  `backend/tests/hub_support.py`), and the Caddyfile made from `deploy/Caddyfile` with only the
  site address replaced by `:80` (the real one would ask Let's Encrypt for the production name).
- `prepare.py legacy-schema`, run with the code of `f5eede4` (`git archive f5eede4 backend`):
  `create_all`, accounts shaped like production (two active admins, one operator, one inactive
  admin), then `alembic stamp 20261112_0033`.
- `checks.py`, through Caddy and directly: `/health`; the system-access gate without Caddy;
  `/admin` 308, `/admin/` with its CSP; `/internal/time/telegram/{id}` not reachable through
  Caddy, refused without the key or with proxy headers, answered directly with the key; a
  pre-migration admin signs in and is a CRM administrator; accounts created through
  `/api/admin/users` for Checks only, Time viewer, Time manager, both, neither, CRM
  administrator without workspaces, Time employee, disabled; for each the `/api/auth/me` fields
  and the matrix Checks API / `/api/time/meta` / a Time write / `/api/admin/meta` / export;
  revocation and re-grant on a live token; deactivation; `revoke-sessions`; refresh rotation;
  lockout after wrong passwords; a locked month refuses edits (409).
- Bot, through `fake_telegram.py` (`TELEGRAM_API_BASE=http://faketg:8081`): a linked employee's
  message in the work group is stored (🤷 until bound, ✍ after `POST /api/time/tg/users/{id}/link`);
  an unknown sender, a revoked or disabled account get 🤷 and nothing is stored; another group
  is ignored; `/id` answers in private.
- `run.sh`: up, migration rehearsal, start, checks, down.

### Commands and sandbox rules

- Desktop tests: `scripts/hub/desktop-tests.sh` (the wrapper itself was never run; the
  `docker run` line inside it is the one every run above used).
- Backend tests: `scripts/hub/backend-tests.sh [pytest arguments]`; about five minutes.
- Time image: `docker build -t timelist:local time`.
- Inline interpreter heredocs are refused: put the script in a file. A shell command is killed
  at its timeout (two minutes by default, ten at most): run long work as `docker run -d`, then
  `docker wait` and `docker logs`. Docker bind mounts only from the checkout. Long tool output
  is replaced by a summary: read at most a hundred lines at a time.
- Read-only production access: `docs/handoff/SHARED_SSH_ACCESS.md`. The key was supplied as a
  chat attachment and kept outside the worktree; a new sandbox needs it again. It expires on
  2026-11-25. It gives no shell and no right to deploy.

### Owner: secrets, decisions, approvals

- The Time bot does not exist yet: token from @BotFather, privacy mode off, the work group's ID.
- `TIMELIST_API_TOKEN` and `TIME_HUB_KEY` are to be generated on the server, never in chat or git.
- Whether both accounts that 0035 makes CRM administrators should keep that right, and
  whether they have 2FA set up (the panel sends an administrator without it to the desktop sign-in).
- Approval of the concrete plan before any migration, new image, new container or Caddy
  reload. Somebody with deploy rights has to carry it out: the shared key cannot.
- A Windows machine for the release build and installer; the version is still 2.0.3.

## Status (previous session, historical)

| # | Milestone | State | Commit |
|---|---|---|---|
| 0 | Investigation, owner questions, plan | done | `f81d7c5` |
| 1 | Backend: migration 0034, workspace access on the account, enforcement on every request | done | `bfbd068` |
| 2 | Backend: admin API (grants, per-user sessions, guards, audit), Time gateway, bot identity endpoint | done | `bfbd068` |
| 3 | Time: hub mode, bot checks per message, UI in gateway mode, container image | done | `35f8138` |
| 4 | Scope correction: CRM administration separate from workspaces (migration 0035, registry, generic API) | done | `b47d06f` |
| 5 | CRM administration panel (Russian), `/admin/` | done | this commit |
| 6 | Desktop: «Учёт времени» and «Администрирование CRM» in the Hub | pending | — |
| 7 | Deployment artefacts, local end-to-end run, deployment and rollback plan for approval | pending | — |

## Milestone 5 — the CRM administration panel (2026-11-12)

`backend/admin_panel/` (`index.html`, `admin.css`, `admin.js`, no build step, no dependency) served by
`backend/api/admin_panel.py` at `/admin/`. It is a client of `/api/admin/*` and nothing more: it holds
no data, and every call it makes is authorised on the server.

What the owner can do in it: find accounts (search; filters by state, by workspace, without any
workspace, CRM administrators); create an account; edit name, expiry, session lifetime; appoint or
remove a CRM administrator; grant or revoke each workspace with its role; edit the Checks data
restrictions of an operator (tabs, folders, sources, closed periods); set a new password (with a
generator and the server's rules shown); require or reset 2FA; see and end sessions; link a Telegram
ID; lift a sign-in lock; disable and enable; read each account's effective access and its journal;
see the workspaces with their roles and holders; read the account journal.

How it keeps the administrator's session: the access token lives in the page's memory only; the
session is continued by the backend's existing HttpOnly refresh cookie, which script cannot read.
Nothing is written to `localStorage`. Behind the launch gate it asks for the launch password first
and keeps only that gate token for the tab. The page is served with
`default-src 'none'; script-src 'self'; style-src 'self'; connect-src 'self'; form-action 'none';
frame-ancestors 'none'`; it has no inline script or style and builds the page from text nodes.

### Evidence

| What | How | Result |
|---|---|---|
| Files and policy | `tests/test_admin_panel_static.py` (4 tests): only the three named files are served, traversal and other names 404, headers and CSP, the page obeys its own policy, files are public while `/api/admin/meta` is 401 / 403 `global_admin_required` | passed |
| Whole backend suite | `pytest tests` | **1330 passed** |
| The panel in a browser | headless Chrome 157 against the real application's HTTP layer with PostgreSQL 15 and Redis 7 in local containers; ten seeded test accounts. The lifespan was replaced so TDLib, history loading and the agent scheduler did not start, and the system-access gate was off because no Caddy stood in front | below |

Observed in the browser run (`/tmp/hoplite/tools/panel-e2e.sh`, `panel-e2e-launch.sh`; a local
simulation with test data, not production):

- A Checks administrator and a Time manager sign in with correct passwords and are refused:
  «У этой учётной записи нет прав администратора CRM»; the session just opened is closed again.
- A CRM administrator with no workspace opens the panel; after a reload the session resumes with
  nothing in web storage and no cookie visible to script.
- Create with a generated password (6/6 rules): the row is `operator | checks f | time manager |
  global_admin f | argon2id`; the password is not in the journal.
- New password: asks first, then the new password signs in and the old sessions are gone.
- A change reaches a signed-in client: with one token, Checks `200` → revoke → `403
  workspace_forbidden` → grant CRM administration → admin API `200`, Checks still `403` → restore →
  admin API `403`, Checks `200`; `permissions_version` unchanged.
- End sessions: the token gets `401`. Disable: token `401`, sign-in `inactive_user`; enable: signs in.
- Own card: the administrator box and the disable button are disabled; direct requests for the same
  are refused by the server (`cannot_demote_self`, `cannot_disable_self`).
- An account stripped of CRM administration but still Checks administrator and Time manager: admin
  API, sessions and system settings `403`, Checks scopes `200`.
- Behind the launch gate: the API answers `launch_required`, the page still loads, the panel asks
  for the launch password, counts wrong attempts, then signs in and survives a reload.
- No page errors and no console output during the runs; dark scheme and a 430 px viewport render.

## Milestone 4 — administration is global (2026-11-12, after the owner's correction)

The owner corrected the scope: the panel is the central place of the whole CRM, administration must
not depend on Time or on any workspace, and workspace managers must not become administrators. What
changed, and how each part was checked:

- **`users.global_admin`** (migration `20261112_0035`) is the CRM administrator. It is filled for
  exactly the accounts whose role is `admin` and for nobody else. `require_global_admin` guards all
  of `/api/admin/*`, `/api/security/sessions*` and `/api/system-settings`.
- **`role` is now only the Checks role.** Everything it did inside Checks is untouched. Every use of
  `require_admin_user` was reviewed: the 19 routes that are CRM administration (14 under
  `/api/admin`, 3 session routes, 2 system-settings routes) moved to the new guard; the rest are
  Checks operations and stayed. The two Checks journal listings (`/api/audit`,
  `/api/security/audit`) leave the account-administration records out for anyone who is not a CRM
  administrator.
- **Workspace registry** in `backend/services/workspace_access.py`: Checks and Time with their roles,
  titles, Hub visibility, API prefixes. The admin API takes `workspaces: {id: role}` and reports
  `workspace_roles`, `workspaces`, `global_admin`, `effective_access`; `/api/admin/meta` lists the
  registry. The earlier `checks_access` / `time_role` request and response fields are gone (they
  existed only in this branch).
- **Older clients:** a role-only request still creates and edits Checks accounts; demoting an
  administrator through such a client also removes `global_admin`; nothing they send can grant it.
- **2FA policy and the expiry exemption** cover both kinds of administrator; `auth_bot`'s system-wide
  switches require a CRM administrator.
- **Bootstrap:** `scripts/create_admin.py` creates or promotes a CRM administrator from the server's
  shell, asks for the password without echo, and writes to the audit journal.

Migration from the Time-only implementation: nothing in production to convert (it was never
deployed). A local database at `20261112_0034` continues with `alembic upgrade head`.

### Evidence (all local; no production, no real Telegram)

| What | How | Result |
|---|---|---|
| Whole backend suite | `pytest tests`, Python 3.11, SQLite in memory, fake session store, dummy secrets | **1326 passed**, 0 failed, 0 skipped |
| Administration boundary | `tests/test_admin_workspace_api.py` (25 tests): every administration route of the running app is refused to an operator with every tab, to a Checks administrator who is also Time manager, and to a Time-only manager; a CRM administrator with no workspace administers and is refused in Checks and Time | passed |
| Grants reach a running client | same file and `tests/test_workspace_access.py`: grant/revoke of `global_admin` and of each workspace takes effect on the next request of an already issued token | passed |
| Lock-out protection | last active CRM administrator cannot be disabled or stripped through API or service; self-disable and self-strip refused | passed |
| Extensibility | `test_another_workspace_is_a_registry_entry`: a third workspace added to the registry appears in `/api/admin/meta`, owns its API prefix and is enforced, with no other change | passed |
| Migrations on SQLite | `tests/test_workspace_access_migration.py`: 0034 and 0035 up, twice, down, twice | passed |
| Migrations on PostgreSQL 15.19 | disposable container; schema built by the 2.0.2 models and stamped 0033, then `alembic upgrade head`, the 2.0.2 code run against the new schema, `downgrade 20261112_0033`, `upgrade head` | passed (below) |

PostgreSQL round trip, abridged output:

```
Running upgrade 20261112_0033 -> 20261112_0034, Workspace access on user accounts.
Running upgrade 20261112_0034 -> 20261112_0035, CRM administration as a right of its own on the account.
checks_access|boolean|NO|true      global_admin|boolean|NO|false      time_role|character varying|YES|
owner|admin|t|t   operator|operator|t|f   gone|operator|f|f   former-admin|admin|f|t
global_admin is true exactly for the accounts whose role was admin
every pre-existing column of every account is unchanged
an admin made by the old code is a Checks administrator, not a CRM administrator
removing the only active CRM administrator is refused: last_admin
columns left after downgrade: 0
```

Known consequence, stated in the plan: a schema downgrade forgets who was made or unmade a CRM
administrator; after a new upgrade the right is derived from the Checks role again.

## Milestones 1–3 — evidence recorded when they were committed

| What | Result |
|---|---|
| Backend suite at `35f8138` | 1312 passed, 1 skipped (optional `oletools`, since installed) |
| Checks routes without the Checks workspace | every route of the running application that resolves a user and belongs to Checks (more than 150) refuses with `workspace_forbidden`, exports and Excel management included |
| Time workspace (`cargo test --workspace --locked`, pinned image) | 17 server tests (bot against a stand-in Bot API: revocation, deactivation, foreign groups, duplicates, restart), 28 core, 1 golden |
| Time container | `time/Dockerfile` builds (`timelist:local`) |
| Desktop baseline before any desktop change | `cargo test --workspace --locked`: 122 core + 302 app passed; the live tests return early without a backend and are not integration evidence |

## Milestone 0 — investigation (2026-11-12)

- Read the task in full, `time/` (both copies are identical apart from `.gitattributes`), the backend
  auth stack, the desktop shell and Telegram WebView host, compose and Caddy.
- `HOPLITE_SSH_KEY_PATH` is not set: the shared read-only SSH key is not provisioned in this sandbox.
  Work continues locally only.
- Owner's answers (plan, section 2): Telegram ID entered by the administrator; no bot yet; no Time
  data; nobody receives Time at migration; one global panel for the whole CRM.
- Tooling: Docker daemon available, 2 CPUs / 8 GiB (a resize was refused by the workspace ceiling).
  Rust runs only in the pinned image built from `desktop/tools/linux-dev.Dockerfile`
  (`rust:1.99.1-bookworm`), Cargo with `--locked`. Backend tests run in a local venv on SQLite with
  dummy secrets and no network services.

Baselines before any change: backend auth subset 13 passed; whole backend suite 1226 passed, 1
skipped — same as the 2.0.2 record; Time workspace 38 passed.

## Open items / unrun checks

- The panel was exercised in headless Chrome only; no other browser, and not behind the production
  Caddy (which adds `X-System-Access` and needs the `/admin*` route from the deployment plan).
- The panel does not set up 2FA for an administrator who is required to have it and has none yet;
  it says so and points to the desktop sign-in, where that flow exists.
- Desktop: no change yet; the Hub does not list «Учёт времени».
- `cargo clippy` and `cargo fmt --check` for Time: the tools are absent from the pinned image.
- Windows build of the desktop, a real Telegram bot and production were not exercised.
- Sign-in records (`login_success`, `login_failed`) of every account are still listed in the Checks
  journal for a Checks administrator, as they always were.
