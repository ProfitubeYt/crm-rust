# UI progress — Rust filters, View and desktop verification (Task B)

Branch `work/rust-ui`, created from baseline `a83b04467db66c24bf904b093a4ba9edc7d232c3`.
Owner of this file: the Task B session. Scope of writes: `desktop/` and this file.
`IMPLEMENTATION_PLAN.md` is read, not modified; final integration consolidates it.

Statuses below are backed by the commands and results written next to them.
Anything not run is said to be not run.

## Environment

- Sandbox: Docker runtime, Docker 29.1.3. The daemon gives containers 1 CPU and 7.7 GiB
  (the resize request to more CPU was refused by the workspace ceiling), so a cold
  build of `tbs-app` tests takes about 12 minutes.
- Rust runs only in the image built from `desktop/tools/linux-dev.Dockerfile`
  (base `rust:1.99.1-bookworm@sha256:e6f2cff3…3a9c`, cargo 1.99.1, rustc 1.99.1,
  targets `x86_64-unknown-linux-gnu` and `x86_64-pc-windows-gnu`). No host Rust.
- Sources are bind-mounted at `/work`; build output and the crate cache live in
  named volumes (`tbs-target`, `tbs-cargo-registry`, `tbs-cargo-git`), never in the repository.
- Docker policy of this sandbox allows bind mounts only from the workspace
  checkout paths, so "clean checkout" runs use a fresh `git clone` of the commit
  under `/tmp/hoplite/workspaces/verify`.

Commands (from the repository root):

    docker build -f desktop/tools/linux-dev.Dockerfile -t tbs-linux-dev:1.99.1 desktop/tools
    docker run --rm -v <checkout>:/work -v tbs-target:/target \
      -v tbs-cargo-registry:/usr/local/cargo/registry -v tbs-cargo-git:/usr/local/cargo/git \
      tbs-linux-dev:1.99.1 cargo test -p tbs-core --locked
    ... cargo test -p tbs-app --features devtools --locked

## Access

- Shared SSH: **not provisioned in this sandbox.** `HOPLITE_SSH_KEY_PATH` is not set
  and no private key file was mounted. Nothing was generated or requested. No
  production host or database was contacted; all work here is local and synthetic.
- No production writes, migrations, deployments or data exports were made.

## Milestone 1 — baseline verified (2026-11-11)

Clean clone of `a83b044`, `git status --porcelain --untracked-files=all` empty.

| Command | Result |
| --- | --- |
| `cargo test -p tbs-core --locked` | exit 0: lib 108 passed, 0 failed; `tests/live_backend.rs` 1 passed; `tests/live_writes.rs` 1 passed; doc-tests 0 |
| `cargo test -p tbs-app --features devtools --locked` | exit 0: 235 passed, 0 failed (build 12m 16s) |

Limits of this result:

- `live_backend`, `live_writes` and `app::live_ui::the_screens_do_what_their_buttons_say`
  return early without `TBS_E2E_*` configuration. They pass but are **no integration coverage**.
- The Linux build prints `windres failed, the executable has no icon`: `build.rs`
  calls `windres`, which the image has only as `x86_64-w64-mingw32-windres`. A warning, not a failure.
- Tests print `XDG_RUNTIME_DIR is invalid or not set` on stderr; the container has no
  desktop session. They do not fail for it.
- Windows cross-build, real-window run and screenshots: not yet done at this milestone.

## Milestone 2 — P3 filters completed (2026-11-11)

The filter bar and sheet were already implemented in the baseline; this milestone
verifies them, adds the checks that were missing, and fixes what the pictures showed.

What each requirement rests on (tests are in `tbs-app`, module
`screens::transactions::filters::tests` unless another module is named):

