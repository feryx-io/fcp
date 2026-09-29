"""A configured client: the token, the tenant, and retries."""

from __future__ import annotations

import asyncio
import time

import httpx

from fcp_api.client import AuthenticatedClient

RETRIED = (429, 503)


def _wait(response: httpx.Response, attempt: int, cap: float) -> float:
    try:
        asked = float(response.headers.get("retry-after", ""))
    except ValueError:
        asked = 2.0**attempt
    return min(max(asked, 0.0), cap)


class RetryTransport(httpx.BaseTransport):
    """Retries a 429 (rate limited) or a 503 (a service the admin needs is
    away), waiting as ``Retry-After`` says."""

    def __init__(self, inner: httpx.BaseTransport, retries: int, cap: float) -> None:
        self.inner, self.retries, self.cap = inner, retries, cap

    def handle_request(self, request: httpx.Request) -> httpx.Response:
        request.read()
        attempt = 0
        while True:
            response = self.inner.handle_request(request)
            if response.status_code not in RETRIED or attempt >= self.retries:
                return response
            wait = _wait(response, attempt, self.cap)
            response.close()
            time.sleep(wait)
            attempt += 1

    def close(self) -> None:
        self.inner.close()


class AsyncRetryTransport(httpx.AsyncBaseTransport):
    """``RetryTransport`` for asyncio."""

    def __init__(self, inner: httpx.AsyncBaseTransport, retries: int, cap: float) -> None:
        self.inner, self.retries, self.cap = inner, retries, cap

    async def handle_async_request(self, request: httpx.Request) -> httpx.Response:
        await request.aread()
        attempt = 0
        while True:
            response = await self.inner.handle_async_request(request)
            if response.status_code not in RETRIED or attempt >= self.retries:
                return response
            wait = _wait(response, attempt, self.cap)
            await response.aclose()
            await asyncio.sleep(wait)
            attempt += 1

    async def aclose(self) -> None:
        await self.inner.aclose()


def _headers(tenant_id: str | None) -> dict[str, str]:
    return {"X-FCP-Tenant-Id": tenant_id} if tenant_id else {}


def client(
    base_url: str,
    token: str,
    *,
    tenant_id: str | None = None,
    retries: int = 3,
    max_retry_wait: float = 60.0,
    timeout: float = 30.0,
) -> AuthenticatedClient:
    """A client for the admin at ``base_url`` (no ``/api/v1``).

    ``token`` is an ``fcp_…`` API token, the static token, or a session
    JWT (docs/API_TOKENS.md). ``tenant_id`` picks the tenant for the static
    token or a platform admin's session; a tenant's own token ignores it.
    """
    return AuthenticatedClient(
        base_url=base_url.rstrip("/"),
        token=token,
        headers=_headers(tenant_id),
        timeout=httpx.Timeout(timeout),
        httpx_args={"transport": RetryTransport(httpx.HTTPTransport(), retries, max_retry_wait)},
    )


def async_client(
    base_url: str,
    token: str,
    *,
    tenant_id: str | None = None,
    retries: int = 3,
    max_retry_wait: float = 60.0,
    timeout: float = 30.0,
) -> AuthenticatedClient:
    """``client`` for asyncio: use it with the operations' ``asyncio_detailed``."""
    return AuthenticatedClient(
        base_url=base_url.rstrip("/"),
        token=token,
        headers=_headers(tenant_id),
        timeout=httpx.Timeout(timeout),
        httpx_args={
            "transport": AsyncRetryTransport(httpx.AsyncHTTPTransport(), retries, max_retry_wait)
        },
    )
