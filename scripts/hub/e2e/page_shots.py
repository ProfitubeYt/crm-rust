#!/usr/bin/env python3
"""Looks at the Time page in a headless Chromium through page_host.py: takes
pictures at the sizes the program's window can have, and measures what a
picture cannot be trusted for: whether the header keeps to one line, whether
anything is cut off, what a viewer is offered and what a viewer's page sends.

Chromium is the family WebView2 belongs to. It is not WebView2, and the page
comes from a stand-in for the program, so this says how the page lays out and
behaves, not how the program's window does. Run in the Playwright image on the
stack's network, after `run.sh start` and `run.sh page`:

    docker run --rm --network hub-e2e_default -v "$PWD":/work -w /work \\
        hub-page-shots:pinned \\
        python scripts/hub/e2e/page_shots.py --out output/hub-e2e/page [--standalone http://host:8080]

`--standalone` adds the page as Time serves it by itself, to a browser, with
no Hub around it. Exit code 1 if a check fails.
"""

import argparse
import sys
from pathlib import Path

from playwright.sync_api import TimeoutError as PlaywrightTimeout
from playwright.sync_api import sync_playwright

HOST = "http://pagehost"
MANAGER, VIEWER = 8101, 8102
TABS = ("timelist", "tabel", "svodka", "telegram", "periods", "refs")
# The program's work window is never under 1024 by 640; its own bar takes 41 of the height.
SMALLEST = (1024, 599)
USUAL = (1360, 819)

MEASURE = """() => {
  const bar = document.querySelector('.topbar');
  const shown = (el) => !!el && el.getClientRects().length > 0 && getComputedStyle(el).visibility !== 'hidden';
  // Lines of the header: a child that starts below everything before it starts a new one.
  let rows = 0, floor = -1;
  for (const box of [...bar.children].filter(shown).map((el) => el.getBoundingClientRect()).sort((a, b) => a.top - b.top)) {
    if (box.top >= floor - 2) rows += 1;
    floor = Math.max(floor, box.bottom);
  }
  const cut = [...bar.querySelectorAll('*')].filter((el) => shown(el) && el.getBoundingClientRect().right > innerWidth + 0.5).length;
  const tabs = document.querySelector('#tabs');
  const strip = tabs.getBoundingClientRect();
  const tools = document.querySelector('.toolbar');
  const edits = (root) => root ? [
    ...[...root.querySelectorAll('button:not([data-ro]), a.btn:not([data-ro]), [data-reset]')].filter(shown).map((el) => (el.id || el.textContent || el.title).trim().slice(0, 30)),
    ...[...root.querySelectorAll('input:not([data-ro]), select:not([data-ro]), textarea:not([data-ro])')].filter((el) => shown(el) && !el.disabled).map((el) => 'field ' + (el.name || el.id || el.dataset.field || el.type)),
  ] : [];
  return {
    header: Math.round(bar.getBoundingClientRect().height),
    headerRows: rows,
    cut,
    tabsScroll: tabs.scrollWidth > tabs.clientWidth + 1,
    tabsSeen: [...tabs.children].filter((el) => el.getBoundingClientRect().right <= Math.min(strip.right, innerWidth) + 0.5 && el.getBoundingClientRect().left >= strip.left - 0.5).length,
    toolbar: tools ? Math.round(tools.getBoundingClientRect().height) : 0,
    sideways: document.documentElement.scrollWidth > innerWidth + 1,
    brand: shown(document.querySelector('.brand')),
    account: shown(document.querySelector('#user-btn')),
    readOnlyChip: shown(document.querySelector('#ro-chip')),
    edits: [...edits(document.querySelector('#main')), ...edits(document.querySelector('#modal:not([hidden]) #modal-body'))],
  };
}"""

failed = []


def check(name, ok, detail=""):
    print(f"  {'ok  ' if ok else 'FAIL'}  {name}" + (f" -- {detail}" if detail and not ok else ""), flush=True)
    if not ok:
        failed.append(name)


class Look:
    """One page in one window of a given size."""

    def __init__(self, browser, base, size, theme="light", scale=1.0):
        self.context = browser.new_context(viewport={"width": size[0], "height": size[1]}, device_scale_factor=scale, locale="ru-RU")
        self.page = self.context.new_page()
        self.sent = []
        self.page.on("request", lambda request: self.sent.append((request.method, request.url.split("/api/")[-1])) if request.method != "GET" else None)
        self.base, self.theme = base, theme

    def open(self, tab="timelist"):
        self.page.goto(f"{self.base}/?theme={self.theme}#{tab}/2026-11")
        self.page.reload()
        self.settle()
        return self

    def settle(self):
        self.page.wait_for_selector("#main > *", timeout=15000)
        try:
            # By itself Time keeps a stream of events open, and the network is never idle.
            self.page.wait_for_load_state("networkidle", timeout=2500)
        except PlaywrightTimeout:
            pass
        self.page.wait_for_timeout(250)

    def tab(self, name):
        self.page.click(f'#tabs button[data-tab="{name}"]')
        self.settle()

    def measure(self):
        return self.page.evaluate(MEASURE)

    def shot(self, out, name):
        self.page.screenshot(path=str(out / f"{name}.png"))

    def close(self):
        self.context.close()


