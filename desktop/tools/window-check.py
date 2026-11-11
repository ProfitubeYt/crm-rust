#!/usr/bin/env python3
"""Shows the client in a real window where there is no desktop to show it on:
Xvfb for the screen, stub-server.py for the server, a keyring on a private
D-Bus for the saved session, xdotool for the mouse and the keyboard. The
pictures it takes are of that screen, not of the test simulator.

Runs inside the image of linux-dev.Dockerfile with these added as root:

    apt-get install -y xdotool gnome-keyring

and a client built with `cargo build -p tbs-app --locked`. `CLIENT` names
another one, a command line included: `CLIENT="wine …/tbsparcer.exe"` runs the
Windows build, which keeps its session in Wine's own credential store, so the
keyring is not needed there; the README says what Wine needs first. `STUB` adds
options for the stub, `SETTLE` and `PATIENCE` give a slow start more time.

    window-check.py up              the screen, the keyring and the stub
    window-check.py start           the client; `stop` ends it, the session stays saved
    window-check.py shot NAME       the whole screen into shots/NAME.png
    window-check.py click X Y       at a point of the client's window, from its corner
    window-check.py move X Y        the pointer only; `drag X Y X2 Y2` presses, moves, lets go
    window-check.py type TEXT       as from the keyboard; `key NAME` for one key
    window-check.py wheel X Y up|down|left|right [TIMES]
    window-check.py window          the client's window: its name and where it is
    window-check.py stub PATH       POST to the stub, e.g. `stub /_stub/revoke`
    window-check.py down            everything, and what was saved

Several steps can go on one line, with `wait SECONDS` between them:

    window-check.py click 617 354 wait 2 shot 03-signed-in
"""

import os
import shlex
import shutil
import subprocess
import sys
import time
import urllib.request
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOM = Path(os.environ.get("ROOM", "/tmp/tbs-window-check"))
SHOTS = Path(os.environ.get("SHOTS", ROOM / "shots"))
CLIENT = os.environ.get("CLIENT", os.path.join(os.environ.get("CARGO_TARGET_DIR", "/target"), "debug", "tbsparcer"))
PORT = os.environ.get("PORT", "8000")
TITLE = "^TBS inform CRM$"
# Wine names the process after its main thread, so the command line is matched,
# and its end only: a shell that merely mentions the client is left alone.
CLIENT_PROCESS = r"tbsparcer(\.exe)?$"
WHEEL = {"up": "4", "down": "5", "left": "6", "right": "7"}

ENV = dict(
    os.environ,
    DISPLAY=os.environ.get("DISPLAY", ":99"),
    DBUS_SESSION_BUS_ADDRESS=f"unix:path={ROOM}/bus",
    XDG_RUNTIME_DIR=str(ROOM / "run"),
)


def run(*command, **more):
    return subprocess.run(command, env=ENV, check=True, text=True, **more)


def spawn(log, *command, **env):
    """Left running after this script ends."""
    with open(ROOM / log, "ab") as out:
        subprocess.Popen(command, env=dict(ENV, **env), stdout=out, stderr=out, stdin=subprocess.DEVNULL, start_new_session=True)


def window():
    found = subprocess.run(["xdotool", "search", "--onlyvisible", "--name", TITLE], env=ENV, text=True, capture_output=True).stdout.split()
    return found[-1] if found else None


def up(_):
    keyrings = Path.home() / ".local/share/keyrings"
    for folder in (ROOM / "data", SHOTS, ROOM / "run", Path.home() / ".cache", keyrings):
        folder.mkdir(parents=True, exist_ok=True)
    (ROOM / "run").chmod(0o700)
    spawn("xvfb.log", "Xvfb", ENV["DISPLAY"], "-screen", "0", os.environ.get("SCREEN", "1600x1000x24"), "-nolisten", "tcp")
    if shutil.which("gnome-keyring-daemon"):
        spawn("dbus.log", "dbus-daemon", "--session", f"--address={ENV['DBUS_SESSION_BUS_ADDRESS']}", "--nofork", "--nopidfile")
        time.sleep(2)
        # The keyring's own password: it guards nothing but this check's session.
        run("gnome-keyring-daemon", "--unlock", "--components=secrets", input="window-check", stdout=subprocess.DEVNULL)
    stub_options = shlex.split(os.environ.get("STUB", ""))
    spawn("stub.log", sys.executable, str(HERE / "stub-server.py"), "--port", PORT, "--sessions", str(ROOM / "sessions.json"), *stub_options)
    time.sleep(1)


def start(_):
    spawn("client.log", *shlex.split(CLIENT), TBS_DATA_DIR=str(ROOM / "data"), TBS_API_BASE_URL=f"http://127.0.0.1:{PORT}")
    for _ in range(int(os.environ.get("PATIENCE", "60"))):
        if window():
            break
        time.sleep(0.5)
    time.sleep(float(os.environ.get("SETTLE", "3")))


def stop(_):
    subprocess.run(["pkill", "-f", CLIENT_PROCESS])
    time.sleep(1)


def shot(args):
    run("import", "-window", "root", str(SHOTS / f"{args.pop(0)}.png"))


def at(args):
    """Moves the pointer to a point of the client's window."""
    x, y = args.pop(0), args.pop(0)
    run("xdotool", "mousemove", "--window", window(), x, y)


def click(args):
    at(args)
    run("xdotool", "click", "1")


def drag(args):
    at(args)
    run("xdotool", "mousedown", "1")
    time.sleep(0.3)
    at(args)
    time.sleep(0.3)
    run("xdotool", "mouseup", "1")


def focus():
    """There is no window manager here to hand the keyboard to the window."""
    run("xdotool", "windowfocus", window())
    time.sleep(0.2)


def type_text(args):
    focus()
    run("xdotool", "type", "--delay", "60", args.pop(0))


def key(args):
    focus()
    run("xdotool", "key", args.pop(0))


def wheel(args):
    at(args)
    button = WHEEL[args.pop(0)]
    times = args.pop(0) if args and args[0].isdigit() else "1"
    run("xdotool", "click", "--repeat", times, "--delay", "80", button)


def show_window(_):
    run("xdotool", "getwindowname", window())
    run("xdotool", "getwindowgeometry", window())


def stub(args):
    request = urllib.request.Request(f"http://127.0.0.1:{PORT}{args.pop(0)}", data=b"{}", method="POST")
    print(urllib.request.urlopen(request).read().decode())


def wait(args):
    time.sleep(float(args.pop(0)))


def down(_):
    subprocess.run(["pkill", "-f", CLIENT_PROCESS])
    for name in ("gnome-keyring-d", "dbus-daemon", "Xvfb"):
        subprocess.run(["pkill", "-x", name])
    subprocess.run(["pkill", "-f", "stub-server.py"])
    for left in (ROOM / "data", ROOM / "run", Path.home() / ".local/share/keyrings"):
        shutil.rmtree(left, ignore_errors=True)
    for left in (ROOM / "sessions.json", ROOM / "bus"):
        left.unlink(missing_ok=True)


STEPS = {
    "up": up, "start": start, "stop": stop, "shot": shot, "move": at, "click": click, "drag": drag, "type": type_text, "key": key,
    "wheel": wheel, "window": show_window, "stub": stub, "wait": wait, "down": down,
}


def main():
    args = sys.argv[1:]
    if not args or args[0] not in STEPS:
        print(__doc__)
        sys.exit(2)
    while args:
        STEPS[args.pop(0)](args)


if __name__ == "__main__":
    main()
