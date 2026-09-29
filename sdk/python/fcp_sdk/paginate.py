"""Every item of a list, page after page (docs/API_TOKENS.md §6)."""

from __future__ import annotations

from typing import Any, AsyncIterator, Awaitable, Callable, Iterator

from fcp_api.types import UNSET, Response
from .errors import ok


def _next(page: Any) -> Any:
    cursor = getattr(page, "next_cursor", None)
    return None if cursor in (None, UNSET) else cursor


def paginate(operation: Callable[..., Response[Any]], **kwargs: Any) -> Iterator[Any]:
    """Every item of a list operation, following ``next_cursor``::

    for user in paginate(users_list_users.sync_detailed, client=fcp, limit=500):
        ...
    """
    cursor: Any = UNSET
    while True:
        page = ok(operation(cursor=cursor, **kwargs))
        yield from page.data
        next_cursor = _next(page)
        if next_cursor is None:
            return
        cursor = next_cursor


async def apaginate(
    operation: Callable[..., Awaitable[Response[Any]]], **kwargs: Any
) -> AsyncIterator[Any]:
    """``paginate`` for an operation's ``asyncio_detailed``."""
    cursor: Any = UNSET
    while True:
        page = ok(await operation(cursor=cursor, **kwargs))
        for item in page.data:
            yield item
        next_cursor = _next(page)
        if next_cursor is None:
            return
        cursor = next_cursor
