# Start window, «Чеки» in the rail, lines between columns

Branch `hoplite/methymna-6dd80d56`, from `bfff4f6` (`main` at the time), the
integrated 2.0.1. Scope of writes: `desktop/` and this file. The backend, the
production server, the default order of the table's columns (another session
owns it: `transactions/table.rs` and `tbs-core/src/columns.rs` are untouched),
the version and `IMPLEMENTATION_PLAN.md` are not changed. Nothing was deployed.

Everything below says what was run and what it showed. What was not run is
said to be not run.

## Starting point

`bfff4f6` holds both earlier workstreams: `git merge-base --is-ancestor` is true
for the backend's `b6419c6` and for the UI's `83f5bdd`, joined by the merge
`b5bbc0d` and followed by the 2.0.1 release commit `88024fd`.

## What changed

**The start window** (`screens/login.rs`, `app.rs`, `ctx.rs`). The window with
the brand, «РАБОЧЕЕ ПРОСТРАНСТВО» and «Чеки» is the first screen of every launch.
Before, a saved session went straight to the table.

- A saved session is asked about (`Api::resume`, unchanged) while the window is
  already up; until the server answers its account half says so and offers no form.
- Signed in, the account half names the account and opens «Чеки» on that session:
  from the workspace row, from «Открыть Чеки», or with Enter. Nothing is typed.
- Signed out, it is the sign-in it was; signing in opens the workspace as before.
- The licence check and the launch password keep their place between the start
  window and the work. An answer that arrives when the start window is back (a
  licence answer, a launch password wanted by a request already over) opens nothing.
- «Выйти» (rail, profile, start window) ends the session as before and leaves the
  same window with the sign-in. Closing the program signs nobody out.
- A session the server ended brings the same window with «Сессия завершена,
  войдите снова», said once however many requests failed on it.

**The way back** (`app.rs`). The rail has «Главный экран» under its head, with a
hint when folded: back to the start window, session kept. The workspace's
screens are closed on the way as on signing out, a locked folder asks for its
code again, and coming back in runs the licence check and loads the table anew.

**The name** (`app.rs`). The rail is headed «Чеки»; «учёт чеков» under it stays.
`PRODUCT` («TBS inform CRM») still names the window, the dialogs, the installer
and the update. No storage key, folder or file name changed.

**Lines between columns** (`grid/mod.rs`, `grid/geometry.rs`). Every column has a
one-pixel line at its right edge, in `border_strong`, in the header and in the
rows. One function places both from the grid's own geometry (`Columns::edge_x`,
which the resize grip now uses too), so they meet, and follow the scroll, a
resize in progress, a reorder and whatever order the columns are in. They are
drawn over hover, selection and painted cells and under the cursor's frame, and
end with the last row. Where the pinned columns end the line is one step darker
(`control_edge`) and now runs through the header as well.

**Off Windows only** (`tbs-core/src/secret.rs`). The Secret Service client
blocks on a runtime of its own; asked from a tokio worker, as a sign-in does
when it saves its refresh token, it panicked and the sign-in never finished.
It is now asked from a thread that drives no runtime. The Windows path
(Credential Manager) is the same code as before.

**Tools** (`desktop/tools/`). `stub-server.py`, a stand-in for the backend, and
`window-check.py`, which shows the real client on an Xvfb screen and drives it
with xdotool: the Linux client, or the Windows build under Wine.
`desktop/README.md` says how. The icon of «Главный экран» is `home_20_regular`
of the same Fluent set, added through `extract-icons.mjs`.

## Verification

Rust runs only in the image of `desktop/tools/linux-dev.Dockerfile`
(`tbs-linux-dev:1.99.1`: rustc and cargo 1.99.1, `x86_64-w64-mingw32-gcc` 12),
sources bind-mounted, build output in the `tbs-target` volume. The Docker
daemon of this sandbox has one CPU.

### Tests

Run on the tree of `8a21cb2`, whose Rust sources are those of `fb7f67c`:

| Run | Result |
| --- | --- |
| `cargo test -p tbs-core --locked` | `118 passed; 0 failed`. 116 were there before; the two new ones are in `secret.rs`. |
| its `live_backend` and `live_writes` | `1 passed` each, and neither did anything: without `TBS_E2E_LOGIN` they return at once. No backend was asked. |
| `cargo test -p tbs-app --features devtools --locked` | `298 passed; 0 failed`. 264 were there before, 34 are new. |
| `cargo build -p tbs-app --locked` | finished. This is the Linux client the window check shows. |
| `cargo build -p tbs-app --locked --target x86_64-pc-windows-gnu` | finished, see «Windows build». |
| `cargo build -p tbs-app --release --locked --target x86_64-pc-windows-gnu` | finished, see «Windows build». |