| Requirement | Evidence |
| --- | --- |
| Chips with individual removal and clear-all; no empty bar | `a_chip_takes_its_filter_off_at_once`, `the_bar_shows_what_is_on_and_nothing_when_nothing_is`, `resetting_on_the_bar_keeps_the_search`, `facets::tests::*`; picture `p3-bar.png` |
| Collapsible groups, summaries, per-group clear | `the_sheet_folds_and_marks_its_groups`, `a_folded_group_says_what_is_chosen_in_it`, `clearing_a_group_leaves_the_other_groups` |
| Date presets and manual dates | `each_period_fills_its_dates`, `periods_cross_months_and_years`, `a_period_fills_the_form_and_typing_still_works` |
| Searchable multi-selects, chosen first, long lists | new `ui::tests::*` (4 tests of the list itself), `the_sheet_fits_the_smallest_window_with_long_names` (cut at 100 with a note) |
| Live count scoped by search and year; no count for a draft that does not read | `the_counter_follows_the_draft`, `the_counter_says_when_the_draft_cannot_be_counted`, `the_counter_takes_the_search_and_the_year_into_account`, `a_locked_period_keeps_its_filters_and_hides_its_rows` |
| Apply / Reset / Esc, draft kept with feedback | `apply_with_an_error_keeps_the_sheet_and_the_draft`, `escape_keeps_a_draft_nobody_applied`, `resetting_the_sheet_clears_every_group_and_waits_for_apply` |
| Draft consistent after chips, quick chips, saved views | `a_draft_does_not_outlive_the_filters_it_was_made_from`, `filters_changed_under_an_open_sheet_restart_its_form`, new `a_draft_that_goes_with_its_filters_says_so` |
| Stored `Filters` and table state compatible | `the_fixtures_of_the_old_build_still_load`, `a_saved_table_state_opens_with_its_filters_and_is_saved_as_it_was` |
| Large tables | `the_table_filters_a_big_table_once_for_a_change_of_the_draft` (100 000 rows) |
| Small windows and long labels | new `the_tools_above_the_table_fit_every_window`, new `the_sheet_fits_the_smallest_window_with_long_names`, `many_chips_wrap_inside_the_page` (now at 1024×640 with the rail) |

Changes made in this milestone:

- **Narrow window fix.** At the minimum work window (1024×640, rail open) the search
  box was squeezed to about 60 px. Below a page width of 1090 px the tools above
  the table now take two lines (type chips, «Сначала новые», «Вид» above; search,
  help, «Фильтры» below); the Export/Help panels open under the taller toolbar.
- **Empty-table banner.** Its button reset the whole view, layout included. It now
  says «Фильтры и поиск скрыли все строки» and its button «Сбросить фильтры и поиск»
  takes off filters and search only (`filters::Msg::ShowAll`). The banner helper
  wraps a long hint instead of pushing its button out of a narrow page.
- **Dropped-draft feedback.** When a parked filter draft is discarded because the
  table filters changed elsewhere (chip, quick type chip, saved view), a toast says so.
- **Test bench.** `filters::tests::Bench` is shared (`pub(super)`), draws the real
  navigation rail beside the page (`App::shell_for_tests`), can resize the window,
  press buttons by their words and locate text boxes and the grid.
- Two helpers used only by tests (`Field::title`, `FilterDraft::to_filters`) are
  now `#[cfg(test)]`, which removes two dead-code warnings from the normal build.

Verification in the working tree before the commit (Docker image as above):

| Command | Result |
| --- | --- |
| `cargo test -p tbs-app --features devtools --locked` | 243 passed, 0 failed (235 baseline + 4 list tests + 4 filter tests) |
| `cargo build -p tbs-app --locked` | builds; 1 warning left (`use_ledger_columns` unused until P4 uses it) |

Clean-clone results for the pushed commit are recorded under "Verification log".

Pictures (synthetic rows only, generated by the test, no production data):

    TBS_SHOTS=<folder> cargo test -p tbs-app --features devtools --locked p3_shots

writes 12 PNG files; five are kept in `desktop/docs/screenshots/`:
`p3-bar.png` (chips above the table, 1280×860), `p3-sheet-open.png` (open groups,
unapplied edit, live count), `p3-sheet-error.png` (inline errors after a refused
«Применить»), `p3-small-bar.png` and `p3-small-sheet-list.png` (1024×640, long
names, a list of 136 operators cut at 100).

