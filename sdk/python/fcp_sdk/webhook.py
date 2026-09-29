"""Verifying a webhook delivery (docs/DEVELOPER_SURFACE.md §3, phase 5)."""

from __future__ import annotations

import hashlib
import hmac
import time


def verify_webhook(
    secret: str,
    signature_header: str,
    body: bytes | str,
    *,
    tolerance_secs: int = 300,
    now: int | None = None,
) -> bool:
    """Whether a delivery is genuine.

    ``X-FCP-Signature`` is ``t=<unix seconds>,v1=<hex HMAC-SHA256 of
    "<t>.<body>">`` under the endpoint's secret; ``t`` must be within
    ``tolerance_secs`` of now, so a captured delivery cannot be replayed.
    Pass the body exactly as received, before parsing it.
    """
    parts: dict[str, list[str]] = {}
    for piece in signature_header.split(","):
        key, sep, value = piece.partition("=")
        if sep:
            parts.setdefault(key.strip(), []).append(value.strip())
    try:
        t = int(parts["t"][0])
    except (KeyError, ValueError):
        return False
    if abs((int(time.time()) if now is None else now) - t) > tolerance_secs:
        return False
    raw = body.encode() if isinstance(body, str) else body
    expected = hmac.new(secret.encode(), f"{t}.".encode() + raw, hashlib.sha256).hexdigest()
    # Any v1 signature may match: a secret being rotated signs with both.
    return any(hmac.compare_digest(expected, given) for given in parts.get("v1", []))
