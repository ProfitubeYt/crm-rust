#!/usr/bin/env python3
"""A stand-in for the backend, for looking at the client's windows where the
real server is out of reach: the sign-in, the session and enough of the table
to draw it. It is not the backend: it keeps nothing, checks nothing a real
server would and answers only what the start window and the table ask at
first. Never point a shipped client at it.

    python3 desktop/tools/stub-server.py [--port 8000] [--rows 80] [--refresh-delay 0]
    TBS_API_BASE_URL=http://127.0.0.1:8000 tbsparcer

Sign in as `anna` with the password `anna-secret`. Sessions live in memory:
a restart of the stub ends them all, unless `--sessions <file>` keeps them.

    POST /_stub/expire   the access tokens stop working, the sessions stay
    POST /_stub/revoke   every session ends, as a sign-out on the server would
"""

import argparse
import json
import secrets
import threading
import time
from datetime import datetime, timedelta
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import parse_qs, urlparse

USER = {
    "id": 1,
    "user_id": 1,
    "username": "anna",
    "display_name": "Анна Каримова",
    "role": "admin",
    "is_active": True,
    "allowed_tabs": ["dashboard", "reference", "userbot", "logs", "admin"],
    "allowed_folders": [],
    "forbidden_periods": [],
    "allowed_sources": [],
    "can_toggle_sources": True,
    "permissions_version": 1,
    "totp_enabled": False,
    "force_2fa": False,
}
PASSWORD = "anna-secret"

OPERATORS = [
    "OOO «Ромашка Маркет» филиал № 12 Чиланзар",
    "Korzinka.uz Супермаркет Юнусабад",
    "Яндекс Go Такси Ташкент",
    "Аптека «Подорожник» круглосуточная",
    "АЗС Вектор № 7 Сергели",
    "Maxway Доставка еды",
    "UZUM Market маркетплейс",
    "Beeline Uzbekistan пополнение счёта",
]
APPS = ["Payme", "Click", "Uzum Bank", "Paynet", "Humo Pay"]
TYPES = [("DEBIT", "Списание"), ("CREDIT", "Пополнение"), ("DEBIT", "Списание"), ("CONVERSION", "Конверсия"), ("DEBIT", "Списание"), ("REVERSAL", "Отмена")]
PERSONS = [("АК", "Анна Каримова"), ("БЛ", "Борис Латипов"), ("ВС", "Вера Садыкова")]
ARTICLES = [("пр", "Продукты"), ("тр", "Транспорт"), ("ар", "Аренда"), ("зп", "Зарплата")]
DETAILS = [
    "закупка для офиса, чек приложен к заявке № 418",
    "поездка в банк и обратно",
    "аренда склада за ноябрь, договор 17/А",
    "",
    "возврат по ошибочному платежу от 03.11",
    "канцтовары и бумага для бухгалтерии",
]


def transactions(count):
    start = datetime(2026, 11, 11, 18, 40)
    rows = []
    for i in range(count):
        at = start - timedelta(hours=7 * i, minutes=13 * i)
        kind, kind_text = TYPES[i % len(TYPES)]
        p2p = i % 7 == 3
        person = PERSONS[i % len(PERSONS)] if i % 3 != 2 else None
        article = ARTICLES[i % len(ARTICLES)] if i % 4 != 3 else None
        telegram = i % 2 == 0
        rows.append(
            {
                "id": count - i,
                "transaction_date": at.strftime("%Y-%m-%dT%H:%M:%S"),
                "amount": f"{(i * 37 % 91 + 1) * 12500 + i * 350}.00",
                "currency": "USD" if i % 9 == 8 else "UZS",
                "card_last_4": ["4821", "0397", "7715"][i % 3],
                "operator_raw": OPERATORS[i % len(OPERATORS)],
                "application_mapped": APPS[i % len(APPS)],
                "description": None,
                "transaction_type": kind,
                "transaction_type_display": kind_text,
                "stored_transaction_type": kind,
                "balance_after": f"{18_450_000 - i * 211_300}.00",
                "receiver_name": "Каримов Рустам Бахтиёрович" if p2p else None,
                "receiver_card": "8600 **** **** 4410" if p2p else None,
                "source_type": "AUTO",
                "source_channel": "TELEGRAM" if telegram else "SMS",
                "source_display": "Касса · чеки" if telegram else "SMS 1220",
                "source_chat_id": -1001234500 - i % 3 if telegram else None,
                "source_chat_title": "Касса · чеки" if telegram else None,
                "source_origin_type": None,
                "excel_bank": None,
                "parsing_method": "gpt" if i % 5 == 0 else "regex",
                "parsing_confidence": 0.55 + (i % 5) / 10,
                "is_p2p": p2p,
                "annotation_details": DETAILS[i % len(DETAILS)] or None,
                "annotation_person_code": person[0] if person else None,
                "annotation_period": at.strftime("%Y-%m-01") if i % 3 != 2 else None,
                "annotation_article_code": article[0] if article else None,
                "created_at": at.strftime("%Y-%m-%dT%H:%M:%S"),
                "updated_at": at.strftime("%Y-%m-%dT%H:%M:%S"),
                "raw_message": f"Оплата {OPERATORS[i % len(OPERATORS)]}",
            }
        )
    return rows


