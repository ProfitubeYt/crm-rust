#!/usr/bin/env python3
"""A stand-in for the backend, for looking at the client's windows where the
real server is out of reach: the sign-in, the session, enough of the table to
draw it and a made-up Telegram for the Telegram section. It is not the
backend: it keeps nothing, checks nothing a real server would and answers only
what those windows ask. Never point a shipped client at it.

    python3 desktop/tools/stub-server.py [--port 8000] [--rows 80] [--refresh-delay 0] [--telegram ready]
    TBS_API_BASE_URL=http://127.0.0.1:8000 tbsparcer

Sign in as `anna` with the password `anna-secret`. Sessions live in memory:
a restart of the stub ends them all, unless `--sessions <file>` keeps them.

    POST /_stub/expire   the access tokens stop working, the sessions stay
    POST /_stub/revoke   every session ends, as a sign-out on the server would

`--telegram` is where Telegram's own sign-in stands when the stub starts:
`ready`, `wait_phone_number`, `wait_code`, `wait_password` or
`tdlib_unavailable`. A number, a code and a password typed in the client move
it on; `POST /_stub/telegram/<state>` puts it anywhere. Every chat, message
and file is invented. The chat «Архив 2025» is protected by the password
`chat-secret`; a message with «не чек» in it fails in the receipt worker.
"""

import argparse
import json
import re
import secrets
import struct
import threading
import time
import zlib
from datetime import datetime, timedelta, timezone
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

CASH, BOT, REPORTS, PERSON, ARCHIVE = -1001000000001, 7000000001, -1001000000002, 7000000002, -1001000000003
CHAT_PASSWORD = "chat-secret"
MINI_APP = "https://miniapp.example/shop"
TELEGRAM_STATES = ["ready", "wait_phone_number", "wait_code", "wait_password", "tdlib_unavailable"]


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


def picture(width=640, height=400):
    """A PNG drawn here: a pale sheet with darker lines, like a receipt from afar."""
    rows = bytearray()
    for y in range(height):
        rows.append(0)
        for x in range(width):
            sheet = 40 < x < width - 40 and 24 < y < height - 24
            line = sheet and y % 34 in (0, 1, 2) and 80 < x < width - 80 - (y * 7) % 160
            shade = 70 if line else 236 if sheet else 150 + (x + y) % 40
            rows += bytes((shade, shade, min(255, shade + 12)))
    chunk = lambda kind, data: struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))
    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0)) + chunk(b"IDAT", zlib.compress(bytes(rows), 6)) + chunk(b"IEND", b"")


PDF = b"%PDF-1.4\n1 0 obj<</Type/Catalog/Pages 2 0 R>>endobj\n2 0 obj<</Type/Pages/Kids[3 0 R]/Count 1>>endobj\n3 0 obj<</Type/Page/Parent 2 0 R/MediaBox[0 0 300 200]>>endobj\ntrailer<</Root 1 0 R>>\n%%EOF\n"


