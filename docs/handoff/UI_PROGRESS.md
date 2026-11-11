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
- [ ] P4 View: implemented in the commit that follows `85403e7`. The working tree passed
  `cargo test -p tbs-app --features devtools --locked` with 264 tests before the last
  wording change of the reset question. Clean-clone runs, pictures and the milestone
  notes come with the next commit.
- [ ] Final desktop checks (clean-clone tests, Windows cross-build, headless run).

## Verification log

| When (UTC) | Commit | Checkout | Command | Result |
| --- | --- | --- | --- | --- |
| 2026-11-11 18:47 | `a83b044` | clean clone | `cargo test -p tbs-core --locked` | 108 + 1 + 1 passed, exit 0 |
| 2026-11-11 18:59 | `a83b044` | clean clone | `cargo test -p tbs-app --features devtools --locked` | 235 passed, exit 0 |
| 2026-11-11 19:13 | `62c86e6` | clean clone | `cargo test -p tbs-core --locked` | 108 + 1 + 1 passed, exit 0 |
| 2026-11-11 19:13 | `62c86e6` | clean clone | `cargo test -p tbs-app --features devtools --locked` | 243 passed, exit 0 |
