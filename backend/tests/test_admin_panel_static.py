"""The CRM administration panel as files: what is served, how, and what is not.

The page is public by design and holds nothing: it is a client of
/api/admin/* like any other, and those calls are authorised on the server.
"""
from __future__ import annotations

import re

import pytest

from api import admin_panel
from hub_support import app_client, bearer, error_code, seed


@pytest.fixture
def client(db_session, monkeypatch, tmp_path):
    with app_client(db_session, monkeypatch, tmp_path) as test_client:
        yield test_client


def test_the_panel_is_three_files_with_a_policy_that_lets_nothing_else_in(client):
    redirect = client.get("/admin", follow_redirects=False)
    assert redirect.status_code == 308 and redirect.headers["location"] == "/admin/"

    for path, media in (("/admin/", "text/html"), ("/admin/index.html", "text/html"),
                        ("/admin/admin.js", "text/javascript"), ("/admin/admin.css", "text/css")):
        response = client.get(path)
        assert response.status_code == 200, path
        assert response.headers["content-type"] == f"{media}; charset=utf-8", path
        policy = response.headers["content-security-policy"]
        # Nothing inline, nothing from elsewhere, no framing, no form posts.
        for part in ("default-src 'none'", "script-src 'self'", "style-src 'self'", "connect-src 'self'",
                     "form-action 'none'", "base-uri 'none'", "frame-ancestors 'none'"):
            assert part in policy, (path, part)
        assert "unsafe" not in policy and "*" not in policy
        assert response.headers["cache-control"] == "no-cache"
        assert response.headers["referrer-policy"] == "no-referrer"
        assert response.headers["x-frame-options"] == "DENY"
        assert response.headers["x-content-type-options"] == "nosniff"


def test_only_the_named_files_are_served(client):
    for name in ("admin.py", "../api/main.py", "..%2Fapi%2Fmain.py", "%2e%2e/requirements.txt", ".env",
                 "index.html.bak", "admin.js.map", "ADMIN.JS", "sub/admin.js"):
        response = client.get(f"/admin/{name}")
        assert response.status_code == 404, name
    assert set(admin_panel._FILES) == {"index.html", "admin.js", "admin.css"}
    assert all((admin_panel.PANEL_DIR / name).is_file() for name in admin_panel._FILES)


def test_the_page_obeys_its_own_policy_and_carries_no_data():
    html = (admin_panel.PANEL_DIR / "index.html").read_text(encoding="utf-8")
    script = (admin_panel.PANEL_DIR / "admin.js").read_text(encoding="utf-8")
    style = (admin_panel.PANEL_DIR / "admin.css").read_text(encoding="utf-8")

    # No inline script or style, no handler attributes, nothing loaded from another origin.
    assert re.findall(r"<script(?![^>]*\bsrc=)", html) == []
    assert "<style" not in html and " style=" not in html
    assert re.findall(r"\son[a-z]+\s*=", html) == []
    assert re.findall(r"""(?:src|href)=["'](?!/admin/)""", html) == []
    assert "http://" not in html + style and "https://" not in html + style
    assert re.findall(r"https?://", script) == []
    assert "@import" not in style and "url(" not in style

    # The script builds the page from text nodes and never evaluates or injects markup.
    for forbidden in ("innerHTML", "outerHTML", "insertAdjacentHTML", "document.write", "eval(", "new Function",
                      "localStorage"):
        assert forbidden not in script, forbidden
    # The one thing kept in the tab is the launch-gate token; the account token is not stored anywhere.
    assert re.findall(r"sessionStorage\.setItem\(([^,]+),", script) == ["LAUNCH_KEY"]

    # Russian, and for the whole CRM rather than for one workspace.
    assert '<html lang="ru">' in html and "Администрирование CRM" in html
    for text in (html, script, style):
        assert "Bearer ey" not in text and "password_hash" not in text


def test_the_files_are_public_and_the_data_behind_them_is_not(client, db_session):
    # No token and no system-access header: the files still load, as a browser loads them.
    bare = {"X-System-Access": ""}
    assert client.get("/admin/", headers=bare).status_code == 200
    assert client.get("/admin/admin.js", headers=bare).status_code == 200

    # What the page then asks for is refused to anyone who is not a CRM administrator.
    anonymous = client.get("/api/admin/meta")
    assert anonymous.status_code == 401
    operator = seed(db_session, "op", time_role="manager")
    refused = client.get("/api/admin/meta", headers=bearer(operator))
    assert refused.status_code == 403 and error_code(refused) == "global_admin_required"
    admin = seed(db_session, "root", role="operator", checks=False, global_admin=True)
    assert client.get("/api/admin/meta", headers=bearer(admin)).status_code == 200