class Telegram:
    """Invented chats. Messages are kept newest first, as the server sends them."""

    def __init__(self, state):
        self.state = state
        self.next_id = 100
        self.hidden = {ARCHIVE}
        self.monitored = {CASH: True, REPORTS: False}
        self.passes = set()
        self.protected = {ARCHIVE}
        self.receipts = {}
        self.picture = picture()
        now = datetime.now(timezone.utc).replace(microsecond=0)
        ago = lambda **back: (now - timedelta(**back)).strftime("%Y-%m-%dT%H:%M:%SZ")
        photo = {"file_id": 9001, "file_name": "photo.jpg", "mime_type": "image/png", "size": len(self.picture), "download_url": None, "local_path": None, "width": 640, "height": 400}
        document = {"file_id": 9002, "file_name": "Чек № 418.pdf", "mime_type": "application/pdf", "size": 48213, "download_url": None, "local_path": None}
        keys = {
            "kind": "inline",
            "rows": [
                [{"text": "Открыть магазин", "type": "web_app", "url": MINI_APP}, {"text": "Канал", "type": "url", "url": "https://t.me/example_channel"}],
                [{"text": "Код 4410", "type": "copy_text", "url": None}, {"text": "Обновить", "type": "callback", "url": None}],
            ],
        }
        say = lambda mid, at, text, out=False, **more: {"id": mid, "date": at, "is_outgoing": out, "sender_id": 41, "text": text, "document": None, "photo": None, "reply_markup": None, **more}
        self.chats = [
            {"chat_id": CASH, "title": "Касса · чеки", "username": None, "chat_type": "supergroup", "member_count": 12},
            {"chat_id": BOT, "title": "Платёжный бот", "username": "example_pay_bot", "chat_type": "bot", "member_count": None},
            {"chat_id": REPORTS, "title": "Отчёты филиала", "username": "example_reports", "chat_type": "channel", "member_count": 348},
            {"chat_id": PERSON, "title": "Борис Латипов", "username": None, "chat_type": "user", "member_count": None},
            {"chat_id": ARCHIVE, "title": "Архив 2025", "username": None, "chat_type": "group", "member_count": 4},
        ]
        self.messages = {
            CASH: [
                say(16, ago(minutes=4), "Это не чек, просто сообщение для проверки ошибки."),
                say(15, ago(minutes=9), "Принято, спасибо! Проведу до конца дня.", out=True),
                say(14, ago(minutes=21), "Чек № 418.pdf", document=document),
                say(13, ago(minutes=38), "[Photo]", photo=photo, processed=True, processed_status="done", transaction_id=80),
                say(12, ago(hours=3), "Оплата 1 250 000.00 UZS\nKorzinka.uz Супермаркет Юнусабад\nКарта *4821, остаток 17 200 000.00 UZS", processed=True, processed_status="done", transaction_id=79),
                say(11, ago(days=1, hours=2), "Списание 87 500.00 UZS, Яндекс Go Такси Ташкент. Чек во вложении не читается.", processed=False, processed_status="failed", processing_error="Парсер не смог распознать чек"),
                say(10, ago(days=1, hours=5), "Коллеги, чеки за вчера присылайте до 12:00, пожалуйста.", out=True),
                say(9, ago(days=3), "Пополнение 5 000 000.00 UZS, касса филиала № 12."),
            ],
            BOT: [
                say(24, ago(minutes=12), "Выберите действие:", reply_markup=keys, sender_id=BOT),
                say(23, ago(minutes=13), "/start", out=True),
                say(22, ago(days=2), "Платёж № 1042 на 310 000.00 UZS проведён.", sender_id=BOT),
            ],
            REPORTS: [say(31, ago(hours=6), "Сводка за неделю: 214 операций, 3 без статьи."), say(30, ago(days=7), "Сводка за неделю: 198 операций.")],
            PERSON: [say(41, ago(days=2, hours=1), "Добрый день! Отправил чек по аренде склада."), say(40, ago(days=2, hours=2), "Здравствуйте, Борис.", out=True)],
            ARCHIVE: [say(51, ago(days=320), "Закрытие года: все чеки переданы в архив.")],
        }

    def status(self):
        ready = self.state == "ready"
        user = {"id": 41, "first_name": "Анна", "last_name": "Каримова", "username": "example_anna", "phone_number": None} if ready else None
        return {"state": self.state, "raw_state": self.state, "is_authorized": ready, "phone_number": None, "user": user, "code_type": None, "code_timeout": None}

    def chat_list(self):
        items = []
        for chat in self.chats:
            cid = chat["chat_id"]
            last = self.messages[cid][0] if self.messages[cid] else None
            # What a protected chat said last is not shown in the list either.
            shown = None if cid in self.protected else last
            items.append(
                {
                    **chat,
                    "is_hidden": cid in self.hidden,
                    "is_protected": cid in self.protected,
                    "is_monitored": cid in self.monitored,
                    "monitor_enabled": self.monitored.get(cid, False),
                    "last_message": shown,
                    "latest_message_id": last["id"] if last else None,
                    "latest_message_date": last["date"] if last else None,
                    "backlog_lag_messages": 7 if cid == ARCHIVE else 0,
                    "backlog_state": "lagging" if cid == ARCHIVE else "healthy",
                }
            )
        return {"total": len(items), "items": items}

    def add(self, cid, text, **more):
        self.next_id += 1
        at = datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
        message = {"id": self.next_id, "date": at, "is_outgoing": True, "sender_id": 41, "text": text, "document": None, "photo": None, "reply_markup": None, **more}
        self.messages[cid].insert(0, message)
        return message

    def find(self, cid, mid):
        return next((m for m in self.messages.get(cid, []) if m["id"] == mid), None)

    def receipt(self, cid, mid):
        """Queued on the first look, settled on the second, as a quick worker would."""
        key = (cid, mid)
        message = self.find(cid, mid)
        seen = self.receipts.get(key, 0)
        self.receipts[key] = seen + 1
        if seen == 0:
            return "queued"
        failed = message is None or "не чек" in (message.get("text") or "")
        if message:
            message.update(processed=not failed, processed_status="failed" if failed else "done", transaction_id=None if failed else 81, processing_error="Парсер не смог распознать чек" if failed else None)
        return "failed" if failed else "done"


