"""The API's errors (docs/API_TOKENS.md §6)."""

from __future__ import annotations

import json
from typing import Any, TypeVar

from fcp_api.types import Response

T = TypeVar("T")


class FcpError(Exception):
    """A call the API refused: its status, stable code, detail and request id."""

    def __init__(self, status: int, body: Any, request_id: str | None = None) -> None:
        body = body if isinstance(body, dict) else {}
        self.status = status
        self.code: str = body.get("code") or f"http_{status}"
        self.details: str | None = body.get("details")
        self.error: str | None = body.get("error")
        self.request_id = request_id
        self.body = body
        super().__init__(f"{status} {self.code}: {self.details or self.error or 'request failed'}")


def ok(response: Response[T]) -> T:
    """The parsed body of a call, or ``FcpError`` with the API's error::

    users = ok(users_list_users.sync_detailed(client=fcp))
    """
    status = int(response.status_code)
    if status >= 400:
        try:
            body = json.loads(response.content)
        except ValueError:
            body = {"details": response.content.decode(errors="replace")}
        raise FcpError(status, body, response.headers.get("x-request-id"))
    return response.parsed  # type: ignore[return-value]
