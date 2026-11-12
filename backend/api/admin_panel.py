"""The account administration panel: three static files served under /admin/.

The page is a client of the admin API like any other. It carries no data and
no secret: everything it shows comes from /api/admin/* with the signed-in
administrator's token, and every one of those calls is authorised on the
server. So the files themselves are public, and the policy below is what keeps
the page from being turned against the administrator who opened it.
"""
from __future__ import annotations

from pathlib import Path

from fastapi import APIRouter, HTTPException
from fastapi.responses import FileResponse, RedirectResponse

PANEL_DIR = Path(__file__).resolve().parents[1] / "admin_panel"

# No inline script or style, nothing loaded from anywhere else, no framing, no
# form posts: a stolen page has nowhere to send what the administrator types.
CONTENT_SECURITY_POLICY = (
    "default-src 'none'; script-src 'self'; style-src 'self'; img-src 'self' data:; "
    "connect-src 'self'; form-action 'none'; base-uri 'none'; frame-ancestors 'none'"
)

_FILES = {
    "index.html": "text/html",
    "admin.js": "text/javascript",
    "admin.css": "text/css",
}

router = APIRouter(include_in_schema=False)


def _serve(name: str) -> FileResponse:
    media_type = _FILES.get(name)
    path = PANEL_DIR / name
    if media_type is None or not path.is_file():
        raise HTTPException(status_code=404, detail="not_found")
    return FileResponse(
        path,
        media_type=media_type,
        headers={
            "Content-Security-Policy": CONTENT_SECURITY_POLICY,
            # Always revalidated: an administrator never works in a stale panel.
            "Cache-Control": "no-cache",
            "Referrer-Policy": "no-referrer",
            "X-Robots-Tag": "noindex, nofollow",
        },
    )


@router.get("/admin")
async def panel_redirect() -> RedirectResponse:
    return RedirectResponse(url="/admin/", status_code=308)


@router.get("/admin/")
async def panel_index() -> FileResponse:
    return _serve("index.html")


@router.get("/admin/{name}")
async def panel_file(name: str) -> FileResponse:
    # Only the names in the table: the path parameter never reaches the disk as is.
    return _serve(name)