Known limits of the pictures:

- They are drawn by the simulator with wgpu on Mesa software Vulkan in the Linux
  image, not by a Windows GPU, and with the image fonts (Noto/DejaVu) instead of
  Segoe UI, so widths differ slightly from Windows.
- «Операция» shows an empty box before the word: the ledger mark is the emoji 🔴,
  and the image has no emoji font. Not checked on Windows in this session.
- `ICED_TEST_BACKEND=tiny-skia` also draws, but lets the text of a cut grid column
  run past the grid scrollbar; the wgpu pictures do not show this.

Not changed on purpose: a parked draft is still discarded (now with a toast) when
the table filters change elsewhere, rather than merged with the change.

## Milestone 3 — «Операция» goes by the stored type (owner request, 2026-11-11)

Contract given by the owner after the backend branch was ready, and consumed in
`tbs-core` only:

- Transaction responses may carry `stored_transaction_type: string | null` and
  `excel_bank: string | null`.
- «Операция», when the receipt has no headline of its own, is worded by
  `stored_transaction_type` when it is non-null, otherwise by `transaction_type`
  (`operation::stored_kind`). The cash-withdrawal mark follows the same type.
- The bank still comes from `excel_bank` when it is `CardXabar` or `HUMO Card`,
  otherwise from `source_display`. Unchanged.
- «Тип», the type filter and the quick type chips are unchanged: they read
  `transaction_type_display` / `transaction_type`.
- An older server that omits the field, and a server that sends `null`, give
  exactly the cells they gave before.
- A non-null stored type is used as it is, also one the workbook has no word
  for (`""`, an unknown code): the cell is then the workbook's own last resort,
  not the server's reading.

Side effect to know at integration: the local cache stamp is derived from the
fields of `Transaction`, so the first start of a build with this field drops the
local row cache once and reads the rows from the server again.

Tests added (`tbs-core`, 108 → 116 lib tests):

| Kind | Tests |
| --- | --- |
| Deserialization | `model::tests::the_stored_type_is_optional`, `model::tests::a_page_reads_with_the_new_fields_and_without_them` |
| Fallback | `operation::tests::the_stored_type_words_the_operation_when_the_server_sends_it`, `an_older_server_leaves_the_operation_to_the_type_it_sends`, `a_withdrawal_is_told_by_the_stored_type`, `the_stored_type_leaves_the_headline_the_notice_and_the_bank_alone` |
| Display | `columns::tests::the_operation_goes_by_the_stored_type_and_the_type_keeps_the_servers_word`, `query::tests::the_stored_type_reaches_the_cell_the_search_and_the_sort` |
| Cache | `store::tests::a_cache_from_a_build_with_other_fields_is_dropped` now also requires the new field in the stamp |

Not verified here: the backend branch itself. It is in the other session's
repository and was not fetched or merged; the field names are the owner's.
A live check against a server that sends the fields belongs to integration.

## Milestone 4 — P4 «Вид» completed (2026-11-11)

Code: `881d938` (the sheet) and `2fad9f1` (the toolbar button sends the sheet's own
message, which removes a dead-code warning from the normal build).
`screens/transactions/view.rs` is new; the view popup, its messages and the saved
views left `menus.rs`.

