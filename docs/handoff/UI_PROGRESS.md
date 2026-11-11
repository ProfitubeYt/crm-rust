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
- [ ] P3 filters completed and verified.
- [ ] P4 View implemented and verified.
- [ ] Final desktop checks (clean-clone tests, Windows cross-build, headless run).
