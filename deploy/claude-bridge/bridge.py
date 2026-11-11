#!/usr/bin/env python3
"""
A thin HTTP front for the Claude Code CLI, running on the host.

The CLI needs Node, and the backend image has none. Rather than fattening a
938MB image with a Node runtime, this follows the pattern the gateway already
uses on this box: a small host service the containers reach at the Docker bridge
address, exactly as they reach 9router on 172.18.0.1:20128.

What it is for: the parts of the product that want a real agent session rather
than a single completion — a question asked in the Telegram bot, an operator
looked up on the web, a code change prepared for review. Sessions are resumable,
so a conversation keeps its context between messages.

Safety, in order of importance:

  * **Read-only by default.** Tools are restricted to reading and searching.
    Writing files or running commands requires `allow_writes`, which the caller
    only sets for a flow that ends in a human pressing "apply".
  * **Not reachable from outside.** Binds to loopback and the Docker bridge, and
    every request must carry the shared secret.
  * **Bounded.** One concurrent session by default (this host has ~3GB of RAM),
    a hard timeout, and a capped prompt size.

It deliberately holds no product knowledge. It runs a CLI and returns what the
CLI said.
"""
from __future__ import annotations

import json
import logging
import os
import shutil
import subprocess
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

logging.basicConfig(
    level=logging.INFO,
    format="%(asctime)s %(levelname)s claude-bridge: %(message)s",
)
logger = logging.getLogger("claude-bridge")

TOKEN = os.environ.get("CLAUDE_BRIDGE_TOKEN", "").strip()
PORT = int(os.environ.get("CLAUDE_BRIDGE_PORT", "20130"))
BINDS = [b.strip() for b in os.environ.get("CLAUDE_BRIDGE_BIND", "127.0.0.1,172.18.0.1").split(",") if b.strip()]
DEFAULT_MODEL = os.environ.get("CLAUDE_BRIDGE_MODEL", "cc/claude-opus-5")
DEFAULT_CWD = os.environ.get("CLAUDE_BRIDGE_CWD", "/opt/receipt-parser")
SANDBOX_CWD = os.environ.get("CLAUDE_BRIDGE_SANDBOX", "/var/lib/claude-bridge/scratch")
MAX_CONCURRENCY = int(os.environ.get("CLAUDE_BRIDGE_CONCURRENCY", "1"))
DEFAULT_TIMEOUT = int(os.environ.get("CLAUDE_BRIDGE_TIMEOUT", "300"))
MAX_PROMPT_CHARS = int(os.environ.get("CLAUDE_BRIDGE_MAX_PROMPT", "60000"))

# Reading and searching only. Anything that mutates the box is added per-request
# and only for a flow that ends with a human approving a diff.
#
# The mcp__parcer-tools__* pair is how a session reaches the product's own vetted
# tools — analytics, audit, annotation, export. They are safe to allow by default
# precisely because each one keeps its own authorization and confirmation gate:
# a session can look at anything an admin could, and still cannot apply a change
# the in-app flow would have asked about.
MCP_TOOLS = "mcp__parcer-tools__list_tools,mcp__parcer-tools__run_tool"
READ_ONLY_TOOLS = "Read,Grep,Glob,WebSearch,WebFetch," + MCP_TOOLS
WRITE_TOOLS = READ_ONLY_TOOLS + ",Edit,Write,Bash"

MCP_SERVER_PATH = os.environ.get("CLAUDE_BRIDGE_MCP_SERVER", "/opt/claude-bridge/mcp_server.py")
BACKEND_URL = os.environ.get("MCP_BACKEND_URL", "http://127.0.0.1:8000")
INTERNAL_API_KEY = os.environ.get("INTERNAL_API_KEY", "")


def _mcp_config(telegram_id: str = "") -> str:
    """The stdio server that fronts the product's tools, as CLI config."""
    return json.dumps({
        "mcpServers": {
            "parcer-tools": {
                "command": "python3",
                "args": [MCP_SERVER_PATH],
                "env": {
                    "MCP_BACKEND_URL": BACKEND_URL,
                    "INTERNAL_API_KEY": INTERNAL_API_KEY,
                    "MCP_ACTOR_TELEGRAM_ID": str(telegram_id or ""),
                },
            }
        }
    })

_slots = threading.Semaphore(MAX_CONCURRENCY)