class State:
    def __init__(self, rows, sessions_file, refresh_delay=0.0, telegram="ready"):
        self.lock = threading.Lock()
        self.rows = rows
        self.refresh_delay = refresh_delay
        self.access = set()
        self.file = Path(sessions_file) if sessions_file else None
        self.refresh = set(json.loads(self.file.read_text())) if self.file and self.file.exists() else set()
        self.telegram = Telegram(telegram)

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

    def send(self, status, kind, data, name=None):
        self.send_response(status)
        self.send_header("content-type", kind)
        self.send_header("content-length", str(len(data)))
        if name:
            self.send_header("content-disposition", f'attachment; filename="{name}"')
        self.end_headers()
        self.wfile.write(data)
        print(f"{self.command:6} {status} {self.path}", flush=True)

    def answer(self, status, body):
        self.send(status, "application/json; charset=utf-8", json.dumps(body, ensure_ascii=False).encode())

    def raw(self):
        return self.rfile.read(int(self.headers.get("content-length") or 0))

    def body(self):
        try:
            return json.loads(self.raw() or b"{}")
        except ValueError:
            return {}

    def signed_in(self):
        bearer = self.headers.get("authorization", "")
        return bearer.startswith("Bearer ") and bearer[7:] in self.state.access

    def telegram(self, method, path, query, raw):
        """`/api/tg/...`, shaped as backend/api/routes/telegram_client.py answers it."""
        tg = self.state.telegram
        try:
            body = json.loads(raw or b"{}") if not self.headers.get("content-type", "").startswith("multipart/") else {}
        except ValueError:
            body = {}
        if path == "/api/tg/status":
            return self.answer(200, tg.status())
        step = {"/api/tg/auth/phone": ("wait_phone_number", "phone_number", "wait_code"), "/api/tg/auth/code": ("wait_code", "code", "wait_password"), "/api/tg/auth/password": ("wait_password", "password", "ready")}.get(path)
        if step and method == "POST":
            expected, field, then = step
            if tg.state != expected or not str(body.get(field) or "").strip():
                return self.answer(400, {"detail": f"Failed to verify {field}: {field.upper()}_INVALID"})
            tg.state = then
            return self.answer(200, tg.status())
        if tg.state != "ready":
            return self.answer(401, {"detail": "TDLib client is not authorized"})
        if path == "/api/tg/chats":
            return self.answer(200, tg.chat_list())
        monitor = re.fullmatch(r"/api/tg/monitors/(-?\d+)", path)
        if monitor and method == "PUT":
            cid = int(monitor[1])
            tg.monitored[cid] = bool(body.get("enabled"))
            return self.answer(200, {"chat_id": cid, "enabled": tg.monitored[cid], "last_processed_message_id": 0, "last_error": None})
        found = re.fullmatch(r"/api/tg/chats/(-?\d+)(/.*)", path)
        if not found or int(found[1]) not in tg.messages:
            return self.answer(404, {"detail": "Not found"})
        cid, rest = int(found[1]), found[2]
        if rest == "/hidden" and method == "PUT":
            (tg.hidden.add if body.get("hidden") else tg.hidden.discard)(cid)
            return self.answer(200, {"chat_id": cid, "hidden": cid in tg.hidden, "updated_at": datetime.now(timezone.utc).isoformat()})
        if rest == "/password/verify":
            if cid in tg.protected and body.get("password") != CHAT_PASSWORD:
                return self.answer(200, {"verified": False, "attempts_left": 4, "locked": False, "locked_until": None})
            token = "chat-" + secrets.token_hex(8)
            tg.passes.add(token)
            return self.answer(200, {"verified": True, "session_token": token, "expires_in": 3600})
        if rest == "/password":
            if method == "DELETE" and cid in tg.protected and body.get("password") != CHAT_PASSWORD:
                return self.answer(401, {"detail": {"verified": False, "error": "invalid_password"}})
            (tg.protected.discard if method == "DELETE" else tg.protected.add)(cid)
            return self.answer(200, {"chat_id": cid, "protected": cid in tg.protected})
        if cid in tg.protected and self.headers.get("x-chat-access") not in tg.passes:
            return self.answer(403, {"detail": {"error": "chat_locked", "chat_id": cid}})
        if rest == "/messages" and method == "GET":
            limit = int(query.get("limit", ["50"])[0])
            return self.answer(200, {"total": len(tg.messages[cid]), "items": tg.messages[cid][:limit]})
        if rest == "/messages" and method == "POST":
            if not str(body.get("text") or "").strip():
                return self.answer(400, {"detail": "Text is required"})
            return self.answer(200, tg.add(cid, body["text"]))
        if rest == "/documents" and method == "POST":
            name = re.search(rb'filename="([^"]*)"', raw)
            kind = re.search(rb"content-type: *([^\r\n]+)", raw, re.I)
            if not kind or kind[1].strip().lower() not in (b"application/pdf", b"application/x-pdf", b"application/octet-stream"):
                return self.answer(400, {"detail": "Only PDF files are supported"})
            title = name[1].decode("utf-8", "replace") if name else "document.pdf"
            file = {"file_id": 9002, "file_name": title, "mime_type": "application/pdf", "size": len(raw), "download_url": None, "local_path": None}
            return self.answer(200, tg.add(cid, title, document=file))
        if rest == "/receipt-status":
            ids = [int(part) for part in query.get("message_ids", [""])[0].split(",") if part.strip().lstrip("-").isdigit()]
            return self.answer(200, {"results": [{"chat_id": cid, "message_id": mid, "status": tg.receipt(cid, mid)} for mid in ids]})
        if rest == "/menu-button":
            bot = cid == BOT
            return self.answer(200, {"available": bot, "text": "Магазин" if bot else None, "url": MINI_APP if bot else None, "is_main": False, "bot_user_id": BOT if bot else None})
        if rest == "/webapp/open":
            return self.answer(200, {"url": body.get("url") or MINI_APP, "launch_id": "1" if body.get("button_kind") != "keyboard" else None})
        if rest in ("/webapp/close", "/webapp/data"):
            return self.answer(200, {"ok": True})
        process = re.fullmatch(r"/messages/(\d+)/process", rest)
        if process and method == "POST":
            mid = int(process[1])
            tg.receipts.pop((cid, mid), None)
            return self.answer(200, {"chat_id": cid, "message_id": mid, "task_id": f"stub-{mid}", "status": tg.receipt(cid, mid), "transaction_id": None, "error": None})
        file = re.fullmatch(r"/messages/(\d+)/files/(\d+)", rest)
        if file and method == "GET":
            message = tg.find(cid, int(file[1]))
            held = [part["file_id"] for part in ((message or {}).get("photo"), (message or {}).get("document")) if part]
            if int(file[2]) not in held:
                return self.answer(404, {"detail": "File not found in message"})
            if message.get("photo"):
                return self.send(200, "image/png", tg.picture, "photo.png")
            return self.send(200, "application/pdf", PDF, "receipt.pdf")
        self.answer(404, {"detail": "Not found"})

    def do_PUT(self):
        self.do_POST()

    def do_DELETE(self):
        self.do_POST()

    def do_POST(self):
        url = urlparse(self.path)
        path, state = url.path, self.state
        if path.startswith("/api/tg/"):
            raw = self.raw()
            if not self.signed_in():
                return self.answer(401, {"detail": "Not authenticated"})
            with state.lock:
                return self.telegram(self.command, path, parse_qs(url.query), raw)
        body = self.body()
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
        if path == "/api/transactions/process-receipt" and self.signed_in():
            return self.answer(200, {"created": True, "duplicate": False, "transaction": state.rows[0], "parsing": {"method": "regex", "confidence": 0.9, "notes": None}})
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
        if path.startswith("/_stub/telegram/") and path.rsplit("/", 1)[1] in TELEGRAM_STATES:
            state.telegram.state = path.rsplit("/", 1)[1]
            return self.answer(200, {"ok": True, "state": state.telegram.state})
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
        if path.startswith("/api/tg/"):
            with self.state.lock:
                return self.telegram("GET", path, query, b"")
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
    parser.add_argument("--telegram", choices=TELEGRAM_STATES, default="ready", help="where Telegram's own sign-in stands at the start")
    args = parser.parse_args()
    Handler.state = State(transactions(args.rows), args.sessions, args.refresh_delay, args.telegram)
    print(f"stub backend on http://127.0.0.1:{args.port}, {args.rows} rows; sign in as {USER['username']} / {PASSWORD}", flush=True)
    ThreadingHTTPServer(("127.0.0.1", args.port), Handler).serve_forever()


if __name__ == "__main__":
    main()
