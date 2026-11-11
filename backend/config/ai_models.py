"""Centralised AI model configuration.

Every parameter falls back to its previous hard-coded default so that
existing deployments keep working without any env changes.
"""

from __future__ import annotations

import os

DEEPSEEK_BASE_URL: str = os.getenv("DEEPSEEK_BASE_URL", "https://api.deepseek.com").strip() or "https://api.deepseek.com"
DEEPSEEK_API_KEY: str = os.getenv("DEEPSEEK_API_KEY", "").strip()
DEEPSEEK_TEXT_MODEL: str = os.getenv("DEEPSEEK_TEXT_MODEL", "deepseek-chat").strip() or "deepseek-chat"
# Provider-neutral text settings. They default to the DeepSeek values above so
# existing deployments keep working untouched, but pointing them at any other
# OpenAI-compatible gateway is a pure configuration change.
#
# All three must move together: a base URL from one provider with a model name
# from another is the most likely way to break this.
AI_TEXT_BASE_URL: str = (os.getenv("AI_TEXT_BASE_URL") or DEEPSEEK_BASE_URL).strip()
AI_TEXT_API_KEY: str = (os.getenv("AI_TEXT_API_KEY") or DEEPSEEK_API_KEY).strip()
AI_TEXT_MODEL: str = (os.getenv("AI_TEXT_MODEL") or DEEPSEEK_TEXT_MODEL).strip()
# Reasoning effort, for gateways that understand it (Anthropic-backed ones do).
# Sent as `reasoning_effort` in the request body. An endpoint that does not know
# the field ignores it, so leaving this set is harmless on providers without it.
AI_TEXT_EFFORT: str = (os.getenv("AI_TEXT_EFFORT") or "").strip()

# --- Vision -----------------------------------------------------------------
# Reading a receipt off a photo or a scanned PDF, for the cases where OCR gives
# nothing usable. Defaults follow the text gateway, because the gateway in use
# is Claude and it already accepts images; point these elsewhere to split the
# two. As above, base URL / key / model must be changed together.
AI_VISION_BASE_URL: str = (os.getenv("AI_VISION_BASE_URL") or AI_TEXT_BASE_URL).strip()
AI_VISION_API_KEY: str = (os.getenv("AI_VISION_API_KEY") or AI_TEXT_API_KEY).strip()
AI_VISION_MODEL: str = (os.getenv("AI_VISION_MODEL") or AI_TEXT_MODEL).strip()
AI_VISION_EFFORT: str = (os.getenv("AI_VISION_EFFORT") or AI_TEXT_EFFORT).strip()
# Off by default: it costs money per receipt and every deployment should opt in.
AI_VISION_ENABLED: bool = (os.getenv("AI_VISION_ENABLED") or "").strip().lower() in {"1", "true", "yes", "on"}
# Read photo and PDF receipts with the model FIRST, before Tesseract. Vision is
# far better at a photographed slip than OCR is, and OCR "succeeding" with a
# half-garbled string is worse than not running: it produces a confident wrong
# parse instead of an honest failure. OCR stays as the fallback, so a vision
# outage still lets receipts through.
AI_VISION_FIRST_FOR_MEDIA: bool = (os.getenv("AI_VISION_FIRST_FOR_MEDIA") or "true").strip().lower() in {"1", "true", "yes", "on"}
# A receipt is one page in practice; the cap stops a 200-page PDF from being
# rendered and billed page by page.
AI_VISION_MAX_PAGES: int = max(1, int(os.getenv("AI_VISION_MAX_PAGES", "2")))
AI_VISION_DPI: int = max(72, int(os.getenv("AI_VISION_DPI", "200")))

AI_REQUEST_TIMEOUT_SECONDS: float = float(os.getenv("AI_REQUEST_TIMEOUT_SECONDS", "45"))
AI_MAX_TOOL_CALLS_PER_REQUEST: int = int(os.getenv("AI_MAX_TOOL_CALLS_PER_REQUEST", "6"))
AI_MAX_TOOL_ROUNDS: int = int(os.getenv("AI_MAX_TOOL_ROUNDS", "3"))
AI_RETRY_COUNT: int = int(os.getenv("AI_RETRY_COUNT", "2"))
AI_AGENT_RECEIPT_GRACE_MINUTES: int = int(os.getenv("AI_AGENT_RECEIPT_GRACE_MINUTES", "5"))


def _bool_env(name: str, default: bool) -> bool:
    raw = os.getenv(name)
    if raw is None:
        return default
    return str(raw).strip().lower() in {"1", "true", "yes", "on"}


# ── AI Agent feature flags ───────────────────────────────────────────────────
AI_AGENT_NATIVE_TOOLS: bool = _bool_env("AI_AGENT_NATIVE_TOOLS", True)
AI_AGENT_MUTATION_TOOLS_ENABLED: bool = _bool_env("AI_AGENT_MUTATION_TOOLS_ENABLED", True)
AI_AGENT_AUTOMATION_REQUIRES_CONFIRM: bool = _bool_env("AI_AGENT_AUTOMATION_REQUIRES_CONFIRM", True)

# ── Mapping Agent (analyze_with_ai) ──────────────────────────────────────────
MAPPING_AI_MODEL: str = os.getenv("AUTOMATION_MAPPING_AI_MODEL", DEEPSEEK_TEXT_MODEL)
MAPPING_AI_TEMPERATURE: float = float(os.getenv("AUTOMATION_MAPPING_AI_TEMPERATURE", "0.3"))
MAPPING_AI_MAX_TOKENS: int = int(os.getenv("AUTOMATION_MAPPING_AI_MAX_TOKENS", "200"))

# ── Verification Agent (verify_transaction_with_ai) ──────────────────────────
VERIFY_AI_MODEL: str = os.getenv("AUTOMATION_VERIFY_AI_MODEL", DEEPSEEK_TEXT_MODEL)
VERIFY_AI_TEMPERATURE: float = float(os.getenv("AUTOMATION_VERIFY_AI_TEMPERATURE", "0.1"))
VERIFY_AI_MAX_TOKENS: int = int(os.getenv("AUTOMATION_VERIFY_AI_MAX_TOKENS", "500"))
