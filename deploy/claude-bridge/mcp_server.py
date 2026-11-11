#!/usr/bin/env python3
"""
An MCP server that hands the product's own agent tools to a Claude Code session.

The system already has 49 vetted tools, each carrying its authorization checks,
its confirmation gating and its change journal. Re-describing them to a session
as prose, or writing a second path to the same data, would mean a weaker copy of
all of that. So this exposes the registry itself: two MCP tools, `list_tools`
and `run_tool`, that proxy to the backend's internal API.

Anything the underlying tool marks as needing confirmation comes back as a
description of what *would* happen, not as a change — the same gate the in-app
flow uses. A session cannot talk its way past it, because the refusal happens on
the far side of this process.

Runs on the host next to the CLI, with no dependency on the application's Python
environment: it speaks HTTP to the backend on loopback and JSON-RPC on stdio.
"""
from __future__ import annotations

import json
import os
import sys
import urllib.error
import urllib.request

BACKEND = os.environ.get("MCP_BACKEND_URL", "http://127.0.0.1:8000").rstrip("/")
INTERNAL_KEY = os.environ.get("INTERNAL_API_KEY", "")
TELEGRAM_ID = os.environ.get("MCP_ACTOR_TELEGRAM_ID", "")
TIMEOUT = float(os.environ.get("MCP_TIMEOUT", "120"))

PROTOCOL_VERSION = "2024-11-05"


def _call_backend(path: str, payload: dict | None = None, method: str = "POST") -> dict:
    url = f"{BACKEND}{path}"
    data = json.dumps(payload or {}).encode("utf-8") if method == "POST" else None
    request = urllib.request.Request(url, data=data, method=method)
    request.add_header("X-Internal-Api-Key", INTERNAL_KEY)
    request.add_header("Content-Type", "application/json")
    try:
        with urllib.request.urlopen(request, timeout=TIMEOUT) as response:
            return json.loads(response.read().decode("utf-8") or "{}")
    except urllib.error.HTTPError as exc:
        body = exc.read().decode("utf-8", "replace")[:500]
        return {"ok": False, "error": f"http_{exc.code}", "detail": body}
    except Exception as exc:  # noqa: BLE001
        return {"ok": False, "error": str(exc)[:300]}


TOOLS = [
    {
        "name": "list_tools",
        "description": (
            "Список инструментов системы разбора чеков: аналитика транзакций, "
            "аудит и сверка, поиск дубликатов, разметка, справочники, экспорт. "
            "Вызови это первым, чтобы узнать, что доступно."
        ),
        "inputSchema": {"type": "object", "properties": {}},
    },
    {
        "name": "run_tool",
        "description": (
            "Выполнить инструмент системы. Передай name из list_tools и arguments. "
            "Если инструмент требует подтверждения, вернётся описание того, что "
            "будет сделано — изменения не применяются без отдельного согласия."
        ),
        "inputSchema": {
            "type": "object",
            "properties": {
                "name": {"type": "string", "description": "Имя инструмента из list_tools"},
                "arguments": {"type": "object", "description": "Аргументы инструмента"},
            },
            "required": ["name"],
        },
    },
]


def _handle(method: str, params: dict) -> dict | None:
    if method == "initialize":
        return {
            "protocolVersion": PROTOCOL_VERSION,
            "capabilities": {"tools": {}},
            "serverInfo": {"name": "parcer-tools", "version": "1.0.0"},
        }

    if method in ("notifications/initialized", "initialized"):
        return None  # notification: no reply

    if method == "tools/list":
        return {"tools": TOOLS}

    if method == "tools/call":
        name = params.get("name")
        arguments = params.get("arguments") or {}

        if name == "list_tools":
            result = _call_backend("/api/internal/agent/tools", method="GET")
        elif name == "run_tool":
            inner = str(arguments.get("name") or "").strip()
            if not inner:
                result = {"ok": False, "error": "name_required"}
            else:
                result = _call_backend("/api/internal/agent/tool", {
                    "name": inner,
                    "arguments": arguments.get("arguments") or {},
                    "telegram_id": TELEGRAM_ID or None,
                })
        else:
            result = {"ok": False, "error": f"unknown_tool:{name}"}

        return {
            "content": [{"type": "text", "text": json.dumps(result, ensure_ascii=False)}],
            "isError": not result.get("ok", False),
        }

    return {"error": {"code": -32601, "message": f"method not found: {method}"}}


def main() -> None:
    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        try:
            message = json.loads(line)
        except json.JSONDecodeError:
            continue

        result = _handle(message.get("method", ""), message.get("params") or {})
        # A notification has no id and expects no answer.
        if result is None or message.get("id") is None:
            continue

        response = {"jsonrpc": "2.0", "id": message["id"]}
        if isinstance(result, dict) and "error" in result and "content" not in result:
            response["error"] = result["error"]
        else:
            response["result"] = result

        sys.stdout.write(json.dumps(response, ensure_ascii=False) + "\n")
        sys.stdout.flush()


if __name__ == "__main__":
    main()