The log of the app's tests says `XDG_RUNTIME_DIR is invalid or not set` many
times: the container has no desktop session. Tests of `screens::transactions`,
which this work did not touch, print it too, and no test fails on it.

Of the 264 earlier tests of `tbs-app` two were edited, each because it asserted
the very thing that was asked to change:

- `window_tests::only_the_start_and_the_sign_in_are_compact` is now
  `only_the_start_window_is_compact`: the launch has no phase of its own any
  more (`Phase::Boot` is gone), it is the start window.
- `rail_tests::the_folded_rail_keeps_its_icons_and_drops_the_words` looks for
  «Чеки» and «Главный экран» where it looked for the product's name.

The other 262 are as they were, those of the table, its editor and its menus
among them.

New tests, by what was asked:

| Asked | Tests |
| --- | --- |
| The start window is first with a saved session, and nothing of the table is opened | `app::hub_tests::a_launch_with_a_saved_session_opens_on_the_start_window`, `login::tests::a_saved_session_is_asked_about_before_any_form_is_offered` |
| … and without one | `a_launch_without_a_session_opens_on_the_start_window_with_the_sign_in`, `without_a_saved_session_the_sign_in_is_there_from_the_start` |
| A refused or unreachable saved session | `a_saved_session_the_server_refused_leaves_the_sign_in`, `a_session_the_server_could_not_be_asked_about_is_kept_for_the_next_launch` |
| «Чеки» opens on the session, no second sign-in | `choosing_the_workspace_opens_it_on_the_saved_session`, `a_session_opens_the_workspace_from_its_row_and_from_the_button`, `enter_opens_the_workspace_unless_a_field_took_it` |
| Signing in through «Чеки» opens it | `signing_in_at_the_start_window_opens_the_workspace` |
| Back to the start window without signing out, and in again | `the_way_back_keeps_the_session_and_the_way_in_asks_for_nothing`, `the_way_back_works_from_the_workspace_only`, `rail_tests::the_rail_leads_back_to_the_start_window_folded_or_not` |
| Explicit sign-out | `signing_out_comes_back_to_the_start_window_signed_out`, `signing_out_at_the_start_window_leaves_the_sign_in`, `signing_out_of_the_start_window_is_asked_of_the_shell` |
| Expiry and access restrictions | `a_session_that_ends_comes_back_to_the_sign_in_and_says_why`, `an_expired_licence_still_stands_between_the_start_window_and_the_workspace`, `the_launch_password_still_stands_before_the_work`, `nothing_opens_the_workspace_from_the_start_window_but_choosing_it` |
| «Чеки» in the rail, the product's name elsewhere | `rail_tests::the_rail_is_headed_by_the_workspace_and_not_by_the_product` |
| Lines: header and rows, both themes | `grid::tests::a_line_parts_every_column_in_the_header_and_in_the_rows_of_both_themes` |
| Lines: scroll and pinned columns | `the_lines_scroll_with_their_columns_and_the_pinned_ones_stay`, `geometry::tests::edges_scroll_with_their_columns_and_pinned_ones_stay` |
| Lines: any order, reorder, resize | `the_lines_stand_between_the_columns_in_whatever_order_they_are`, `edges_follow_whatever_order_the_columns_are_in`, `a_line_goes_with_its_column_while_the_column_is_resized`, `an_edge_moves_with_its_column_when_columns_are_resized_or_moved`, `the_grip_is_on_the_line_between_columns` |
| Lines: selection and editing kept | `a_selected_cell_keeps_its_line_and_the_cursor_keeps_its_frame`, `the_lines_end_with_the_last_row` |
| Off Windows: the credential store asked from inside a runtime | `secret::tests::the_store_can_block_while_a_runtime_is_being_driven`, `a_store_that_panics_has_no_answer_and_takes_nothing_down` |