class State:
    def __init__(self, rows, sessions_file, refresh_delay=0.0):
        self.lock = threading.Lock()
        self.rows = rows
        self.refresh_delay = refresh_delay
        self.access = set()
        self.file = Path(sessions_file) if sessions_file else None
        self.refresh = set(json.loads(self.file.read_text())) if self.file and self.file.exists() else set()

    def save(self):
        if self.file:
            self.file.write_text(json.dumps(sorted(self.refresh)))

    def open_session(self, old=None):
        """A new pair of tokens; a refresh token is spent by using it."""
        with self.lock:
            self.refresh.discard(old)
            token, refresh = "access-" + secrets.token_hex(16), "refresh-" + secrets.token_hex(24)
            self.access.add(token)
            self.refresh.add(refresh)
            self.save()
        return {"ok": True, "token": token, "refresh_token": refresh, "user": USER}


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"
    state = None

    def log_message(self, *_):
        pass

    def answer(self, status, body):
        data = json.dumps(body, ensure_ascii=False).encode()
        self.send_response(status)
        self.send_header("content-type", "application/json; charset=utf-8")
        self.send_header("content-length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)
        print(f"{self.command:6} {status} {self.path}", flush=True)

    def body(self):
        length = int(self.headers.get("content-length") or 0)
        try:
            return json.loads(self.rfile.read(length) or b"{}")
        except ValueError:
            return {}

    def signed_in(self):
        bearer = self.headers.get("authorization", "")
        return bearer.startswith("Bearer ") and bearer[7:] in self.state.access

    def do_POST(self):
        path, state, body = urlparse(self.path).path, self.state, self.body()
        if path == "/api/auth/login":
            if body.get("username") == USER["username"] and body.get("password") == PASSWORD:
                return self.answer(200, state.open_session())
            return self.answer(401, {"ok": False, "error": "invalid_credentials", "attempts_left": 4})
        if path == "/api/auth/refresh":
            # Slow on request, so the start window can be seen while it still waits.
            time.sleep(state.refresh_delay)
            token = body.get("refresh_token") or self.headers.get("x-refresh-token")
            if token in state.refresh:
                return self.answer(200, state.open_session(token))
            return self.answer(401, {"ok": False, "error": "invalid_refresh_token"})
        if path == "/api/auth/logout":
            with state.lock:
                state.refresh.discard(body.get("refresh_token"))
                state.access.discard(self.headers.get("authorization", "")[7:])
                state.save()
            return self.answer(200, {"ok": True})
        if path == "/_stub/expire":
            with state.lock:
                state.access.clear()
            return self.answer(200, {"ok": True})
        if path == "/_stub/revoke":
            with state.lock:
                state.access.clear()
                state.refresh.clear()
                state.save()
            return self.answer(200, {"ok": True})
        self.answer(404, {"detail": "Not found"})

    def do_GET(self):
        url = urlparse(self.path)
        path, query, rows = url.path, parse_qs(url.query), self.state.rows
        if path == "/health":
            return self.answer(200, {"status": "healthy", "database": "stub"})
        if not path.startswith("/api/"):
            return self.answer(404, {"detail": "Not found"})
        if not self.signed_in():
            return self.answer(401, {"detail": "Not authenticated"})
        if path == "/api/auth/me":
            return self.answer(200, USER)
        if path == "/api/auth/entitlement":
            return self.answer(200, {"ok": True, "expired": False, "allowed_until": None, "username": USER["username"], "role": USER["role"]})
        if path == "/api/sync/manifest":
            table = {"row_count": len(rows), "updated_at": rows[0]["updated_at"] if rows else None, "checksum": "sha256:stub"}
            return self.answer(200, {"tables": {"transactions": table}})
        if path == "/api/transactions/":
            page, size = int(query.get("page", ["1"])[0]), int(query.get("page_size", ["1000"])[0])
            items = rows[(page - 1) * size : page * size]
            return self.answer(200, {"ok": True, "total": len(rows), "page": page, "page_size": size, "items": items, "has_more": page * size < len(rows)})
        if path == "/api/security/status":
            return self.answer(200, {"scopes_enabled": False, "available_years": [2026], "current_scope": None})
        if path == "/api/transactions/init":
            years = [{"year": 2026, "count": len(rows)}]
            return self.answer(200, {"security_status": {"scopes_enabled": False}, "years": years, "operators": OPERATORS, "applications": APPS, "telegram_sources": []})
        if path == "/api/reference":
            items = [{"id": n + 1, "operator_name": name, "application_name": APPS[n % len(APPS)], "is_p2p": False, "is_active": True} for n, name in enumerate(OPERATORS)]
            return self.answer(200, {"total": len(items), "page": 1, "page_size": len(items), "items": items})
        if path == "/api/reference/applications":
            return self.answer(200, APPS)
        if path == "/api/reference/annotations/persons":
            return self.answer(200, [{"id": n + 1, "code": code, "full_name": name, "sort_order": n, "is_active": True, "is_auto": False} for n, (code, name) in enumerate(PERSONS)])
        if path == "/api/reference/annotations/articles":
            return self.answer(200, [{"id": n + 1, "code": code, "label": label, "direction": "expense", "sort_order": n, "is_active": True} for n, (code, label) in enumerate(ARTICLES)])
        # What the table asks for and can do without.
        if path in ("/api/security/scopes", "/api/periods"):
            return self.answer(200, [])
        if path == "/api/security/locked-periods":
            return self.answer(200, {})
        self.answer(404, {"detail": "Not found"})


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--port", type=int, default=8000)
    parser.add_argument("--rows", type=int, default=80)
    parser.add_argument("--sessions", help="a file that keeps the sessions over a restart of the stub")
    parser.add_argument("--refresh-delay", type=float, default=0.0, help="seconds a saved session takes to be confirmed")
    args = parser.parse_args()
    Handler.state = State(transactions(args.rows), args.sessions, args.refresh_delay)
    print(f"stub backend on http://127.0.0.1:{args.port}, {args.rows} rows; sign in as {USER['username']} / {PASSWORD}", flush=True)
    ThreadingHTTPServer(("127.0.0.1", args.port), Handler).serve_forever()


if __name__ == "__main__":
    main()
