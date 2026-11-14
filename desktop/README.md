# TBS inform CRM, native client

The desktop client in Rust: iced for the interface, WebView2 only for the
Telegram section, whose page is built from `frontend/` and embedded at compile
time. The Electron client in `frontend/` stays until the switch.

The program is called TBS inform CRM wherever a person reads its name: on its
window, on the start window every launch opens with, in the setup. A workspace
opened from the start window goes by its own name: «Чеки» in its rail; «Учёт
времени» and «Администрирование CRM», which are pages, in one slim bar over the
page, with the way back to the start window, the connection mark, the account
and the sign-out. A page has no rail.
The executable, the data folder, the keyring entry and the setup's file name keep
`tbsparcer`: installed clients find their settings and their updates by them.

- `crates/tbs-core`: API, cache, exports, no interface. A future CRM can reuse it.
- `crates/tbs-app`: the program.
- `installer/`, `tools/build-installer.ps1`: the setup.

## Requirements

Rust on the `stable-x86_64-pc-windows-gnu` toolchain, MinGW with `windres`,
Node, Inno Setup (`iscc`). Keep `CARGO_TARGET_DIR` outside the repository.

## Build and test

    npm --prefix ../frontend run build:telegram
    cargo run -p tbs-app
    cargo test -p tbs-core
    cargo test -p tbs-app --features devtools

`--features devtools` adds the `TBS_AUTOLOGIN`, `TBS_GOTO`, `TBS_OPEN`, `TBS_SHOT`
and `TBS_CMD` hooks. Release builds do not have them.

`live_writes` (`TBS_E2E_WRITES=1`) and the `live_ui` flow (`TBS_E2E_LOGIN=<login.json>`)
change data. Run them against a local copy of the database, never the server.

## A window without Windows

`tools/linux-dev.Dockerfile` builds and tests the client on Linux. To see the
real window there, with no server and no desktop at hand:

    python3 tools/window-check.py up start shot first-launch
    python3 tools/window-check.py click 610 178 type anna click 610 253 type anna-secret key Return wait 9 shot table
    python3 tools/window-check.py down

`tools/stub-server.py` stands in for the backend (sign-in, session, eighty made-up
rows; it is not the backend and checks nothing a server would), Xvfb is the
screen, a keyring on a private D-Bus keeps the saved session, and xdotool is the
mouse and the keyboard. The pictures are of that screen, not of the test
simulator. `window-check.py` lists what it needs installed and every step.

The stub knows nothing of workspaces. To see the Hub, «Учёт времени» and the
administration panel, point the client at the stack of `scripts/hub/e2e` with
`SERVER=http://127.0.0.1:<port>`: the client takes plain http on loopback only,
so from another container forward a local port to the stack's Caddy first
(`socat TCP-LISTEN:18080,bind=127.0.0.1,fork TCP:caddy:80`). Those pages are
web views: on Linux that is WebKitGTK, which `screens/hosted/host.rs` starts
and keeps running, and it is not the WebView2 the Windows build shows them in.

The same steps drive the Windows build under Wine, which stands in for Windows
without being it. Two things first: the client reads `C:\windows\Fonts` itself
and a new Wine prefix has none there, so its window comes up without a word;
and left to itself it draws through DX12, whose shaders Wine cannot compile, so
it is told to use Vulkan.

    python3 tools/window-check.py up
    DISPLAY=:99 wineboot --init
    cp /usr/share/fonts/truetype/dejavu/*.ttf ~/.wine/drive_c/windows/Fonts/
    CLIENT="wine $CARGO_TARGET_DIR/x86_64-pc-windows-gnu/debug/tbsparcer.exe" \
    WGPU_BACKEND=vulkan WINEDEBUG=-all SETTLE=12 \
        python3 tools/window-check.py start shot first-launch

## Setup

    pwsh tools/build-installer.ps1

The result is `C:\Dev\tmp\tbs-installer\TBSparcer-Native-Setup-<version>.exe`. It
installs per user, without elevation, which an update started from inside the
program relies on. It contains `frontend/client-access.json` with the system
access token, so it must be handled like the Electron setup and never committed.

Without Windows the same two files can be made on Linux, for a look at the result
and nothing more:

    cargo build -p tbs-app --release --locked --target x86_64-pc-windows-gnu
    docker run --rm -v "$PWD/..":/work:ro -v <dir with tbsparcer.exe and WebView2Loader.dll>:/build:ro \
        -v <output dir>:/out amake/innosetup /DAppVersion=<version> '/DBuildDir=Z:\build' \
        '/DOutputDirectory=Z:\out' 'Z:\work\desktop\installer\tbsparcer.iss'

`build.rs` finds MinGW's `x86_64-w64-mingw32-windres` there, so the executable
has its icon and version. A setup made without `frontend/client-access.json`
only updates an installed client. Neither file has run on Windows by being
built: do not publish such a setup. `.github/workflows/windows-client.yml`
builds, tests, installs and starts the client on a hosted Windows runner when
started by hand, and keeps the setup and a picture of the first start. It does
not sign in, so it shows nothing of the pages in WebView2.

The client looks for updates in `asintiko/parcer20-updates` and takes only an
asset named `TBSparcer-Native-Setup-*.exe`, so the Electron releases there do not
reach it. The Telegram section needs the WebView2 runtime on the machine.
