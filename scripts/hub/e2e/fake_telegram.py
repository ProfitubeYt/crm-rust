#!/usr/bin/env python3
"""A stand-in for the Telegram Bot API, for the Time bot in the disposable
stack: it hands out the updates a check posts to it and records what the bot
sends back. It is not Telegram and talks to nobody but the bot.

    POST /bot<token>/getMe | getUpdates | sendMessage | setMessageReaction
    POST /_fake/update   {"chat_id", "from_id", "text", ...} -> the ids given
    GET  /_fake/calls    what the bot sent, oldest first
    GET  /_fake/state    how far the bot has read
"""

import json
import os
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

TOKEN = os.environ["FAKE_TG_TOKEN"]
PORT = int(os.environ.get("FAKE_TG_PORT", "8081"))
# Telegram holds an empty poll for as long as the bot asks; a check cannot wait that long.
LONGEST_POLL = 2.0

lock = threading.Condition()
updates = []
calls = []
counters = {"update": 0, "message": 100, "read_up_to": 0, "polls": 0}


def new_update(spec):
    with lock:
        counters["update"] += 1
        counters["message"] += 1
        chat_id = int(spec["chat_id"])
        message = {
            "message_id": int(spec.get("message_id") or counters["message"]),
            "date": int(spec.get("date") or time.time()),
            "chat": {
                "id": chat_id,
                "type": spec.get("chat_type") or ("private" if chat_id > 0 else "supergroup"),
                "title": spec.get("chat_title", "e2e"),
            },
            "from": {
                "id": int(spec["from_id"]),
                "is_bot": False,
                "first_name": spec.get("first_name", "Synthetic"),
                "last_name": spec.get("last_name"),
                "username": spec.get("username"),
            },
            "text": spec["text"],
        }
        kind = "edited_message" if spec.get("edited") else "message"
        update = {"update_id": counters["update"], kind: message}
        updates.append(update)
        lock.notify_all()
        return {"update_id": update["update_id"], "message_id": message["message_id"]}


def get_updates(body):
    offset = int(body.get("offset") or 0)
    wait = min(float(body.get("timeout") or 0), LONGEST_POLL)
    deadline = time.monotonic() + wait
    with lock:
        counters["polls"] += 1
        # An offset confirms everything before it, as with Telegram.
        counters["read_up_to"] = max(counters["read_up_to"], offset - 1)
        updates[:] = [item for item in updates if item["update_id"] >= offset]
        while not updates:
            left = deadline - time.monotonic()
            if left <= 0:
                break
            lock.wait(left)
        return list(updates)


class Handler(BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def answer(self, status, payload):
        data = json.dumps(payload, ensure_ascii=False).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def body(self):
        size = int(self.headers.get("Content-Length") or 0)
        return json.loads(self.rfile.read(size) or b"{}") if size else {}

    def do_GET(self):
        if self.path == "/_fake/calls":
            with lock:
                return self.answer(200, list(calls))
        if self.path == "/_fake/state":
            with lock:
                return self.answer(200, {**counters, "waiting": len(updates)})
        self.answer(404, {"ok": False, "description": "Not Found"})

    def do_POST(self):
        if self.path == "/_fake/update":
            return self.answer(200, new_update(self.body()))
        prefix = f"/bot{TOKEN}/"
        if not self.path.startswith(prefix):
            return self.answer(401, {"ok": False, "error_code": 401, "description": "Unauthorized"})
        method, body = self.path[len(prefix):], self.body()
        if method == "getMe":
            result = {"id": 1000000001, "is_bot": True, "first_name": "Time e2e", "username": "time_e2e_bot"}
        elif method == "getUpdates":
            result = get_updates(body)
        elif method in ("sendMessage", "setMessageReaction"):
            with lock:
                calls.append({"method": method, **body})
            result = True if method == "setMessageReaction" else {"message_id": 1, "date": int(time.time())}
        else:
            return self.answer(404, {"ok": False, "error_code": 404, "description": "Not Found"})
        self.answer(200, {"ok": True, "result": result})


if __name__ == "__main__":
    ThreadingHTTPServer(("0.0.0.0", PORT), Handler).serve_forever()