| Plan item | What is there |
| --- | --- |
| P4.1 sheet | «Вид» on the toolbar opens the side sheet «Вид таблицы» with the tabs Столбцы · Отображение · Сохранённые виды, on the tab it was left on. The sheet has no draft: a press changes the table at once and is saved, like a drag on a column header. Esc closes an open question first, then the sheet. |
| P4.2 Столбцы | Search by title (narrows the list, never the table); a tick shows or hides a column (the row number cannot be hidden); arrows move a column one place and are not drawn while the search narrows the list; the first shown columns carry «закреплён»; «Показать все», «Как в книге Excel» (the ledger's set and order, widths stay), «Сбросить ширину», «Скрыть все»; the counter «Показано N из 23». |
| P4.3 Отображение | Density, rows per page, pinned columns 0–3, «Сначала новые» (one setting with the toolbar button) with the current order said in words. |
| P4.4 Сохранённые виды | Save (the same name replaces the view, and the button then says «Заменить вид»), apply, rename (an empty or taken name is refused under the box), default (one at most; pressed again, the mark comes off), delete (asks first). The view that equals the table is marked «применён». |
| «Сбросить вид» | Asks first, then resets the layout only: columns, widths, column formatting, density, pinning, sorting, page size. Filters, search, an unapplied filter draft, hand-formatted cells and saved views stay. |

Differences from the plan and from the build before, to know at integration:

- The counter says «из 23», not «из 22» as the plan words it: «Операция» is the
  23rd column, and the number is `columns::COUNT`.
- «Сбросить вид» used to reset filters and search too, at once and without a
  question. It is the layout only now; filters have «Сбросить всё» on their bar.
- Saved views keep their file and format (`transactions-presets`). They are now
  repaired on load the way the table's own state is, so a view saved by an older
  build gets today's columns; `fixtures/presets.json` holds three such views.
- A default view starts only a table that has no saved state of its own. The
  rule is unchanged and now has a test.
- `Popup::View` and `menus::Msg::{ToggleColumn, ShowAll, HideAll, Density, Pinned,
  Reset, Preset*}` are gone; `view::Msg` replaces them. Devtools: `TBS_OPEN=view`
  opens the sheet on Столбцы, `view-display` and `view-saved` on the other tabs.
- New helper `ui::btn_small`, a lower text button for a line that holds several.
- The arrows step past hidden columns too: the list shows every column, and one
  press is one place in it. As with a drag on a header, a column may be put
  before the row number; only hiding the row number is refused.
- The `use_ledger_columns` warning noted in milestone 2 is gone: «Как в книге
  Excel» uses it.

Tests added (`tbs-app`, 243 → 264):

| Area | Tests |
| --- | --- |
| Move, reset, ledger order (`table::tests`) | `a_column_moves_one_place_and_stops_at_the_ends`, `a_hidden_column_is_a_place_in_the_order_too`, `resetting_the_layout_keeps_the_filters_and_the_painted_cells`, `a_fresh_layout_is_told_from_one_that_was_arranged`, `pinned_columns_are_counted_among_the_shown` |
| Sheet and tabs (`view::tests`) | `the_sheet_opens_on_the_tab_it_was_left_on`, `a_tick_shows_and_hides_a_column_at_once`, `arrows_move_a_column_and_the_table_follows`, `the_search_narrows_the_list_and_leaves_the_table_alone`, `show_all_the_ledger_and_the_widths_each_have_a_button`, `the_tab_of_display_changes_the_table_at_once`, `the_tab_of_display_offers_every_page_size_and_pin_count`, `the_order_of_the_rows_and_a_saved_view_are_said_in_words`, `escape_closes_the_question_first_and_then_the_sheet` |
| Reset and filters | `resetting_the_view_asks_first_and_leaves_what_narrows_the_table`, `applying_a_view_drops_a_filter_draft_and_opening_the_sheet_keeps_it`; `filters::tests::a_draft_does_not_outlive_the_filters_it_was_made_from` now expects a reset of the look to keep the draft |
| Saved views | `a_view_is_saved_applied_renamed_made_default_and_deleted`, `the_views_saved_by_older_builds_load_and_apply`, `a_default_view_starts_a_table_that_has_no_state_of_its_own`; `saving_a_name_again_replaces_the_view` and `only_one_preset_stays_default` moved here from `menus.rs` |
| Small window | `the_sheet_fits_the_smallest_window` (1024×640: the controls of the three tabs stand inside the sheet, the list scrolls between head and foot, a long view name is cut so its buttons keep their place) |
| Pictures | `p4_shots` (returns at once without `TBS_SHOTS`) |

Pictures (synthetic rows only, generated by the test, no production data):

    TBS_SHOTS=<folder> cargo test -p tbs-app --features devtools --locked p4_shots

writes 12 PNG files. It was run on the clean clones of `881d938` and `2fad9f1`, and
both runs gave the same bytes for the seven kept in `desktop/docs/screenshots/`:
`p4-columns-arranged.png` (Столбцы after arranging by hand: one column shown, one
hidden, «Сумма» moved up beside the date, two pinned), `p4-display.png`,
`p4-saved.png` (a default view that is on, a long name, «Заменить вид»),
`p4-reset-ask.png` (the question before the reset), `p4-table-reset.png` (the
table after it: the default layout, both filters still on), `p4-small-columns.png`
and `p4-small-display-dark.png` (1024×640). All seven were looked at. The limits
of the P3 pictures apply: software Vulkan, the image fonts, no emoji font.

## Milestone 5 — final desktop checks (2026-11-11)

Clean clone of `2fad9f1`, `git status --porcelain --untracked-files=all` empty,
image `tbs-linux-dev:1.99.1`:

| Command | Result |
| --- | --- |
| `cargo test -p tbs-core --locked` | exit 0: lib 116 passed, 0 failed; `tests/live_backend.rs` 1 passed; `tests/live_writes.rs` 1 passed; doc-tests 0 |
| `cargo test -p tbs-app --features devtools --locked` | exit 0: 264 passed, 0 failed |
| `cargo build -p tbs-app --locked` | exit 0, no compiler warnings, only the `windres` warning of the build script. `tbsparcer`: ELF x86-64, 283 329 416 bytes |
| `cargo build -p tbs-app --locked --target x86_64-pc-windows-gnu` | exit 0 (11m 55s cold on `881d938`, 18 s after it). `tbsparcer.exe`: PE32+ x86-64, 259 235 719 bytes, `WebView2Loader.dll` beside it |
| The Linux binary under Xvfb (1280×860×24) for 30 s | still running after 30 s, nothing on stdout or stderr, stopped by SIGTERM. The framebuffer shows the sign-in window drawn in full: «Вход в Чеки», «Сервер недоступен», version 2.0.0 |

The headless run, inside the image, with an empty `HOME` and no server:

    Xvfb :99 -screen 0 1280x860x24 -fbdir /tmp/fb -nolisten tcp &
    DISPLAY=:99 /target/debug/tbsparcer

`/tmp/fb/Xvfb_screen0` is the framebuffer as an XWD image; converted to PNG it is
`desktop/docs/screenshots/smoke-login-linux.png`.

What these checks do and do not show:

- `live_backend`, `live_writes` and `live_ui` return early without `TBS_E2E_*`.
  No server was started or contacted: **no integration coverage**.
- The Windows build shows that the dev profile compiles and links for
  `x86_64-pc-windows-gnu` with the MinGW of the image. The executable was **not
  run**: there is no Windows and no Wine here. Nothing about its behaviour on
  Windows follows from it (GPU rendering, Segoe UI widths, emoji, WebView2, the
  keyring, updates). The release profile and the installer were not built.
- The cross-built executable has no icon and no version resource (no `.rsrc`
  section): `build.rs` runs `windres`, which the image has only as
  `x86_64-w64-mingw32-windres`. A build on Windows with MinGW's `windres` on the
  path is not affected, and `build.rs` was left as it is.
- The headless run reaches the sign-in window only. Without a server there is no
  session, so the table, the filter sheet and «Вид» were not seen in a real
  window: their pictures come from the test simulator with synthetic rows.
- Times in this file are the sandbox clock's.

Left for integration, not done here:

- A run on Windows: the real window with its GPU, fonts and emoji, and the installer.
- A live check against a server that sends `stored_transaction_type` and `excel_bank`.
- `IMPLEMENTATION_PLAN.md` still shows P3 and P4 unticked: it is not this
  session's file to edit.
- `desktop/tools/linux-dev.Dockerfile` points to `desktop/README.md` for its use,
  and the README has no such section; the commands are under "Environment" above.
- No branch was merged into this one, and this one into none.

## Plan of this session

1. P3 filters: verify the existing implementation, add the missing checks
   (searchable lists, small windows, long labels, saved-view fixture), fix what the
   pictures show, capture screenshots.
2. P4 «Вид»: side sheet with Столбцы · Отображение · Сохранённые виды; layout reset
   that leaves filters alone; tests and screenshots.
3. Final checks: clean-clone core and app tests, Windows GNU cross-build, a
   headless run of the real binary, record exact results here.

## Status

- [x] Baseline verified on a clean clone (milestone 1).
- [x] P3 filters completed and verified (milestone 2).
- [x] «Операция» fallback by `stored_transaction_type` with tests (milestone 3).
- [x] P4 View implemented and verified on a clean clone (milestone 4).
- [x] Final desktop checks: clean-clone tests, normal build, Windows GNU
  cross-build, headless start of the real binary (milestone 5).

The code verified is `2fad9f1`. The commit after it adds only this file's notes
and the pictures under `desktop/docs/screenshots/`.

## Verification log

| When (UTC) | Commit | Checkout | Command | Result |
| --- | --- | --- | --- | --- |
| 2026-11-11 18:47 | `a83b044` | clean clone | `cargo test -p tbs-core --locked` | 108 + 1 + 1 passed, exit 0 |
| 2026-11-11 18:59 | `a83b044` | clean clone | `cargo test -p tbs-app --features devtools --locked` | 235 passed, exit 0 |
| 2026-11-11 19:13 | `62c86e6` | clean clone | `cargo test -p tbs-core --locked` | 108 + 1 + 1 passed, exit 0 |
| 2026-11-11 19:13 | `62c86e6` | clean clone | `cargo test -p tbs-app --features devtools --locked` | 243 passed, exit 0 |
| 2026-11-11, 19:27–19:30 | `85403e7` | clean clone | `cargo test -p tbs-core --locked` | 116 + 1 + 1 passed, exit 0 (the minute was not written down) |
| 2026-11-11, 19:27–19:30 | `85403e7` | clean clone | `cargo test -p tbs-app --features devtools --locked` | 243 passed, exit 0 (the minute was not written down) |
| 2026-11-11 19:42 | `881d938` | clean clone | `cargo test -p tbs-core --locked` | 116 + 1 + 1 passed, exit 0 |
| 2026-11-11 19:42 | `881d938` | clean clone | `cargo test -p tbs-app --features devtools --locked` | 264 passed, exit 0 |
| 2026-11-11 19:43 | `881d938` | clean clone | `cargo build -p tbs-app --locked` | exit 0, 1 dead-code warning (`view::Msg::Open`), fixed in `2fad9f1` |
| 2026-11-11 19:54 | `881d938` | clean clone | `cargo build -p tbs-app --locked --target x86_64-pc-windows-gnu` | exit 0, cold, 11m 55s |
| 2026-11-11 19:58 | `2fad9f1` | clean clone | `cargo test -p tbs-core --locked` | 116 + 1 + 1 passed, exit 0 |
| 2026-11-11 19:59 | `2fad9f1` | clean clone | `cargo test -p tbs-app --features devtools --locked` | 264 passed, exit 0 |
| 2026-11-11 19:59 | `2fad9f1` | clean clone | `cargo build -p tbs-app --locked` | exit 0, no compiler warnings |
| 2026-11-11 19:59 | `2fad9f1` | clean clone | `cargo build -p tbs-app --locked --target x86_64-pc-windows-gnu` | exit 0, `tbsparcer.exe` 259 235 719 bytes, not run |
| 2026-11-11 20:01 | `2fad9f1` | its Linux binary | `tbsparcer` under Xvfb for 30 s | alive, sign-in window drawn, no output |