The `grid::tests` read colours back from a picture the test simulator draws
(through the grid's own drawing code, at twice the size). They need `png`, so
they run with `--features devtools`, as the suite is run. Those pictures are
read and kept nowhere; none of the pictures in the repository is one of them.

`live_ui` (the flow against a local server) has a new step, out to the start
window and back in. Against the stub it printed `ok    start window and back:
Peek { loaded: true, total: 80, … }`, with one `POST /api/auth/login` in the
stub's log for the whole of it; the flow then stops at «Добавить», which the
stub does not have. It was **not** run against the real backend.

### The real window

The Linux client (`cargo build -p tbs-app --locked`) on an Xvfb screen of
1600×1000, in the image with `xdotool` and `gnome-keyring` added, signed in to
`tools/stub-server.py`: a stand-in that answers the sign-in, the session and
eighty made-up rows, and is not the backend. `tools/window-check.py` moved the
pointer and typed. The pictures in `desktop/docs/screenshots/` are of that
screen (`import -window root`); `p5-window-01`…`16` are cut to the client's
window, `p5-window-18` is the whole screen.

The stub prints every request it answers. Below, the polls of `/health` and
`/api/sync/manifest` and the reference data that loads with the table are left
out.

The first run, on the Rust sources as they are committed:

| Step | What the stub answered | Picture, `p5-window-…` |
| --- | --- | --- |
| First launch, nothing saved | nothing | `01-start-no-session` |
| Signing in at the start window | `POST 200 /api/auth/login`, the entitlement, the table | `03-signed-in-table-light` |
| «Главный экран» | nothing | `06-start-signed-in` |
| «Открыть Чеки» | the entitlement, the table; no sign-in | |
| Closed and started again | `POST 200 /api/auth/refresh` and nothing of the table | `08-restart-saved-session`, byte for byte the file of `06` |
| The workspace row | the entitlement, the table | |
| The access token made stale (`/_stub/expire`) | `POST 200 /api/auth/refresh`, then the table again: the session renewed, not ended | |
| The session ended on the server (`/_stub/revoke`) | `POST 401 /api/auth/refresh` | `15-session-ended-start-window`: the sign-in and «Сессия завершена, войдите снова» |
| Signed in again, «Выйти» in the rail | `POST 200 /api/auth/login`, the entitlement, the table, `POST 200 /api/auth/logout` | `16-start-after-sign-out` |
| Closed and started again | nothing: the sign-in | |

The lines in the pictures of that run: `03` (light), `04` (the edge of a column
dragged), `05` (scrolled sideways: the pinned columns and the darker line after
them stay, the others pass under), `11` and `12` (dark, and dark scrolled), `13`
(a selected cell keeps its frame and the lines run on), `14` (the rail folded).

The second run, once the tools were finished: a container made anew from the
image with only `apt-get install -y xdotool gnome-keyring` added, as
`window-check.py` says, the tools as committed in `8a21cb2`, the stub slow to
confirm a saved session (`--refresh-delay 12`). First launch, sign-in (one
`POST /api/auth/login`), «Главный экран», «Открыть Чеки» (the entitlement and
the table, no sign-in), closed and started again: five seconds after its window
appeared it shows «Проверяем сохранённый вход на этом компьютере», no form and
no table, the refresh not answered yet (`18-saved-session-being-confirmed`);
then «Вход выполнен», and the workspace row opened the table. The sign-out at
the start window had a short run of its own in the same container, the first
press having missed the link: `POST 200 /api/auth/logout`, the sign-in; closed
and started again, no request, the sign-in.

The client's stderr (`client.log`) was empty in every one of these runs.

### Windows build

Built on Linux, in the same image, for `x86_64-pc-windows-gnu` with MinGW-w64.
It is the target the README names; it was not built on Windows.

| Build | Result | `tbsparcer.exe` |
| --- | --- | --- |
| `cargo build -p tbs-app --locked --target x86_64-pc-windows-gnu` | ``Finished `dev` profile … in 13m 41s``, exit 0 | 259 272 178 bytes, PE32+ console |
| `cargo build -p tbs-app --release --locked --target x86_64-pc-windows-gnu` | ``Finished `release` profile [optimized] … in 31m 39s``, exit 0. For some nineteen of those minutes the build stood stopped, the one CPU given to the window check. | 26 913 280 bytes, PE32+ GUI |

Both print `warning: tbs-app@2.0.1: windres failed, the executable has no icon`:
`build.rs`, which is not changed, calls `windres`, and the cross toolchain here
has it only as `x86_64-w64-mingw32-windres`. So these two files have neither
icon nor version resource. `frontend/dist-telegram` was not built either, so
their Telegram section has no page. They are for checking and not for shipping;
they stay in the sandbox's volume and are in no repository. The setup
(`tools/build-installer.ps1`, Inno Setup) was not built: it needs Windows.

**Under Wine.** Both files were then run under Wine 10.0 (Debian
`10.0~repack-6`, a `debian:trixie-slim` container with `wine wine64 xvfb xdotool
imagemagick python3 mesa-vulkan-drivers libvulkan1 libgl1-mesa-dri
fonts-dejavu-core fonts-noto-core procps`), on an Xvfb screen of 1600×1000,
drawn by Mesa's software Vulkan (`WGPU_BACKEND=vulkan`), against the same stub
and driven by the same `window-check.py` with `CLIENT="wine …/tbsparcer.exe"`
(`desktop/README.md` has the whole line).
**Wine is not Windows.** This shows that the Windows build starts, draws and
keeps its session through the Windows calls as Wine answers them. Nothing here
was run on Windows itself.

The debug build:

| Step | What the stub answered | Picture, `p5-wine-…`, the whole screen |
| --- | --- | --- |
| First launch, nothing saved | nothing | `01-start-no-session` |
| Signing in at the start window | `POST 200 /api/auth/login`, the entitlement, the table | `03-signed-in-table-light` |
| «Главный экран» | nothing | |
| Closed and started again | `POST 200 /api/auth/refresh` and nothing of the table | |
| The workspace row | the entitlement, the table | |
| Scrolled sideways; the edge of «Источник» dragged 80 px | | `07-table-light-scrolled`, `08-table-light-column-resized` |
| Closed, `"theme": "dark"` written into `preferences.json`, started again | `POST 200 /api/auth/refresh` and nothing of the table | `09-restart-saved-session-dark` |
| The workspace row; scrolled sideways | the entitlement, the table | `10-table-dark`, `11-table-dark-scrolled` |
| «Главный экран», «Выйти из учётной записи» | `POST 200 /api/auth/logout` | `13-start-after-sign-out` |
| Closed and started again | nothing: the sign-in | |

In this run the press on «Открыть Чеки» after coming back missed the button and
nothing was asked of the stub; that step is in the release build's run below.

The lines of the Windows build, read off its pictures as the colour of single
pixels along one row of the header (y 207) and one row of the table (y 300).
The numbers are x on the screen; the window's corner is at 124, 100.

| Picture | Lines at x, the same in the header and in the row |
| --- | --- |
| light | 415, 493, **618**, 748, 898, 994, 1234, 1364: `#C6CDD7`, and `#7F8A99` at 618, where the pinned columns end |
| light, «Источник» 80 px wider | 415, 493, **618**, 828, 978, 1074, 1314, 1444: every line right of that edge moved by 80 |
| light, scrolled | 415, 493, **618** stay; then 850, 980, 1090, 1210, 1270, 1320 |
| dark, after the restart | 415, 493, **618**, 828, 978, 1074, 1314, 1444, the width kept over the restart: `#3C4552` on `#161A20`, `#67727F` at 618 |
| dark, scrolled | 415, 493, **618** stay; then 690, 930, 1060, 1170, 1290, 1350, 1400 |

The release build, under the same Wine:

| Step | What the stub answered, and what Wine kept |
| --- | --- |
| Started, nothing saved | nothing: the sign-in |
| Signing in | `POST 200 /api/auth/login`, the entitlement, the table. One entry in Wine's credential store (`wine reg query "HKCU\Software\Wine\Credential Manager" /s`) |
| «Главный экран», «Выйти из учётной записи» | `POST 200 /api/auth/logout`. No entry left in the store |
| Signing in again, «Главный экран» | `POST 200 /api/auth/login`, the entitlement, the table; then nothing |
| «Открыть Чеки» | the entitlement, the table; no sign-in |
| «Главный экран», «Выйти из учётной записи» | `POST 200 /api/auth/logout` |

Its screens (the sign-in, the start window signed in, the dark table, the start
window after the sign-out) differ from the debug build's in no pixel (`magick
compare -metric AE`: 0), so no pictures of it are kept. Over all the runs under
Wine the stub answered three sign-ins, one for each time the program stood
signed out, and `client.log` stayed empty.

## Limits

- **Not run on Windows.** Wine answered for it. Not seen on a real machine: the
  Credential Manager itself (under Wine the same calls keep the entry in Wine's
  registry), drawing through a graphics driver (here Mesa's software Vulkan
  drew), Windows' fonts and scaling, WebView2 and the Telegram section, the
  setup and the update. A run of the installed program on Windows is still owed
  before a release.
- **The pictures are not Windows' look.** They have the DejaVu and Noto fonts of
  the containers, so text is wider or narrower than it will be. The square
  before each operation's name is an emoji those fonts lack; `operation.rs`
  puts it there and is not changed.
- **The server was the stub** in every run with a window, and it checks nothing
  a server would. How the real backend answers `refresh`, `logout` and an ended
  session was not observed here. `tbs-core`'s requests are 2.0.1's (only
  `secret.rs` changed there); what changed is where the answers lead.
- **Not run against the real backend:** `live_ui`, `live_backend`, `live_writes`.
- **The executables built here are not the release's:** no icon, no version
  resource, no Telegram page, no setup. The version is still 2.0.1. Nothing was
  released, deployed or changed on the server.
- **The order of the columns** was left to its owner. The lines are placed from
  the grid's geometry whatever the order: the tests check shuffled orders and a
  reorder, the windows above showed the default order only.
- **The name on the start window.** The task calls it «TBS Information CRM». The
  window shows the brand it had, «TBS inform | CRM», and its title is «TBS
  inform CRM»; no words were changed there. If the longer name is wanted on it
  in words, that is still to be decided and done.