def _run_claude(payload: dict) -> dict:
    prompt = str(payload.get("prompt") or "").strip()
    if not prompt:
        return {"ok": False, "error": "prompt_required"}
    if len(prompt) > MAX_PROMPT_CHARS:
        return {"ok": False, "error": f"prompt_too_long:{len(prompt)}"}

    binary = shutil.which("claude")
    if not binary:
        return {"ok": False, "error": "claude_cli_not_installed"}

    allow_writes = bool(payload.get("allow_writes"))
    args = [
        binary,
        "--print",
        "--output-format", "json",
        "--model", str(payload.get("model") or DEFAULT_MODEL),
        "--allowed-tools", WRITE_TOOLS if allow_writes else READ_ONLY_TOOLS,
    ]
    if os.path.exists(MCP_SERVER_PATH) and INTERNAL_API_KEY:
        args += ["--mcp-config", _mcp_config(str(payload.get("telegram_id") or ""))]
    session_id = str(payload.get("session_id") or "").strip()
    if session_id:
        args += ["--resume", session_id]
    if payload.get("system_prompt"):
        args += ["--append-system-prompt", str(payload["system_prompt"])[:8000]]

    # A read-only session has no reason to sit in the source tree.
    cwd = DEFAULT_CWD if allow_writes else SANDBOX_CWD
    os.makedirs(SANDBOX_CWD, exist_ok=True)

    timeout = max(10, min(int(payload.get("timeout") or DEFAULT_TIMEOUT), 900))

    env = dict(os.environ)
    env.setdefault("ANTHROPIC_BASE_URL", os.environ.get("ANTHROPIC_BASE_URL", ""))
    env.setdefault("ANTHROPIC_AUTH_TOKEN", os.environ.get("ANTHROPIC_AUTH_TOKEN", ""))

    if not _slots.acquire(timeout=30):
        return {"ok": False, "error": "busy", "retryable": True}
    try:
        completed = subprocess.run(
            args,
            input=prompt,
            capture_output=True,
            text=True,
            timeout=timeout,
            cwd=cwd,
            env=env,
        )
    except subprocess.TimeoutExpired:
        logger.warning("claude timed out after %ss", timeout)
        return {"ok": False, "error": "timeout", "retryable": True}
    except Exception as exc:  # noqa: BLE001
        logger.warning("claude failed: %s", exc)
        return {"ok": False, "error": str(exc)[:300], "retryable": True}
    finally:
        _slots.release()

    if completed.returncode != 0:
        return {
            "ok": False,
            "error": (completed.stderr or "claude_failed")[:600],
            "retryable": True,
        }

    try:
        data = json.loads(completed.stdout or "{}")
    except json.JSONDecodeError:
        # A non-JSON body still carries the answer; hand it back rather than
        # losing the work to a parsing detail.
        return {"ok": True, "text": (completed.stdout or "").strip()[:20000], "session_id": None}

    return {
        "ok": True,
        "text": str(data.get("result") or "").strip(),
        "session_id": data.get("session_id"),
        "cost_usd": data.get("total_cost_usd"),
        "duration_ms": data.get("duration_ms") or data.get("duration_api_ms"),
        "is_error": bool(data.get("is_error")),
    }


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, fmt, *args):  # noqa: A003 - quieten the default logger
        logger.info("%s - %s", self.address_string(), fmt % args)

    def _reply(self, status: int, body: dict) -> None:
        raw = json.dumps(body, ensure_ascii=False).encode("utf-8")
        self.send_response(status)
        self.send_header("Content-Type", "application/json; charset=utf-8")
        self.send_header("Content-Length", str(len(raw)))
        self.end_headers()
        self.wfile.write(raw)

    def do_GET(self):  # noqa: N802
        if self.path == "/health":
            self._reply(200, {"ok": True, "cli": bool(shutil.which("claude"))})
            return
        self._reply(404, {"ok": False, "error": "not_found"})

    def do_POST(self):  # noqa: N802
        if self.path != "/run":
            self._reply(404, {"ok": False, "error": "not_found"})
            return
        if TOKEN and self.headers.get("X-Bridge-Token", "") != TOKEN:
            self._reply(403, {"ok": False, "error": "forbidden"})
            return

        try:
            length = int(self.headers.get("Content-Length") or 0)
            payload = json.loads(self.rfile.read(length) or b"{}")
        except Exception:  # noqa: BLE001
            self._reply(400, {"ok": False, "error": "bad_request"})
            return

        result = _run_claude(payload if isinstance(payload, dict) else {})
        self._reply(200 if result.get("ok") else 502, result)


def main() -> None:
    if not TOKEN:
        logger.warning("CLAUDE_BRIDGE_TOKEN is unset — refusing to start")
        raise SystemExit(2)

    servers = []
    for bind in BINDS:
        try:
            server = ThreadingHTTPServer((bind, PORT), Handler)
        except OSError as exc:
            logger.warning("Could not bind %s:%s (%s)", bind, PORT, exc)
            continue
        servers.append(server)
        threading.Thread(target=server.serve_forever, daemon=True).start()
        logger.info("listening on %s:%s", bind, PORT)

    if not servers:
        raise SystemExit("no listeners could be bound")

    threading.Event().wait()


if __name__ == "__main__":
    main()
