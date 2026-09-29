from http import HTTPStatus
from typing import Any, cast
from urllib.parse import quote

import httpx

from ...client import AuthenticatedClient, Client
from ...types import Response, UNSET
from ... import errors

from ...models.error_response import ErrorResponse
from ...models.seal_request import SealRequest
from ...models.seal_response import SealResponse
from typing import cast



def _get_kwargs(
    *,
    body: SealRequest,

) -> dict[str, Any]:
    headers: dict[str, Any] = {}


    

    

    _kwargs: dict[str, Any] = {
        "method": "post",
        "url": "/api/v1/system/encryption/seal",
    }

    _kwargs["json"] = body.to_dict()

    headers["Content-Type"] = "application/json"

    _kwargs["headers"] = headers
    return _kwargs



def _parse_response(*, client: AuthenticatedClient | Client, response: httpx.Response) -> ErrorResponse | SealResponse:
    if response.status_code == 200:
        response_200 = SealResponse.from_dict(response.json())



        return response_200

    response_default = ErrorResponse.from_dict(response.json())



    return response_default



def _build_response(*, client: AuthenticatedClient | Client, response: httpx.Response) -> Response[ErrorResponse | SealResponse]:
    return Response(
        status_code=HTTPStatus(response.status_code),
        content=response.content,
        headers=response.headers,
        parsed=_parse_response(client=client, response=response),
    )


def sync_detailed(
    *,
    client: AuthenticatedClient | Client,
    body: SealRequest,

) -> Response[ErrorResponse | SealResponse]:
    """ POST /api/v1/system/encryption/seal — Seal one credential with the
    active DEK, for secrets that live in a configuration file rather than a
    column: the E911 validator's bearer token (`docs/E911_BLOCK_2.md` §7).
    The value is never stored or logged.

    Args:
        body (SealRequest):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[ErrorResponse | SealResponse]
     """


    kwargs = _get_kwargs(
        body=body,

    )

    response = client.get_httpx_client().request(
        **kwargs,
    )

    return _build_response(client=client, response=response)

def sync(
    *,
    client: AuthenticatedClient | Client,
    body: SealRequest,

) -> ErrorResponse | SealResponse | None:
    """ POST /api/v1/system/encryption/seal — Seal one credential with the
    active DEK, for secrets that live in a configuration file rather than a
    column: the E911 validator's bearer token (`docs/E911_BLOCK_2.md` §7).
    The value is never stored or logged.

    Args:
        body (SealRequest):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        ErrorResponse | SealResponse
     """


    return sync_detailed(
        client=client,
body=body,

    ).parsed

async def asyncio_detailed(
    *,
    client: AuthenticatedClient | Client,
    body: SealRequest,

) -> Response[ErrorResponse | SealResponse]:
    """ POST /api/v1/system/encryption/seal — Seal one credential with the
    active DEK, for secrets that live in a configuration file rather than a
    column: the E911 validator's bearer token (`docs/E911_BLOCK_2.md` §7).
    The value is never stored or logged.

    Args:
        body (SealRequest):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[ErrorResponse | SealResponse]
     """


    kwargs = _get_kwargs(
        body=body,

    )

    response = await client.get_async_httpx_client().request(
        **kwargs
    )

    return _build_response(client=client, response=response)

async def asyncio(
    *,
    client: AuthenticatedClient | Client,
    body: SealRequest,

) -> ErrorResponse | SealResponse | None:
    """ POST /api/v1/system/encryption/seal — Seal one credential with the
    active DEK, for secrets that live in a configuration file rather than a
    column: the E911 validator's bearer token (`docs/E911_BLOCK_2.md` §7).
    The value is never stored or logged.

    Args:
        body (SealRequest):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        ErrorResponse | SealResponse
     """


    return (await asyncio_detailed(
        client=client,
body=body,

    )).parsed