def sizes(browser, out):
    print("\n== the header and the toolbar by window width (Chromium; manager and viewer)")
    print("   width  who      header rows cut tabs-seen scroll toolbar sideways")
    for width in (640, 800, 940, 1024, 1100, 1240, 1360, 1600, 1920):
        for who, port in (("manager", MANAGER), ("viewer", VIEWER)):
            look = Look(browser, f"{HOST}:{port}", (width, 700)).open()
            m = look.measure()
            print(f"   {width:5}  {who:8} {m['header']:4}px {m['headerRows']:3} {m['cut']:4} {m['tabsSeen']:6}/6 {str(m['tabsScroll']):6} {m['toolbar']:4}px {m['sideways']}")
            if width >= 1024:
                check(f"{width} px, {who}: the header is one line, nothing cut, all six sections in sight", m["headerRows"] == 1 and m["cut"] == 0 and m["tabsSeen"] == 6 and not m["sideways"], str(m))
                check(f"{width} px, {who}: the toolbar is one line", m["toolbar"] <= 48, f"{m['toolbar']} px")
            else:
                check(f"{width} px, {who}: narrower than the program allows, still nothing cut and no sideways scroll", m["cut"] == 0 and not m["sideways"], str(m))
            check(f"{width} px, {who}: no name of its own, no account button in the page", not m["brand"] and not m["account"], str(m))
            look.close()
    # The fullest the header gets: a closed month adds its mark, a viewer adds theirs.
    closer = Look(browser, f"{HOST}:{MANAGER}", SMALLEST).open()
    closed = closer.page.evaluate("api('POST', '/months/2026-09/lock').then(() => true, (e) => e.message)")
    check("September is closed for the next two checks", closed is True, str(closed))
    for who, port in (("manager", MANAGER), ("viewer", VIEWER)):
        look = Look(browser, f"{HOST}:{port}", SMALLEST)
        look.page.goto(f"{HOST}:{port}/?theme=light#timelist/2026-09")
        look.page.reload()
        look.settle()
        m = look.measure()
        check(f"1024 px, {who}, a closed month: still one line with all six sections", m["headerRows"] == 1 and m["cut"] == 0 and m["tabsSeen"] == 6 and look.page.locator("#lock-chip").is_visible(), str(m))
        look.shot(out, f"{who}-closed-month-1024")
        look.close()
    closer.page.evaluate("api('DELETE', '/months/2026-09/lock')")
    closer.close()


def viewer(browser, out):
    print("\n== what a viewer is offered, section by section")
    look = Look(browser, f"{HOST}:{VIEWER}", SMALLEST).open()
    check("the page says it is for viewing only", look.measure()["readOnlyChip"])
    for name in TABS:
        look.tab(name)
        offered = look.measure()["edits"]
        check(f"«{name}»: nothing that edits is offered", not offered, str(offered[:6]))
        look.shot(out, f"viewer-{name}-1024")
    look.tab("timelist")
    look.page.click("table.grid tbody tr:nth-child(5) td.t")
    look.page.wait_for_timeout(600)
    offered = look.measure()["edits"]
    check("the day panel: fields cannot be typed in, no actions", not offered, str(offered[:6]))
    fields = look.page.locator("#side input").count()
    check("the day panel still shows the day's fields", fields >= 7, f"{fields} fields")
    look.shot(out, "viewer-timelist-day-1024")
    for key in ("Enter", "F2", "9", "Delete"):
        look.page.keyboard.press(key)
    look.page.dblclick("table.grid tbody tr:nth-child(6) td.t")
    look.page.wait_for_timeout(300)
    check("typing, Enter, F2, Delete and a double click open no editor", look.page.locator(".cell-editor").count() == 0)
    look.page.click("#issues-btn")
    look.page.wait_for_timeout(300)
    listed = look.page.locator(".issues-list button").count()
    check("«На проверку» opens and its list can be walked", listed > 0 and not look.measure()["edits"], f"{listed} entries")
    look.shot(out, "viewer-issues-1024")
    look.page.click(".issues-list button >> nth=0")
    look.page.wait_for_timeout(300)
    check("a viewer's page sent nothing but reads", not look.sent, str(look.sent[:5]))
    look.tab("telegram")
    look.page.fill("#probe-text", "офис 9:10")
    look.page.click("#probe-btn")
    look.page.wait_for_timeout(600)
    check("«Проверить фразу» works for a viewer and is the only thing sent", [path for _, path in look.sent] == ["time/parse"], str(look.sent))
    refused = look.page.evaluate("api('PUT', '/settings', {}).then(() => 'sent', (e) => e.message)")
    check("an edit asked of the page's own code is refused before it is sent", "только просмотр" in refused and len(look.sent) == 1, refused)
    look.close()


