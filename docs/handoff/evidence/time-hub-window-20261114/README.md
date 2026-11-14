# Time and CRM administration under the new top bar, 2026-11-14

Pictures of the client with ONE slim top bar (home button, workspace name, online mark,
account, sign-out) and no rail for the two hosted pages, «Учёт времени» and
«Администрирование CRM». Checks keeps its rail.

## Platform and what was tested

- **Linux build, WebKitGTK web view, Xvfb screen (96 dpi). NOT Windows, NOT WebView2.**
  Anything about WebView2 (hint stacking over the web view, resize, fonts) is not verified.
- Client: `cargo build -p tbs-app --locked` in `tbs-linux-dev:pinned`, debug build, version 2.1.0.
  Built from branch `vorflux/time-release` at `0efe423` plus the then-uncommitted changes of
  `desktop/Cargo.toml`, `Cargo.lock`, `app.rs`, `screens/login.rs`; those are the content of the later
  commits `e6d3940` and `a29f2ac` (the tree was at `6624799` when the run ended).
- Server: the disposable stack of `scripts/hub/e2e` (`run.sh up`, alembic upgrade, `start`, `page`), reached through
  `socat` on 127.0.0.1:18080. Only made-up accounts: `ui_manager` (Time manager), `ui_viewer` (Time viewer),
  `e2e_admin` (CRM administrator, Checks admin, Time manager), `e2e_both` (Checks operator, Time manager, Time taken
  away during the run), `e2e_checks`. No password is stored here.
- Pictures `01`..`24` are crops of the Xvfb screen to the client's window (1360 x 860 unless the name says other,
  start window 880 x 560). `B1`..`B8` are Chromium 131 headless pictures of the Time page from
  `scripts/hub/e2e/page_shots.py` (74 checks ok, 0 failed), not of the program.

## Pictures

| File | Shows |
|---|---|
| 01..03 | Time as manager, light, 1360: bar, no rail/hamburger, page full width under the bar; «Табель» tab; previous month (Октябрь) |
| 04, 05 | hint of the home button and hint of the connection mark: both open sideways inside the bar, visible |
| 06 | home button: start window again, session kept («Открыть Учёт времени») |
| 07 | Time as manager, dark, 1360 |
| 08, 09 | Time as manager, dark, window at the minimum 1024 x 640 (Табель, Тайм-лист) |
| 10 | window resized to 1200 x 760 with the page open: web view follows, bar stays |
| 11 | sign-out from the bar: sign-in form (dark) |
| 12, 13 | other account `ui_viewer` after sign-out: start window offers only Time; page shows `ui_viewer`, «Просмотр» mark, none of the manager's session |
| 14, 15 | viewer, light, 1024 x 640; day panel opened |
| 16 | sign-out at 1024: sign-in form |
| 17 | CRM administration with a user drawer open (opened by the click-through described below) |
| 18 | CRM administration, user list of 8 accounts under the bar |
| 19 | Checks: rail (folded), no bar |
| 20 | Hub, Time, Hub, Checks: the fold is kept |
| 21 | Checks, rail unfolded, no bar |
| 22, 23 | `grant.py e2e_both time=-` while Time was open: five seconds later the start window with the toast «Доступ к разделу «Учёт времени» закрыт администратором», only Checks offered |
| 24 | click on «Открыть» in the start window also lands in the Time page (see below) |
| B1..B8 | Chromium: manager 1024 light and dark, viewer 1024, closed month (manager, viewer), standalone 1024 light and dark, viewer 1360 dark |

## Limitations

- Debug build; one run; Linux fonts (DejaVu/Noto), not Windows fonts.
- Not done: offline state of the bar (stack stayed up), long account name, loading and failed-page states of a hosted
  page, revocation of CRM administration in this run, 1024 x 640 for light manager and dark viewer (light x 1024 was
  seen for the viewer, dark x 1024 for the manager).
- Click-through (picture 17, 24): a click on «Открыть» in the start window at point P selects the cell/row at P of the
  page that opens, also when the pointer is moved away at once. Reproduced with xdotool; the base commit
  `21b3ebf` has no embedded Time page, so no before/after. Not known whether it happens with WebView2 or a real mouse.
