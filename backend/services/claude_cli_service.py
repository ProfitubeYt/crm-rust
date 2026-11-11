"""
Talking to a Claude Code session.

The product's own agent runs single completions through the OpenAI-compatible
gateway: a fresh context every time, ~2000 tokens of tool schema resent on each
call, and a hard ceiling of three tool rounds (`AI_MAX_TOOL_ROUNDS`). That is
fine for classifying a receipt and poor for a conversation, an investigation, or
anything that needs the web.

A CLI session keeps its context between messages, carries its own search and
file tools, and is not bounded by our round limit. It is reached through a small
host service (`deploy/claude-bridge`) because the CLI needs Node and the backend
image has none.

Two rules this module exists to enforce:

  * **The CLI is never load-bearing.** It is an external process on a 3GB box: it
    can hang, hit a subscription limit, or change its output format on upgrade.
    Every call returns a result object rather than raising, and callers are
    expected to fall back to the existing provider when `ok` is false.
  * **Writes are opt-in.** `allow_writes` is what lets a session edit files and
    run commands. Only a flow that ends with a human approving a diff should set
    it; everything else reads.
"""
from __future__ import annotations

import logging
import os
from dataclasses import dataclass
from typing import Any, Dict, Optional

import httpx

logger = logging.getLogger(__name__)

BRIDGE_URL = os.getenv("CLAUDE_BRIDGE_URL", "http://172.18.0.1:20130").rstrip("/")
BRIDGE_TOKEN = os.getenv("CLAUDE_BRIDGE_TOKEN", "").strip()
BRIDGE_TIMEOUT = float(os.getenv("CLAUDE_BRIDGE_CLIENT_TIMEOUT", "300"))

# Opus for the conversation, Sonnet for the mechanical work underneath it —
# the split the customer asked for, to spend the subscription where it shows.
MODEL_CONVERSATION = os.getenv("CLAUDE_CLI_MODEL", "cc/claude-opus-5")
MODEL_WORKER = os.getenv("CLAUDE_CLI_WORKER_MODEL", "cc/claude-sonnet-5")


@dataclass
class ClaudeResult:
    ok: bool
    text: str = ""
    session_id: Optional[str] = None
    cost_usd: Optional[float] = None
    duration_ms: Optional[int] = None
    error: Optional[str] = None
    retryable: bool = False

    def __bool__(self) -> bool:
        return self.ok


def is_configured() -> bool:
    return bool(BRIDGE_TOKEN)


async def ask(
    prompt: str,
    *,
    session_id: Optional[str] = None,
    model: Optional[str] = None,
    system_prompt: Optional[str] = None,
    allow_writes: bool = False,
    timeout: Optional[float] = None,
) -> ClaudeResult:
    """
    Put a question to a Claude Code session and return what it said.

    Pass `session_id` from a previous result to continue that conversation; the
    session keeps its own context, so the caller does not resend history.
    """
    if not is_configured():
        return ClaudeResult(ok=False, error="bridge_not_configured")
    if not (prompt or "").strip():
        return ClaudeResult(ok=False, error="empty_prompt")

    payload: Dict[str, Any] = {
        "prompt": prompt,
        "model": model or MODEL_CONVERSATION,
        "allow_writes": bool(allow_writes),
        "timeout": int(timeout or BRIDGE_TIMEOUT),
    }
    if session_id:
        payload["session_id"] = session_id
    if system_prompt:
        payload["system_prompt"] = system_prompt

    try:
        async with httpx.AsyncClient(timeout=httpx.Timeout(timeout or BRIDGE_TIMEOUT, connect=10.0)) as client:
            response = await client.post(
                f"{BRIDGE_URL}/run",
                json=payload,
                headers={"X-Bridge-Token": BRIDGE_TOKEN},
            )
        data = response.json()
    except Exception as exc:  # noqa: BLE001 - the caller falls back, never crashes
        logger.warning("Claude bridge unreachable: %s", exc)
        return ClaudeResult(ok=False, error=str(exc)[:300], retryable=True)

    if not isinstance(data, dict) or not data.get("ok"):
        error = (data or {}).get("error") if isinstance(data, dict) else "bad_response"
        logger.info("Claude bridge returned an error: %s", error)
        return ClaudeResult(
            ok=False,
            error=str(error)[:300],
            retryable=bool((data or {}).get("retryable")) if isinstance(data, dict) else True,
        )

    return ClaudeResult(
        ok=True,
        text=str(data.get("text") or ""),
        session_id=data.get("session_id"),
        cost_usd=data.get("cost_usd"),
        duration_ms=data.get("duration_ms"),
    )


async def health() -> Dict[str, Any]:
    """Whether the bridge is up and the CLI is installed behind it."""
    if not is_configured():
        return {"ok": False, "error": "bridge_not_configured"}
    try:
        async with httpx.AsyncClient(timeout=httpx.Timeout(10.0, connect=5.0)) as client:
            response = await client.get(f"{BRIDGE_URL}/health")
        return response.json()
    except Exception as exc:  # noqa: BLE001
        return {"ok": False, "error": str(exc)[:200]}