def manager(browser, out):
    print("\n== a manager still edits")
    look = Look(browser, f"{HOST}:{MANAGER}", SMALLEST).open()
    m = look.measure()
    check("no read-only mark for a manager", not m["readOnlyChip"])
    for control in ("#period-btn", "#lock-btn"):
        check(f"{control} is offered", look.page.locator(control).is_visible())
    cell = look.page.locator("table.grid tbody tr:nth-child(5) td.t").first
    # A time the cell does not hold yet, so that there is something to save on every run.
    typed = "9:18" if "9:17" in cell.inner_text() else "9:17"
    cell.click()
    look.page.keyboard.type(typed)
    look.page.keyboard.press("Enter")
    look.page.wait_for_timeout(700)
    check("typing into a cell saves the mark", any(method == "PUT" and "time/marks/" in path for method, path in look.sent), str(look.sent[:3]))
    check("and the cell shows it", typed in look.page.locator("table.grid tbody tr:nth-child(5)").inner_text())
    look.page.click("#period-btn")
    look.page.wait_for_timeout(300)
    check("«+ Отсутствие» opens the period form", look.page.locator("#modal-body select[name=code]").is_visible())
    look.shot(out, "manager-period-1024")
    look.page.click("#modal-close")
    look.close()


def pictures(browser, out, standalone):
    print("\n== pictures")
    plan = [
        ("manager-timelist-1024-light", MANAGER, SMALLEST, "light", 1.0, "timelist", True),
        ("manager-timelist-1024-dark", MANAGER, SMALLEST, "dark", 1.0, "timelist", True),
        ("manager-timelist-1360-light", MANAGER, USUAL, "light", 1.0, "timelist", True),
        ("manager-timelist-1360-dark", MANAGER, USUAL, "dark", 1.0, "timelist", False),
        ("manager-timelist-1280x599-150pct", MANAGER, (1280, 599), "light", 1.5, "timelist", True),
        ("manager-timelist-1920-light", MANAGER, (1920, 1000), "light", 1.0, "timelist", False),
        ("manager-timelist-800-light", MANAGER, (800, 600), "light", 1.0, "timelist", False),
        ("viewer-timelist-1360-dark", VIEWER, USUAL, "dark", 1.0, "timelist", True),
    ] + [(f"manager-{name}-1024-light", MANAGER, SMALLEST, "light", 1.0, name, False) for name in TABS[1:]]
    for name, port, size, theme, scale, tab, day in plan:
        look = Look(browser, f"{HOST}:{port}", size, theme, scale).open(tab)
        if day:
            look.page.click("table.grid tbody tr:nth-child(5) td.t")
            look.page.wait_for_timeout(500)
        look.shot(out, name)
        look.close()
        print(f"  {name}.png")
    if standalone:
        for name, size in (("standalone-1360", (1360, 800)), ("standalone-1024", (1024, 700)), ("standalone-640", (640, 800))):
            look = Look(browser, standalone, size)
            look.page.goto(f"{standalone}/#timelist/2026-11")
            look.settle()
            m = look.measure()
            check(f"by itself in a browser, {size[0]} px: its name and its own buttons are there, nothing cut", m["brand"] and m["account"] and m["cut"] == 0 and not m["sideways"], str(m))
            look.shot(out, name + "-light")
            look.page.click("#theme-btn")
            look.page.wait_for_timeout(200)
            look.shot(out, name + "-dark")
            look.close()
            print(f"  {name}-light.png, {name}-dark.png")


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--out", default="output/hub-e2e/page")
    parser.add_argument("--standalone", help="address of a Time server running by itself, without the Hub")
    parser.add_argument("--only", choices=("sizes", "viewer", "manager", "pictures"))
    args = parser.parse_args()
    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    with sync_playwright() as playwright:
        browser = playwright.chromium.launch()
        print(f"Chromium {browser.version}, headless, Linux")
        steps = {"sizes": lambda: sizes(browser, out), "viewer": lambda: viewer(browser, out), "manager": lambda: manager(browser, out), "pictures": lambda: pictures(browser, out, args.standalone)}
        for name, step in steps.items():
            if args.only in (None, name):
                step()
        browser.close()
    print(f"\n{len(failed)} failed" + "".join(f"\n  failed: {name}" for name in failed))
    sys.exit(1 if failed else 0)


if __name__ == "__main__":
    main()
